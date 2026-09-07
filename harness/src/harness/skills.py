"""Tracked project skills: stage with Kit 3, review changes, preserve local edits."""
from __future__ import annotations

import io
import hashlib
import gzip
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import signal
import stat
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import uuid
from urllib.request import Request, urlopen

from .sessions import SessionError, open_project_path, read_context
from .setup import _read, _root_fd, _identity, _metadata, _diff, SetupManager

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'scripts'))
import kit3
from install_open_source_kit import open_target_directory, KitError

AGENTS = {'claude': ('.claude/skills', 'Claude Code'),
          'codex': ('.agents/skills', 'Codex'), 'cursor': ('.cursor/skills', 'Cursor')}
NAME = re.compile(r'[a-zA-Z0-9][a-zA-Z0-9._-]{0,99}\Z')
MAX_BYTES = 64 * 1024 * 1024
MAX_FILES = 5000


def safe_path(name):
    path = PurePosixPath(name)
    return (bool(name) and name != '.' and len(name) <= 1024 and not path.is_absolute()
            and str(path) == name and '..' not in path.parts and '.git' not in path.parts
            and '\\' not in name and not any(ord(c) < 32 or ord(c) == 127 for c in name))


def extract_source(payload, destination):
    """Do not let the native manager dereference links from an upstream archive."""
    size, count = 0, 0
    archive_limit = MAX_BYTES + MAX_FILES * 2048
    with gzip.GzipFile(fileobj=io.BytesIO(payload)) as compressed:
        expanded = compressed.read(archive_limit + 1)
    if len(expanded) > archive_limit:
        raise SessionError('The expanded skill archive exceeds the size limit.')
    with tarfile.open(fileobj=io.BytesIO(expanded), mode='r:') as archive:
        members = archive.getmembers()
        skill_roots = [member.name.rsplit('/', 1)[0] + '/' for member in members
                       if member.isfile() and member.name.endswith('/SKILL.md')]
        for member in members:
            count += 1
            size += member.size
            name = member.name.rstrip('/')
            if count > MAX_FILES or size > MAX_BYTES or not safe_path(name):
                raise SessionError('The skill archive contains unsupported paths, links, or exceeds the size limit.')
            if member.issym() or member.islnk():
                # Repository policy aliases are irrelevant; links inside a skill would omit required files.
                if not skill_roots or any(name.startswith(root) for root in skill_roots):
                    raise SessionError('Links inside skill directories are not supported.')
                continue
            if not (member.isdir() or member.isfile()):
                raise SessionError('The skill archive contains unsupported file types.')
            target = destination / name
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                with archive.extractfile(member) as source, target.open('xb') as output:
                    shutil.copyfileobj(source, output)
                target.chmod(0o755 if member.mode & 0o111 else 0o644)


def description(text):
    # ponytail: display-only YAML excerpt; the native manager validates skill metadata.
    match = re.search(r'^description:\s*(.*?)(?=\n\S|\Z)', text[:16000], re.M | re.S)
    if not match:
        return ''
    return ' '.join(match.group(1).strip().strip('|>\"\'').split())[:1200]


def validate_selection(values, allowed, label):
    if (not isinstance(values, list) or not values or len(values) > len(allowed)
            or any(not isinstance(value, str) or value not in allowed for value in values)
            or len(values) != len(set(values))):
        raise SessionError(f'Select valid {label}.')


