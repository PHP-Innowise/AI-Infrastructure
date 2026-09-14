"""Bounded session uploads, retained outside the user's project."""
import base64
import binascii
import hashlib
import os
import re
import uuid

from .sessions import SessionError
from .setup import _root_fd, _read, _safe_path, MAX_FILE_BYTES

MAX_FILES = 5
MAX_TOTAL_BYTES = 8 * 1024 * 1024
MAX_JSON_BYTES = 12 * 1024 * 1024


def validate(files):
    if not isinstance(files, list) or len(files) > MAX_FILES:
        raise SessionError('Attach up to 5 files per message.')
    decoded, total = [], 0
    for item in files:
        if not isinstance(item, dict) or set(item) != {'name', 'data'}:
            raise SessionError('Each attachment requires a name and base64 data.')
        name, data = item['name'], item['data']
        if (not isinstance(name, str) or not name or len(name.encode('utf-8')) > 200
                or '/' in name or '\\' in name or not _safe_path(name)):
            raise SessionError('Use an attachment filename without paths or control characters (up to 200 UTF-8 bytes).')
        if not isinstance(data, str) or len(data) > 4 * ((MAX_FILE_BYTES + 2) // 3):
            raise SessionError('Each attachment must be at most 4 MiB.')
        try:
            body = base64.b64decode(data, validate=True)
        except (ValueError, binascii.Error) as error:
            raise SessionError('Invalid attachment base64 data.') from error
        total += len(body)
        if len(body) > MAX_FILE_BYTES or total > MAX_TOTAL_BYTES:
            raise SessionError('Attachments must be at most 4 MiB each and 8 MiB per message.')
        decoded.append((name, body))
    return decoded


class Attachments:
    def __init__(self, sessions):
        self.sessions = sessions
        self.root = sessions.state_dir / 'attachments'
        self.root.mkdir(mode=0o700, exist_ok=True)
        os.close(_root_fd(self.root))
        sessions.db.execute('CREATE TABLE IF NOT EXISTS attachments (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, name TEXT NOT NULL, size INTEGER NOT NULL, digest TEXT NOT NULL)')
        sessions.db.commit()

    def save(self, sid, files):
        root = _root_fd(self.root)
        saved, created = [], []
        try:
            for name, body in files:
                identifier = uuid.uuid4().hex
                os.mkdir(identifier, mode=0o700, dir_fd=root)
                folder = os.open(identifier, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=root)
                # Prefix avoids treating an uploaded AGENTS.md/CLAUDE.md as directory policy.
                filename = 'upload-' + name
                created.append((identifier, folder, filename))
                descriptor = os.open(filename, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o400, dir_fd=folder)
                with os.fdopen(descriptor, 'wb') as output:
                    output.write(body)
                saved.append({'id':identifier, 'name':name, 'size':len(body), 'digest':hashlib.sha256(body).hexdigest()})
            self.sessions.db.executemany('INSERT INTO attachments VALUES (?,?,?,?,?)',
                [(item['id'], sid, item['name'], item['size'], item['digest']) for item in saved])
            return [{key: item[key] for key in ('id', 'name', 'size')} for item in saved]
        except Exception:
            for identifier, folder, filename in created:
                try: os.unlink(filename, dir_fd=folder)
                except FileNotFoundError: pass
                os.rmdir(identifier, dir_fd=root)
            raise
        finally:
            for _, folder, _ in created: os.close(folder)
            os.close(root)

    def read(self, sid, identifier):
        if not isinstance(identifier, str) or not re.fullmatch('[a-f0-9]{32}', identifier):
            raise SessionError('Invalid attachment ID.')
        with self.sessions.lock:
            row = self.sessions.db.execute('SELECT * FROM attachments WHERE id=? AND session_id=?', (identifier, sid)).fetchone()
        if row is None:
            raise SessionError('Attachment not found in this session.')
        relative = identifier + '/upload-' + row['name']
        root = _root_fd(self.root)
        try:
            item = _read(root, relative, required=True)
        finally:
            os.close(root)
        body = item['body']
        if len(body) != row['size'] or hashlib.sha256(body).hexdigest() != row['digest']:
            raise SessionError('The stored attachment changed. Attach the file again.')
        return dict(row), body, str(self.root / relative)

    def current(self, sid):
        import json
        with self.sessions.lock:
            rows = self.sessions.db.execute('SELECT data FROM events WHERE session_id=? ORDER BY id DESC', (sid,))
            event = next((data for row in rows if (data := json.loads(row[0])).get('kind') == 'user'), {})
        return [self.read(sid, item['id']) for item in event.get('attachments', [])]

    def directories(self, sid):
        with self.sessions.lock:
            rows = self.sessions.db.execute('SELECT id FROM attachments WHERE session_id=?', (sid,)).fetchall()
        directories = [self.root / row['id'] for row in rows]
        for directory in directories:
            os.close(_root_fd(directory))
        return [str(directory) for directory in directories]
