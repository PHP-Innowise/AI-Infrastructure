"""Form-backed system authoring with CAS previews and recoverable file writes."""
from __future__ import annotations

from contextlib import nullcontext
import base64
import json
import os
from pathlib import Path
import stat
import time
from types import SimpleNamespace
import uuid

from ai_system_lib import (MAX_BYTES, MAX_SOURCE_BYTES, MAX_SERVICES, System, SystemError,
                           digest, encoded, fields, inside, items, manifest,
                           parse_json, read_file, relative, source, text)
from ai_system_execution import workspace_locks
from .sessions import SessionError, open_project_path


def payload(value):
    return (json.dumps(value, ensure_ascii=False, indent=2) + '\n').encode('utf-8')


def snapshot(root, name):
    parent = open_project_path(root, str(Path(name).parent), directory=True)
    try:
        identity = os.fstat(parent)
        try:
            raw = read_file(root, name)
            fd = open_project_path(root, name)
            try:
                info = os.fstat(fd)
            finally:
                os.close(fd)
            return {'sha256': digest(raw), 'body': base64.b64encode(raw).decode(),
                    'mode': stat.S_IMODE(info.st_mode), 'parent': [identity.st_dev, identity.st_ino]}
        except FileNotFoundError:
            return {'sha256': None, 'body': None, 'mode': 0o600,
                    'parent': [identity.st_dev, identity.st_ino]}
    finally:
        os.close(parent)


def replace(root, name, raw, expected, mode):
    """Compare again and replace through an opened parent, never a link."""
    if snapshot(root, name) != expected:
        raise SessionError('System files changed. Reload the editor before saving.')
    parent = open_project_path(root, str(Path(name).parent), directory=True)
    temporary = '.ai-system-edit-' + uuid.uuid4().hex
    try:
        identity = os.fstat(parent)
        if [identity.st_dev, identity.st_ino] != expected['parent']:
            raise SessionError('The selected folder changed. Reload the editor.')
        if raw is None:
            os.unlink(Path(name).name, dir_fd=parent)
        else:
            fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode, dir_fd=parent)
            with os.fdopen(fd, 'wb') as handle:
                os.fchmod(handle.fileno(), mode)
                handle.write(raw); handle.flush(); os.fsync(handle.fileno())
            if snapshot(root, name) != expected:
                raise SessionError('System files changed during saving. Reload the editor.')
            os.replace(temporary, Path(name).name, src_dir_fd=parent, dst_dir_fd=parent)
        os.fsync(parent)
    finally:
        try:
            os.unlink(temporary, dir_fd=parent)
        except FileNotFoundError:
            pass
        os.close(parent)


class Candidate(System):
    def __init__(self, config, roots, files):
        self.proposed = {str(Path(f['root']) / f['name']): base64.b64decode(f['after']) for f in files}
        super().__init__(config, roots)

    def read(self, sid, root, name):
        raw = self.proposed.get(str(root / name))
        if raw is None:
            return super().read(sid, root, name)
        self.snapshot[(sid, name)] = digest(raw)
        return raw