class SkillManager:
    def __init__(self, store):
        self.store = store
        resources = kit3.catalog.load_resources(kit3.catalog.DEFAULT_RESOURCES)
        self.sources = {item['id']: item for item in resources if item.get('skills_source')}
        self.loaded, self.previews = {}, {}
        self.lock = threading.RLock()
        self.process = None

    def catalog(self):
        available = shutil.which('npx') is not None
        return {'sources': [{key: item[key] for key in ('id', 'name', 'description', 'url')}
                            for item in self.sources.values()],
                'agents': [{'id': key, 'name': value[1]} for key, value in AGENTS.items()],
                'available': available,
                'detail': 'Load a source to choose individual skills.' if available else 'Install Node.js with npx to load skills.'}

    def _run(self, command, cwd):
        read_fd, write_fd = os.pipe()
        environment = {**os.environ, 'DISABLE_TELEMETRY': '1', 'NO_COLOR': '1',
                       'npm_config_ignore_scripts': 'true', 'CI': '1',
                       'XDG_STATE_HOME': str(Path(cwd).parent / 'native-state')}
        try:
            guarded = [sys.executable, str(Path(__file__).with_name('process_guard.py')), str(read_fd), '--', *command]
            with tempfile.TemporaryFile() as output:
                process = subprocess.Popen(guarded, cwd=cwd, env=environment, stdin=subprocess.DEVNULL,
                                           stdout=output, stderr=subprocess.DEVNULL,
                                           start_new_session=True, pass_fds=(read_fd,))
                self.process = process
                started = time.monotonic()
                while process.poll() is None:
                    if self.store.stopping.is_set() or time.monotonic() - started > 90:
                        raise SessionError('Loading skills stopped or timed out. Try again.')
                    if os.fstat(output.fileno()).st_size > 2 * 1024 * 1024:
                        raise SessionError('The skill manager produced too much output.')
                    time.sleep(.05)
                if process.returncode:
                    raise SessionError('The skill manager could not load this source. Check Node.js and network access.')
                output.seek(0)
                return output.read(2 * 1024 * 1024).decode('utf-8')
        finally:
            os.close(read_fd)
            os.close(write_fd)
            if self.process:
                self.store._signal(self.process, signal.SIGKILL)
                self.process.wait(timeout=3)
                self.process = None

    @staticmethod
    def _revision(source):
        # Resolve HEAD once, then download that immutable commit (GitHub REST).
        url = f'https://api.github.com/repos/{source}/commits/HEAD'
        request = Request(url, headers={'User-Agent': 'AI-Infrastructure-Harness',
                                       'Accept': 'application/vnd.github.sha'})
        with urlopen(request, timeout=30) as response:
            if response.geturl() != url:
                raise SessionError('Unexpected skill revision redirect.')
            revision = response.read(128).decode('ascii').strip()
        if not re.fullmatch(r'[a-f0-9]{40}', revision):
            raise SessionError('The source did not return a valid commit.')
        return revision

    def _load(self, source_id):
        source = kit3.resolve_source(source_id, list(self.sources.values()))
        if not re.fullmatch(r'[A-Za-z0-9_][\w.-]*/[A-Za-z0-9_][\w.-]*', source, re.ASCII):
            raise SessionError('This catalog source has no supported GitHub skill archive.')
        if not shutil.which('npx'):
            raise SessionError('Install Node.js with npx to load skills.')
        revision = self._revision(source)
        url = f'https://codeload.github.com/{source}/tar.gz/{revision}'
        with urlopen(Request(url, headers={'User-Agent': 'AI-Infrastructure-Harness'}), timeout=30) as response:
            if response.geturl() != url:
                raise SessionError('Unexpected skill archive redirect.')
            payload = response.read(16 * 1024 * 1024 + 1)
        if len(payload) > 16 * 1024 * 1024:
            raise SessionError('The skill archive exceeds the download limit.')
        skills = {}
        with tempfile.TemporaryDirectory(prefix='skills-', dir=self.store.state_dir) as temporary:
            root = Path(temporary)
            source_dir, stage = root / 'source', root / 'stage'
            source_dir.mkdir(); stage.mkdir()
            extract_source(payload, source_dir)
            args = kit3.parser().parse_args(['add', str(source_dir), '--skill', '*', '--agent', 'codex', '--copy', '--yes'])
            self._run(kit3.native_command(args, []), stage)
            args = kit3.parser().parse_args(['list', '--json', '--agent', 'codex'])
            entries = json.loads(self._run(kit3.native_command(args, []), stage))
            if not isinstance(entries, list) or len(entries) > 500:
                raise SessionError('Invalid skill list from the native manager.')
            total, count = 0, 0
            for entry in entries:
                folder = Path(entry['path'])
                name = folder.name
                if (folder.parent != stage / '.agents/skills' or not NAME.fullmatch(name)
                        or entry.get('scope') != 'project' or name in skills):
                    raise SessionError('Invalid staged skill path.')
                files, modes = {}, {}
                for directory, dirs, names in os.walk(folder, followlinks=False):
                    if any((Path(directory) / child).is_symlink() for child in dirs):
                        raise SessionError('Staged skill directories must not be links.')
                    for filename in names:
                        path = Path(directory) / filename
                        relative = path.relative_to(folder).as_posix()
                        if not safe_path(relative):
                            raise SessionError('Invalid staged skill filename.')
                        fd = open_project_path(stage, path.relative_to(stage).as_posix())
                        with os.fdopen(fd, 'rb') as handle:
                            info = os.fstat(handle.fileno())
                            total += info.st_size; count += 1
                            if (not stat.S_ISREG(info.st_mode) or info.st_nlink != 1
                                    or total > MAX_BYTES or count > MAX_FILES):
                                raise SessionError('Staged skills exceed the file or size limit.')
                            files[relative] = handle.read()
                            modes[relative] = 0o755 if info.st_mode & 0o111 else 0o644
                if 'SKILL.md' not in files:
                    raise SessionError('A staged skill is missing SKILL.md.')
                skills[name] = {'name': str(entry.get('name', name))[:200],
                                'description': description(files['SKILL.md'].decode('utf-8', errors='replace')),
                                'files': files, 'modes': modes, 'revision': revision, 'source': source}
        return skills

    def discover(self, source_id, refresh=False):
        if type(refresh) is not bool:
            raise SessionError('Invalid source refresh option.')
        if not isinstance(source_id, str) or source_id not in self.sources:
            raise SessionError('Select a supported skill source.')
        if not self.lock.acquire(blocking=False):
            raise SessionError('Another skill operation is in progress. Try again shortly.')
        try:
            if refresh or source_id not in self.loaded:
                self.loaded[source_id] = self._load(source_id)
            return {'source_id': source_id,
                    'skills': [{'id': key, 'name': value['name'], 'description': value['description']}
                               for key, value in sorted(self.loaded[source_id].items())],
                    'notice': 'Source snapshot loaded. Refresh source to check the latest commit; existing previews keep their reviewed bytes.'}
        except (OSError, ValueError, KeyError, tarfile.TarError) as error:
            if isinstance(error, SessionError):
                raise
            raise SessionError('Could not load this skill source. Check network access and try again.') from error
        finally:
            self.lock.release()

    def _record_name(self, project):
        return 'skills-' + hashlib.sha256(project['id'].encode()).hexdigest() + '.json'

    def _records(self, project):
        fd = _root_fd(self.store.state_dir)
        try:
            value = _read(fd, self._record_name(project))
        finally:
            os.close(fd)
        if value is None:
            return {}
        try:
            data = json.loads(value['body'])
            if data['root'] != str(project['path']) or not isinstance(data['skills'], dict):
                raise ValueError()
            records = data['skills']
            if len(records) > 1500:
                raise ValueError()
            for path, record in records.items():
                agent, name = record['agent'], record['name']
                if (agent not in AGENTS or not NAME.fullmatch(name)
                        or path != f'{AGENTS[agent][0]}/{name}' or not isinstance(record['files'], dict)
                        or not record['files'] or len(record['files']) > MAX_FILES
                        or not isinstance(record['source_id'], str)
                        or not isinstance(record['source'], str) or not isinstance(record['revision'], str)
                        or len(record['revision']) > 128 or len(record['identity']) != 2):
                    raise ValueError()
                for relative, fingerprint in record['files'].items():
                    if (not safe_path(relative) or not re.fullmatch(r'[a-f0-9]{64}', fingerprint['hash'])
                            or type(fingerprint['mode']) is not int or not 0 <= fingerprint['mode'] <= 0o777):
                        raise ValueError()
            return records
        except (ValueError, KeyError, TypeError, AttributeError):
            raise SessionError('Skill tracking metadata is invalid. Restore the runner state before managing skills.')

    def _save_records(self, project, records):
        body = json.dumps({'root': str(project['path']), 'skills': records}, sort_keys=True).encode()
        if len(body) > 4 * 1024 * 1024:
            raise SessionError('Skill tracking exceeds the runner storage limit.')
        fd = _root_fd(self.store.state_dir)
        temporary = '.skills-' + uuid.uuid4().hex
        try:
            descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=fd)
            with os.fdopen(descriptor, 'wb') as handle:
                handle.write(body); handle.flush(); os.fsync(handle.fileno())
            os.replace(temporary, self._record_name(project), src_dir_fd=fd, dst_dir_fd=fd)
        finally:
            try:
                os.unlink(temporary, dir_fd=fd)
            except FileNotFoundError:
                pass
            os.close(fd)

    @staticmethod
    def _tree(root_fd, path):
        files, directories = {}, {}
        total = 0
        def walk(parent, relative):
            nonlocal total
            directories[relative] = _identity(os.fstat(parent))
            if len(directories) > MAX_FILES or relative.count('/') > 64:
                raise SessionError('The skill contains too many directories.')
            for name in sorted(os.listdir(parent)):
                full = relative + '/' + name
                if not safe_path(full):
                    raise SessionError('The skill contains an unsupported filename.')
                info = os.stat(name, dir_fd=parent, follow_symlinks=False)
                if stat.S_ISDIR(info.st_mode):
                    child = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
                    try:
                        walk(child, full)
                    finally:
                        os.close(child)
                else:
                    value = _read(root_fd, full, directories, required=True)
                    total += value['bytes']
                    if len(files) >= MAX_FILES or total > MAX_BYTES:
                        raise SessionError('The skill exceeds the file or size limit.')
                    files[full[len(path)+1:]] = value
        parent = os.dup(root_fd)
        try:
            parts = path.split('/')
            for i, part in enumerate(parts):
                try:
                    child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
                except FileNotFoundError:
                    return files, directories
                os.close(parent); parent = child
                directories['/'.join(parts[:i+1])] = _identity(os.fstat(parent))
            walk(parent, path)
            return files, directories
        except OSError as error:
            raise SessionError('A skill directory is unsafe or unavailable.') from error
        finally:
            os.close(parent)

    @staticmethod
    def _hashes(files):
        return {name: {'hash': value['hash'], 'mode': value['mode']} for name, value in files.items()}

    def installed(self, project_id):
        project = self.store.project(project_id)
        root = Path(project['path'])
        with self.lock:
            records = self._records(project)
        entries = {}
        for agent, (base, _) in AGENTS.items():
            try:
                fd = open_project_path(root, base, directory=True)
            except OSError:
                continue
            try:
                for name in sorted(os.listdir(fd))[:500]:
                    if NAME.fullmatch(name) and read_context(root, f'{base}/{name}/SKILL.md') is not None:
                        path = f'{base}/{name}'
                        entries[path] = {'name': name, 'agent': agent, 'path': path, 'managed': False,
                                         'state': 'untracked', 'can_update': False, 'can_remove': False}
            finally:
                os.close(fd)
        fd = _root_fd(root)
        try:
            identity = list(_identity(os.fstat(fd)))
            for path, record in records.items():
                try:
                    files, _ = self._tree(fd, path)
                    status = 'clean' if identity == record['identity'] and self._hashes(files) == record['files'] else 'modified'
                except SessionError:
                    status = 'unsafe'
                source_id = record['source_id']
                entries[path] = {'name': record['name'], 'agent': record['agent'], 'path': path,
                    'managed': True, 'source_id': source_id, 'source': record['source'],
                    'revision': record['revision'], 'state': status,
                    'can_update': status == 'clean' and source_id in self.sources,
                    'can_remove': status == 'clean'}
        finally:
            os.close(fd)
        return {'project_id': project_id, 'installed': [entries[key] for key in sorted(entries)]}

    @staticmethod
    def _file_status(root, path, payload):
        try:
            fd = open_project_path(root, path)
            with os.fdopen(fd, 'rb') as handle:
                info = os.fstat(handle.fileno())
                if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or info.st_size != len(payload):
                    return 'conflict'
                return 'identical' if handle.read(len(payload) + 1) == payload else 'conflict'
        except FileNotFoundError:
            return 'new'
        except OSError:
            return 'conflict'

    def preview(self, data):
        with self.lock:
            if set(data) != {'project_id', 'source_id', 'skills', 'agents'}:
                raise SessionError('Select a project, source, skills and tools.')
            project = self.store.project(data['project_id'])
            source_id = data['source_id']
            if not isinstance(source_id, str) or source_id not in self.loaded:
                raise SessionError('Load this source before previewing installation.')
            for key, allowed in (('skills', self.loaded[source_id]), ('agents', AGENTS)):
                validate_selection(data[key], allowed, key)
            files = {f'{AGENTS[agent][0]}/{skill}/{relative}': body
                     for agent in data['agents'] for skill in data['skills']
                     for relative, body in self.loaded[source_id][skill]['files'].items()}
            modes = {f'{AGENTS[agent][0]}/{skill}/{relative}': mode
                     for agent in data['agents'] for skill in data['skills']
                     for relative, mode in self.loaded[source_id][skill].get('modes', {}).items()}
            return self._preview(project, source_id, files, modes)

    def create_preview(self, data):
        if not isinstance(data, dict) or set(data) != {'project_id', 'agents', 'name', 'description', 'instructions'}:
            raise SessionError('Enter a project, tools, name, description and instructions.')
        project = self.store.project(data['project_id'])
        validate_selection(data['agents'], AGENTS, 'tools')
        for key in ('name', 'description', 'instructions'):
            value = data[key]
            if not isinstance(value, str) or not value.strip():
                raise SessionError(f'Enter skill {key}.')
            if re.search(r'[\x00-\x08\x0b\x0c\x0e-\x1f\x7f-\x9f\ud800-\udfff\ufffe\uffff]', value):
                raise SessionError(f'Skill {key} contains unsupported characters.')
        name = data['name'].strip()
        if len(name) > 64 or not re.fullmatch(r'[a-z0-9]+(?:-[a-z0-9]+)*', name):
            raise SessionError('Use a name of 1–64 lowercase letters, digits and single hyphens.')
        summary = ' '.join(data['description'].split())
        if len(data['description'].strip()) > 1024 or '<' in summary or '>' in summary:
            raise SessionError('Use a description of at most 1024 characters without angle brackets.')
        instructions = data['instructions'].replace('\r\n', '\n').replace('\r', '\n').strip()
        if len(instructions.encode('utf-8')) > 32000:
            raise SessionError('Skill instructions must be at most 32000 UTF-8 bytes.')
        # JSON strings are YAML-compatible scalars: quotes, colons and numeric names stay literal.
        content = f'---\nname: {json.dumps(name)}\ndescription: {json.dumps(summary, ensure_ascii=False)}\n---\n\n{instructions}\n'
        files = {f'{AGENTS[agent][0]}/{name}/SKILL.md': content.encode('utf-8') for agent in data['agents']}
        return {**self._preview(project, 'local', files), 'name': name, 'content': content}

    def _cache_preview(self, preview_id, preview):
        self.previews = {key: value for key, value in self.previews.items() if time.monotonic() - value['created'] < 900}
        self.previews[preview_id] = preview
        def size(value):
            return sum(map(len, value.get('files', {}).values())) + sum(len(item['body']) for item in value.get('payloads', {}).values())
        while len(self.previews) > 32 or sum(size(value) for value in self.previews.values()) > MAX_BYTES:
            self.previews.pop(next(iter(self.previews)))

    def _preview(self, project, source_id, files, modes=None):
        root = Path(project['path'])
        try:
            fd = open_target_directory(root)
            info = os.fstat(fd)
            os.close(fd)
        except KitError as error:
            raise SessionError('The registered project path is no longer safe.') from error
        # Keep the preview and write bounded, including copies for multiple tools.
        if len(files) > MAX_FILES or sum(map(len, files.values())) > MAX_BYTES:
            raise SessionError('Select fewer skills or tools for one installation.')
        statuses = [{'path': path, 'status': self._file_status(root, path, files[path])} for path in sorted(files)]
        records = {}
        for path, body in files.items():
            base, name, relative = '/'.join(path.split('/')[:2]), path.split('/')[2], '/'.join(path.split('/')[3:])
            agent = next(key for key, value in AGENTS.items() if value[0] == base)
            skill = self.loaded.get(source_id, {}).get(name, {})
            record = records.setdefault(base + '/' + name, {'name': name, 'agent': agent,
                'source_id': source_id, 'source': skill.get('source', source_id),
                'revision': skill.get('revision', ''), 'identity': [info.st_dev, info.st_ino], 'files': {}})
            record['files'][relative] = {'hash': hashlib.sha256(body).hexdigest(), 'mode': (modes or {}).get(path, 0o644)}
        for record in records.values():
            if not record['revision']:
                record['revision'] = 'sha256:' + hashlib.sha256(json.dumps(record['files'], sort_keys=True).encode()).hexdigest()
        can_install = all(item['status'] != 'conflict' for item in statuses)
        preview_id = uuid.uuid4().hex
        with self.lock:
            self._cache_preview(preview_id, {'root': root, 'identity': (info.st_dev, info.st_ino),
                                         'project_id': project['id'], 'files': dict(files), 'modes': dict(modes or {}), 'records': records, 'created': time.monotonic()})
        return {'preview_id': preview_id, 'project_id': project['id'], 'source_id': source_id,
                'files': statuses, 'can_install': can_install,
                'summary': f'{len(files)} files. ' + ('Ready to install.' if can_install else 'Existing files differ or a path is unsafe. Resolve conflicts before installing.')}

    def install(self, preview_id):
        if not isinstance(preview_id, str):
            raise SessionError('Preview the selection before installing.')
        with self.lock:
            preview = self.previews.pop(preview_id, None)
        if preview is None or 'operation' in preview or time.monotonic() - preview['created'] >= 900:
            raise SessionError('The preview expired or was already used. Preview again.')
        installed, unchanged, tracked = [], [], []
        # Use the queue's lock: no session can be queued between checking and copying.
        with self.lock, self.store.lock:
            if self.store.stopping.is_set() or self.store.db.execute("SELECT 1 FROM sessions WHERE status IN ('queued','running') LIMIT 1").fetchone():
                raise SessionError('Wait for active sessions to finish before installing skills.')
            root = Path(self.store.project(preview['project_id'])['path'])
            fd = None
            try:
                fd = open_target_directory(root)
                info = os.fstat(fd)
                if root != preview['root'] or (info.st_dev, info.st_ino) != preview['identity']:
                    raise SessionError('The project changed. Preview again.')
                records = self._records(self.store.project(preview['project_id']))
                files = preview['files']
                statuses = {path: self._file_status(root, path, body) for path, body in files.items()}
                if 'conflict' in statuses.values():
                    raise SessionError('Files changed or conflict with this selection. Preview again.')
                # Publish SKILL.md last so interrupted copies do not expose incomplete new skills.
                for path in sorted(files, key=lambda value: (value.endswith('/SKILL.md'), value)):
                    if statuses[path] == 'identical':
                        unchanged.append(path)
                        continue
                    parent = os.dup(fd)
                    try:
                        parts = path.split('/')
                        for part in parts[:-1]:
                            try:
                                os.mkdir(part, mode=0o755, dir_fd=parent)
                            except FileExistsError:
                                pass
                            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
                            os.close(parent); parent = child
                        temporary = '.harness-skill-' + uuid.uuid4().hex
                        target = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                                         preview['modes'].get(path, 0o644), dir_fd=parent)
                        try:
                            with os.fdopen(target, 'wb') as handle:
                                handle.write(files[path])
                                os.fchmod(handle.fileno(), preview['modes'].get(path, 0o644))
                                handle.flush()
                                os.fsync(handle.fileno())
                            # Atomic publication without replacing a file created since preflight.
                            os.link(temporary, parts[-1], src_dir_fd=parent, dst_dir_fd=parent, follow_symlinks=False)
                        finally:
                            os.unlink(temporary, dir_fd=parent)
                        installed.append(path)
                    finally:
                        os.close(parent)
                for path, record in preview['records'].items():
                    actual, _ = self._tree(fd, path)
                    if self._hashes(actual) == record['files']:
                        records[path] = record
                        tracked.append(path)
                self._save_records(self.store.project(preview['project_id']), records)
            except (OSError, KitError, SessionError) as error:
                if not installed and isinstance(error, SessionError):
                    raise
                raise SessionError(f'Installation stopped; {len(installed)} files added. Existing files were kept. Refresh and preview again.') from error
            finally:
                if fd is not None:
                    os.close(fd)
        return {'ok': True, 'installed': installed, 'unchanged': unchanged,
                'summary': f'{len(installed)} files installed, {len(unchanged)} already identical. {len(tracked)} skills match tracked payloads. Extra or mismatched files are kept. Start a new agent session to use the skills.'}

    def change_preview(self, data):
        if (not isinstance(data, dict) or set(data) != {'project_id', 'agent', 'name', 'operation'}
                or data['operation'] not in ('update', 'remove')
                or not isinstance(data['agent'], str) or data['agent'] not in AGENTS
                or not isinstance(data['name'], str) or not NAME.fullmatch(data['name'])):
            raise SessionError('Select an installed skill, tool and operation.')
        project = self.store.project(data['project_id'])
        path = f"{AGENTS[data['agent']][0]}/{data['name']}"
        if not self.lock.acquire(blocking=False):
            raise SessionError('Another skill operation is in progress. Try again shortly.')
        fd = None
        try:
            record = self._records(project).get(path)
            if record is None:
                raise SessionError('This skill is untracked. Preview installation from its source to track matching files first.')
            fd = _root_fd(Path(project['path']))
            identity = list(_identity(os.fstat(fd)))
            if identity != record['identity']:
                raise SessionError('The project directory changed. Restore it before managing this skill.')
            before, directories = self._tree(fd, path)
            payloads = {}
            next_record = None
            if data['operation'] == 'update':
                source_id = record['source_id']
                if source_id not in self.sources:
                    raise SessionError('This skill has no catalog source. Locally created skills do not have upstream updates.')
                loaded = self._load(source_id)
                skill = loaded.get(data['name'])
                if skill is None:
                    raise SessionError('The skill no longer exists in its source. Nothing was changed; removal is a separate action.')
                if skill.get('source', source_id) != record['source']:
                    raise SessionError('The catalog source changed. Review a new installation instead of updating.')
                self.loaded[source_id] = loaded
                for name, body in skill['files'].items():
                    payloads[name] = {'body': body, 'hash': hashlib.sha256(body).hexdigest(),
                                      'mode': skill.get('modes', {}).get(name, 0o644)}
                next_record = {**record, 'files': self._hashes(payloads), 'revision': skill.get('revision', '')}
                if not next_record['revision']:
                    next_record['revision'] = 'sha256:' + hashlib.sha256(json.dumps(next_record['files'], sort_keys=True).encode()).hexdigest()
            if (len(set(before) | set(payloads)) > MAX_FILES
                    or sum(value['bytes'] for value in before.values()) + sum(len(value['body']) for value in payloads.values()) > MAX_BYTES):
                raise SessionError('The skill exceeds the preview size limit.')
            clean = self._hashes(before) == record['files']
            entries = []
            diff_budget = 256 * 1024
            for name in sorted(set(before) | set(payloads) | set(record['files'])):
                old, new = before.get(name), payloads.get(name)
                local = self._hashes({name: old}).get(name) if old else None
                status = ('conflict' if local != record['files'].get(name) else
                          'delete' if new is None else 'new' if old is None else
                          'identical' if local == self._hashes({name: new})[name] else 'update')
                diff, truncated = _diff(old['body'] if old else b'', new['body'] if new else b'', path + '/' + name)
                if len(diff.encode('utf-8')) > diff_budget:
                    diff = diff.encode('utf-8')[:diff_budget].decode('utf-8', errors='ignore')
                    truncated = True
                diff_budget -= len(diff.encode('utf-8'))
                entries.append({'path': path + '/' + name, 'status': status, 'diff': diff,
                    'diff_truncated': truncated, 'before_bytes': old['bytes'] if old else 0,
                    'after_bytes': len(new['body']) if new else 0,
                    'before_mode': old['mode'] if old else None, 'after_mode': new['mode'] if new else None})
            preview_id = uuid.uuid4().hex
            self._cache_preview(preview_id, {'operation': data['operation'], 'created': time.monotonic(),
                'project_id': project['id'], 'root': Path(project['path']), 'identity': identity, 'path': path,
                'before': {name: _metadata(value) for name, value in before.items()}, 'directories': directories,
                'record': record, 'next_record': next_record, 'payloads': payloads, 'can_apply': clean})
            changes = sum(entry['status'] != 'identical' for entry in entries)
            return {'preview_id': preview_id, 'project_id': project['id'], 'path': path,
                'operation': data['operation'], 'source': record['source'], 'revision': record['revision'],
                'next_revision': next_record['revision'] if next_record else None,
                'files': entries, 'can_apply': clean, 'expires_at': time.time() + 900,
                'summary': f'{changes} file changes. ' + ('Review before applying.' if clean else
                    'Local changes or extra files detected. Restore the tracked version before updating or removing; nothing will be overwritten.')}
        except (OSError, ValueError, KeyError, tarfile.TarError) as error:
            if isinstance(error, SessionError):
                raise
            raise SessionError('Could not prepare the skill change. Check source availability and retry; project files were kept.') from error
        finally:
            if fd is not None:
                os.close(fd)
            self.lock.release()

    def apply_change(self, preview_id):
        if not isinstance(preview_id, str):
            raise SessionError('Preview the skill change first.')
        # ponytail: one skill mutation at a time; per-project locks if this local runner needs parallel installs.
        with self.lock, self.store.lock:
            preview = self.previews.pop(preview_id, None)
            if (not preview or 'operation' not in preview or time.monotonic() - preview['created'] >= 900
                    or not preview['can_apply']):
                raise SessionError('This preview is blocked, expired or already used. Preview again.')
            if self.store.stopping.is_set() or self.store.db.execute("SELECT 1 FROM sessions WHERE status IN ('queued','running') LIMIT 1").fetchone():
                raise SessionError('Wait for active sessions to finish before changing skills.')
            project = self.store.project(preview['project_id'])
            root = Path(project['path'])
            fd = _root_fd(root)
            completed = []
            try:
                records = self._records(project)
                if (root != preview['root'] or list(_identity(os.fstat(fd))) != preview['identity']
                        or records.get(preview['path']) != preview['record']):
                    raise SessionError('The project or tracking record changed. Preview again.')
                actual, _ = self._tree(fd, preview['path'])
                SetupManager._check_directories(fd, preview['directories'])
                if {name: _metadata(value) for name, value in actual.items()} != preview['before']:
                    raise SessionError('Skill files changed after preview. No files were changed; preview again.')
                payloads = preview['payloads']
                for name in sorted(set(preview['before']) | set(payloads), key=lambda value: (value == 'SKILL.md', value)):
                    full = preview['path'] + '/' + name
                    old, new = preview['before'].get(name), payloads.get(name)
                    if old and new and old['hash'] == new['hash'] and old['mode'] == new['mode']:
                        continue
                    if self.store.stopping.is_set():
                        raise SessionError('Skill operation stopped.')
                    self._check_change_root(root, fd, preview)
                    parent = os.dup(fd)
                    temporary = None
                    try:
                        parts = full.split('/')
                        for i, part in enumerate(parts[:-1]):
                            if new:
                                try:
                                    os.mkdir(part, mode=0o755, dir_fd=parent)
                                except FileExistsError:
                                    pass
                            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
                            os.close(parent); parent = child
                            prefix = '/'.join(parts[:i+1])
                            identity = _identity(os.fstat(parent))
                            if prefix in preview['directories'] and preview['directories'][prefix] != identity:
                                raise SessionError('A skill directory changed during application.')
                            preview['directories'][prefix] = identity
                        if _metadata(_read(fd, full)) != old:
                            raise SessionError('A skill file changed during application.')
                        if new:
                            temporary = '.harness-skill-' + uuid.uuid4().hex
                            descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, new['mode'], dir_fd=parent)
                            with os.fdopen(descriptor, 'wb') as handle:
                                handle.write(new['body']); os.fchmod(handle.fileno(), new['mode'])
                                handle.flush(); os.fsync(handle.fileno())
                        self._check_change_root(root, fd, preview)
                        if _metadata(_read(fd, full)) != old:
                            raise SessionError('A skill file changed during application.')
                        if new is None:
                            os.unlink(parts[-1], dir_fd=parent)
                        elif old is None:
                            os.link(temporary, parts[-1], src_dir_fd=parent, dst_dir_fd=parent, follow_symlinks=False)
                        else:
                            os.replace(temporary, parts[-1], src_dir_fd=parent, dst_dir_fd=parent)
                        completed.append(full)
                    finally:
                        if temporary:
                            try:
                                os.unlink(temporary, dir_fd=parent)
                            except FileNotFoundError:
                                pass
                        os.close(parent)
                self._check_change_root(root, fd, preview)
                actual, _ = self._tree(fd, preview['path'])
                if self._hashes(actual) != self._hashes(payloads):
                    raise SessionError('The resulting skill differs from the reviewed files.')
                if preview['next_record'] is None:
                    records.pop(preview['path'])
                else:
                    records[preview['path']] = preview['next_record']
                self._save_records(project, records)
            except (OSError, SessionError) as error:
                if completed:
                    return {'ok': False, 'changed': completed,
                        'summary': 'The operation stopped after some files changed. Tracking was not advanced.',
                        'error': 'Partial result: preserve these files and inspect the skill locally before retrying. Local edits were not intentionally overwritten.'}
                if isinstance(error, SessionError):
                    raise
                raise SessionError('The operation failed before changing files. Preview again.') from error
            finally:
                os.close(fd)
            return {'ok': True, 'changed': completed, 'summary': 'Skill removed.' if preview['operation'] == 'remove' else
                    'Reviewed update applied. Start a new agent session to load it.'}

    @staticmethod
    def _check_change_root(root, fd, preview):
        current = _root_fd(root)
        try:
            if list(_identity(os.fstat(current))) != preview['identity']:
                raise SessionError('The project directory changed during application.')
        finally:
            os.close(current)
        SetupManager._check_directories(fd, preview['directories'])

    def close(self):
        if self.process:
            self.store._signal(self.process, signal.SIGTERM)