class SystemEditor:
    def __init__(self, sessions):
        self.sessions = sessions
        self.previews = {}
        with sessions.lock:
            sessions.db.execute('CREATE TABLE IF NOT EXISTS system_edits (id TEXT PRIMARY KEY, data TEXT NOT NULL)')
            sessions.db.commit()
            self.recover()

    def recover(self, key=None, locked=False):
        # A completed write can be acknowledged after a crash. A partial write
        # rolls back only bytes owned by this edit; outside changes are preserved.
        if not locked and self.sessions.db.execute(
                "SELECT 1 FROM sessions WHERE status IN ('queued','running') LIMIT 1").fetchone():
            return
        rows = (self.sessions.db.execute('SELECT id,data FROM system_edits WHERE id=?', (key,)).fetchall()
                if key else self.sessions.db.execute('SELECT id,data FROM system_edits').fetchall())
        for row in rows:
            edit = json.loads(row[1])
            try:
                roots = [Path(f['root']) for f in edit['files']]
                scope = SimpleNamespace(root=roots[-1], services={str(i): {'root': r} for i, r in enumerate(roots)})
                with nullcontext() if locked else workspace_locks(scope, list(scope.services)):
                    current = [snapshot(Path(f['root']), f['name']) for f in edit['files']]
                    if not all(c['sha256'] == digest(base64.b64decode(f['after']))
                               and c['parent'] == f['before']['parent'] and c['mode'] == f['before']['mode']
                               for f, c in zip(edit['files'], current)):
                        for f, c in reversed(list(zip(edit['files'], current))):
                            if c == f['before']:
                                continue
                            if (c['parent'] != f['before']['parent'] or
                                    c['mode'] != f['before']['mode'] or
                                    c['sha256'] != digest(base64.b64decode(f['after']))):
                                raise SessionError('Interrupted system save conflicts with external changes.')
                            old = base64.b64decode(f['before']['body']) if f['before']['body'] is not None else None
                            replace(Path(f['root']), f['name'], old, c, f['before']['mode'])
                    self.sessions.db.execute('DELETE FROM system_edits WHERE id=?', (row[0],))
                    self.sessions.db.commit()
            except (OSError, SystemError, SessionError):
                # Keep the recovery receipt. Opening unrelated systems still works.
                continue

    def check_recovery(self, root, config_path):
        self.recover()
        for row in self.sessions.db.execute('SELECT data FROM system_edits').fetchall():
            edit = json.loads(row[0])
            if any(inside(Path(f['root']), root) or inside(root, Path(f['root'])) for f in edit['files']):
                raise SessionError('An interrupted system save needs recovery. Preserve external changes and inspect the selected folders before continuing.')

    def location(self, project_id, config_path):
        root = Path(self.sessions.project(project_id)['path'])
        relative(config_path)
        if not config_path.endswith('.json'):
            raise SessionError('Choose a relative system file ending in .json.')
        self.check_recovery(root, config_path)
        return root, root / config_path

    def folder(self, project_id, folder='.'):
        root = Path(self.sessions.project(project_id)['path'])
        fd = open_project_path(root, folder, directory=True)
        os.close(fd)
        return root / folder if folder != '.' else root

    def registered(self, root):
        for project in sorted(self.sessions.projects.values(), key=lambda p: len(p['path']), reverse=True):
            if inside(root, Path(project['path'])):
                return project['id'], root.relative_to(Path(project['path'])).as_posix()
        raise SessionError('Select and register every service folder before editing this system.')

    def service(self, data):
        fields(data, ('project_id',), ('folder',))
        root = self.folder(data['project_id'], data.get('folder', '.'))
        self.check_recovery(root, 'ai-service.json')
        before = snapshot(root, 'ai-service.json')
        if before['body'] is not None:
            value = parse_json(base64.b64decode(before['body']))
            manifest(value, value['id'])
        else:
            value = {'schema_version': 1, 'id': '', 'description': '', 'owner': '',
                     'relationships_complete': False, 'capabilities': [], 'provides': [], 'consumes': [], 'sources': []}
        return {'project_id': data['project_id'], 'folder': data.get('folder', '.'), 'path': str(root),
                'manifest': 'ai-service.json', 'fingerprint': before['sha256'], 'passport': value}

    def load(self, data):
        fields(data, ('project_id', 'config_path'))
        root, config = self.location(data['project_id'], data['config_path'])
        before = snapshot(root, data['config_path'])
        services, fingerprints = [], [[str(config), before['sha256']]]
        if before['body'] is None:
            name, shared = root.name, []
        else:
            system = System(config, [p['path'] for p in self.sessions.projects.values()])
            name, shared = system.config_data['name'], system.config_data['shared_sources']
            for entry in system.config_data['services']:
                service = system.services[entry['id']]
                self.check_recovery(service['root'], service['manifest_path'])
                if service['access'] != 'available':
                    raise SessionError('A service folder or passport is unavailable. Register its folder before editing.')
                pid, folder = self.registered(service['root'])
                prior = snapshot(service['root'], service['manifest_path'])
                services.append({'project_id': pid, 'folder': folder, 'path': str(service['root']),
                                 'manifest': service['manifest_path'], 'fingerprint': prior['sha256'],
                                 'passport': service['manifest']})
                fingerprints.append([str(service['root'] / service['manifest_path']), prior['sha256']])
        return {**data, 'name': name, 'services': services, 'shared_sources': shared,
                'revision': digest(encoded(fingerprints).encode())}

    def preview(self, data):
        fields(data, ('project_id', 'config_path', 'revision', 'name', 'services', 'shared_sources'), ('discovery_id',))
        discovery = None
        if 'discovery_id' in data:
            from .system_discovery import receipt
            discovery = receipt(self.sessions, data['discovery_id'], data)
        current = self.load({'project_id': data['project_id'], 'config_path': data['config_path']})
        if data['revision'] != current['revision']:
            raise SessionError('System files changed. Reload the editor before saving.')
        text(data['name'], 'system name', 200)
        for entry in items(data['shared_sources']):
            source(entry)
        root, config = self.location(data['project_id'], data['config_path'])
        files, registry, targets, transaction_bytes = [], [], set(), 2
        def add(folder, name, value, maximum):
            nonlocal transaction_bytes
            key = str(folder / name)
            if key in targets:
                raise SessionError('Two records would write the same file.')
            targets.add(key)
            raw = payload(value)
            if len(raw) > maximum:
                raise SessionError('System metadata exceeds its file size limit.')
            prior = snapshot(folder, name)
            record = {'root': str(folder), 'name': name, 'before': prior,
                      'after': base64.b64encode(raw).decode()}
            transaction_bytes += len(encoded(record).encode('utf-8')) + 1
            if transaction_bytes > 8 * MAX_BYTES:
                raise SessionError('System edit is too large; save a smaller set of services.')
            files.append(record)
            return prior
        for service in items(data['services'], MAX_SERVICES):
            fields(service, ('project_id', 'folder', 'manifest', 'fingerprint', 'passport'))
            location = self.folder(service['project_id'], service['folder'])
            self.check_recovery(location, service['manifest'])
            relative(service['manifest'])
            if not service['manifest'].endswith('.json'):
                raise SessionError('Service passports must be JSON files.')
            value = service['passport']
            manifest(value, value['id'])
            prior = add(location, service['manifest'], value, MAX_SOURCE_BYTES)
            if prior['sha256'] != service['fingerprint']:
                raise SessionError('A service passport changed. Select its folder again or reload the editor.')
            if prior['body'] is not None:
                previous = parse_json(base64.b64decode(prior['body']))
                manifest(previous, previous['id'])  # Refuse unrelated file collisions.
            registry.append({'id': value['id'], 'root': Path(os.path.relpath(location, config.parent)).as_posix(),
                             'manifest': service['manifest']})
        # Commit the system registry last, after every passport has been written.
        add(root, data['config_path'], {'schema_version': 1, 'name': data['name'],
            'services': registry, 'shared_sources': data['shared_sources']}, MAX_BYTES)
        system = Candidate(config, [p['path'] for p in self.sessions.projects.values()], files)
        key = uuid.uuid4().hex
        self.previews = {k: v for k, v in self.previews.items() if time.monotonic() - v['created'] < 600}
        while self.previews and (len(self.previews) >= 32 or
                sum(v['size'] for v in self.previews.values()) + transaction_bytes > 16 * MAX_BYTES):
            self.previews.pop(next(iter(self.previews)))
        self.previews[key] = {'created': time.monotonic(), 'project_id': data['project_id'],
                              'config_path': data['config_path'], 'files': files, 'system': system,
                              'size': transaction_bytes, 'discovery_request': discovery}
        return {'preview_id': key, 'catalog': system.catalog(), 'file_count': len(files)}

    def apply(self, data):
        fields(data, ('preview_id',))
        preview = self.previews.pop(data['preview_id'], None) if isinstance(data['preview_id'], str) else None
        if preview is None or time.monotonic() - preview['created'] >= 600:
            raise SessionError('The prepared save expired or was already used. Save again.')
        if self.sessions.stopping.is_set() or self.sessions.db.execute(
                "SELECT 1 FROM sessions WHERE status IN ('queued','running') LIMIT 1").fetchone():
            raise SessionError('Wait for active sessions to finish before saving system metadata.')
        root, _ = self.location(preview['project_id'], preview['config_path'])
        with workspace_locks(preview['system'], list(preview['system'].services)):
            if preview.get('discovery_request'):
                from .system_discovery import check_fresh
                check_fresh(preview['discovery_request'])
            for f in preview['files']:
                if snapshot(Path(f['root']), f['name']) != f['before']:
                    raise SessionError('System files changed. Reload the editor before saving.')
            key = uuid.uuid4().hex
            self.sessions.db.execute('INSERT INTO system_edits VALUES (?,?)', (key, encoded({'files': preview['files']})))
            self.sessions.db.commit()
            try:
                for f in preview['files']:
                    if digest(base64.b64decode(f['after'])) == f['before']['sha256']:
                        continue
                    replace(Path(f['root']), f['name'], base64.b64decode(f['after']), f['before'], f['before']['mode'])
                self.sessions.db.execute('DELETE FROM system_edits WHERE id=?', (key,))
                self.sessions.db.commit()
            except (OSError, SystemError, SessionError):
                self.recover(key, locked=True)
                pending = self.sessions.db.execute('SELECT 1 FROM system_edits WHERE id=?', (key,)).fetchone()
                raise SessionError('Saving stopped. ' + ('Recovery conflicts with external changes; inspect the service folders.'
                                   if pending else 'Prior metadata was restored. Reload the editor before saving again.')) from None
        loaded = self.load({'project_id': preview['project_id'], 'config_path': preview['config_path']})
        system = System(root / preview['config_path'], [p['path'] for p in self.sessions.projects.values()])
        return {'editor': loaded, 'catalog': system.catalog(), 'diagram': system.diagram(), 'root': str(system.root)}
