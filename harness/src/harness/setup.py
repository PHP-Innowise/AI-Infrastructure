"""Review and install the existing accelerator payload without running project code."""
from __future__ import annotations

from contextlib import contextmanager
import difflib
import hashlib
import json
import os
from pathlib import Path
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time
import uuid

from .sessions import SessionError, git_details

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'scripts'))
import install_accelerator as installer
from install_open_source_kit import KitError, open_target_directory

MAX_FILES = 5000
MAX_BYTES = 64 * 1024 * 1024
MAX_FILE_BYTES = 4 * 1024 * 1024
DIFF_BYTES = 12 * 1024
PREVIEW_TTL = 900
MAX_PREVIEWS = 4
TOOL_NAMES = {'claude': 'Claude Code', 'codex': 'Codex', 'cursor': 'Cursor'}


def _safe_path(value):
    return (installer.is_safe_relative_path(value) and len(value) <= 1024
            and '.git' not in Path(value).parts
            and not any(ord(char) < 32 or ord(char) == 127 for char in value))


def _identity(metadata):
    return (metadata.st_dev, metadata.st_ino)


def _fingerprint(metadata, body):
    return {'identity': _identity(metadata), 'mode': stat.S_IMODE(metadata.st_mode),
            'bytes': len(body), 'hash': hashlib.sha256(body).hexdigest()}


def _root_fd(root):
    try:
        return open_target_directory(root)
    except (OSError, KitError) as error:
        raise SessionError('The selected directory is unavailable or contains a link.') from error


def _read(root_fd, name, directories=None, required=False):
    """Read from retained directory descriptors; every ancestor must be a directory."""
    if not _safe_path(name):
        raise SessionError('The accelerator inventory contains an unsupported path.')
    parent = os.dup(root_fd)
    parts = name.split('/')
    try:
        for index, part in enumerate(parts[:-1]):
            try:
                child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
            except FileNotFoundError:
                if required:
                    raise SessionError('A selected accelerator source file is missing: ' + name)
                return None
            os.close(parent)
            parent = child
            if directories is not None:
                directories['/'.join(parts[:index + 1])] = _identity(os.fstat(parent))
        try:
            descriptor = os.open(parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
        except FileNotFoundError:
            if required:
                raise SessionError('A selected accelerator source file is missing: ' + name)
            return None
        try:
            before = os.fstat(descriptor)
            if not stat.S_ISREG(before.st_mode) or before.st_nlink != 1:
                raise SessionError('Only regular files without hard links are supported: ' + name)
            if before.st_size > MAX_FILE_BYTES:
                raise SessionError('A selected file exceeds the 4 MiB limit: ' + name)
            body = bytearray()
            while len(body) <= MAX_FILE_BYTES:
                chunk = os.read(descriptor, min(65536, MAX_FILE_BYTES + 1 - len(body)))
                if not chunk:
                    break
                body.extend(chunk)
            after = os.fstat(descriptor)
            if (len(body) > MAX_FILE_BYTES or _identity(before) != _identity(after)
                    or before.st_size != after.st_size or before.st_mtime_ns != after.st_mtime_ns
                    or before.st_ctime_ns != after.st_ctime_ns or after.st_nlink != 1):
                raise SessionError('A selected file changed while being read. Preview again.')
            value = bytes(body)
            return {**_fingerprint(after, value), 'body': value}
        finally:
            os.close(descriptor)
    except OSError as error:
        raise SessionError('A selected path is unsafe or obstructed: ' + name) from error
    finally:
        os.close(parent)


def _metadata(value):
    return {key: item for key, item in value.items() if key != 'body'} if value is not None else None


def _write_stage(root, name, value):
    path = root / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(value['body'])
    path.chmod(value['mode'] & 0o777)


def _diff(before, after, name):
    try:
        old, new = before.decode('utf-8'), after.decode('utf-8')
    except UnicodeError:
        return 'Binary content differs.' if before != after else '', False
    if '\x00' in old or '\x00' in new:
        return 'Binary content differs.' if before != after else '', False
    # Avoid a quadratic diff over large generated or minified files. The full
    # payload is still immutable and its size is shown for every selected path.
    if len(before) + len(after) > 128 * 1024:
        return 'Diff omitted for a large file; full reviewed payload size is shown.', True
    lines, total, truncated = [], 0, False
    for line in difflib.unified_diff(old.splitlines(True), new.splitlines(True),
                                     fromfile='before/' + name, tofile='after/' + name):
        encoded = line.encode('utf-8')
        if total + len(encoded) > DIFF_BYTES:
            lines.append(encoded[:max(0, DIFF_BYTES - total)].decode('utf-8', errors='ignore'))
            truncated = True
            break
        total += len(encoded)
        lines.append(line)
    return ''.join(lines), truncated


class SetupManager:
    def __init__(self, sessions, source_root=None):
        self.sessions = sessions
        self.source_root = Path(os.path.abspath(str(source_root or ROOT)))
        self.lock = threading.Lock()
        self.closed = threading.Event()
        self.process = None
        self.previews = {}

    @contextmanager
    def _operation(self):
        if self.closed.is_set() or not self.lock.acquire(blocking=False):
            raise SessionError('Accelerator setup is busy or stopping. Try again when it finishes.')
        try:
            if self.closed.is_set():
                raise SessionError('Accelerator setup is stopping.')
            yield
        finally:
            self.lock.release()

    def _project(self, project_id):
        project = self.sessions.project(project_id)
        root = Path(project['path'])
        descriptor = _root_fd(root)
        return project, root, descriptor

    def _check_overlap(self, root):
        for boundary in (self.source_root, self.sessions.state_dir):
            if root == boundary or root in boundary.parents or boundary in root.parents:
                raise SessionError('Install into a separate project outside the accelerator source and Harness state directories.')

    def _installer(self, script, source, target, edition, tools):
        if self.closed.is_set() or self.sessions.stopping.is_set():
            raise SessionError('Accelerator setup is stopping. No project files were changed.')
        command = [sys.executable, str(script), '--source-root', str(source), '--edition', edition,
                   '--target', str(target), '--merge-existing']
        for tool in tools:
            command.extend(['--tool', tool])
        read_fd, write_fd = os.pipe()
        process = None
        try:
            environment = {**os.environ, 'PYTHONDONTWRITEBYTECODE': '1'}
            guarded = [sys.executable, str(Path(__file__).with_name('process_guard.py')), str(read_fd), '--', *command]
            process = subprocess.Popen(guarded, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                stderr=subprocess.PIPE, env=environment, start_new_session=True,
                pass_fds=(read_fd, self.sessions.runner_lock))
            self.process = process
            if self.closed.is_set() or self.sessions.stopping.is_set():
                self.sessions._signal(process, signal.SIGTERM)
            os.close(read_fd)
            read_fd = None
            try:
                output, errors = process.communicate(timeout=30)
            except subprocess.TimeoutExpired as error:
                raise SessionError('The staged accelerator preview timed out. No project files were changed.') from error
            if self.closed.is_set() or len(output) + len(errors) > 8 * 1024 * 1024:
                raise SessionError('The staged accelerator preview stopped or exceeded its output limit.')
            return process.returncode, output.decode('utf-8', errors='replace'), errors.decode('utf-8', errors='replace')
        finally:
            if read_fd is not None:
                os.close(read_fd)
            os.close(write_fd)
            if process is not None:
                self.sessions._signal(process, signal.SIGKILL)
                process.wait(timeout=3)
                process.stdout.close()
                process.stderr.close()
            self.process = None

    def preview(self, data):
        if not isinstance(data, dict) or set(data) != {'project_id', 'edition', 'tools'}:
            raise SessionError('Select a project, accelerator edition and tools.')
        edition, tools = data['edition'], data['tools']
        if not isinstance(edition, str) or edition not in installer.EDITIONS:
            raise SessionError('Select a supported accelerator edition.')
        if (not isinstance(tools, list) or not 1 <= len(tools) <= 3
                or any(not isinstance(tool, str) or tool not in TOOL_NAMES for tool in tools)
                or len(set(tools)) != len(tools)):
            raise SessionError('Select distinct supported tools.')
        with self._operation():
            project, root, target_fd = self._project(data['project_id'])
            source_fd, code_fd = None, None
            try:
                self._check_overlap(root)
                source_fd, code_fd = _root_fd(self.source_root), _root_fd(ROOT)
                source_dirs, target_dirs = {}, {}
                inventory_name = 'install/inventories/' + installer.inventory_path(edition).name
                inventory = _read(source_fd, inventory_name, source_dirs, required=True)
                code = _read(code_fd, 'scripts/install_accelerator.py', required=True)
                sources = {inventory_name: inventory}
                before = {}
                with tempfile.TemporaryDirectory(prefix='setup-', dir=self.sessions.state_dir) as temporary:
                    private = Path(temporary)
                    staged_source, staged_target = private / 'source', private / 'target'
                    staged_source.mkdir(); staged_target.mkdir()
                    _write_stage(staged_source, inventory_name, inventory)
                    try:
                        catalog = installer.load_inventory(edition, staged_source)
                    except (installer.InventoryError, TypeError, ValueError) as error:
                        raise SessionError('The accelerator inventory is invalid.') from error
                    selected = installer.selected_files(catalog, tools)
                    if len(selected) > MAX_FILES:
                        raise SessionError('The selected accelerator exceeds the 5000-file limit.')
                    total = len(inventory['body']) + len(code['body'])
                    prefix = installer.edition_path(edition).as_posix() + '/'
                    for _, name in selected:
                        source_name = prefix + catalog['source_overrides'].get(name, name)
                        source = _read(source_fd, source_name, source_dirs, required=True)
                        sources[source_name] = source
                        total += source['bytes']
                        _write_stage(staged_source, source_name, source)
                        before[name] = _read(target_fd, name, target_dirs)
                        if before[name] is not None:
                            total += before[name]['bytes']
                            _write_stage(staged_target, name, before[name])
                        if total > MAX_BYTES:
                            raise SessionError('The selected source and project files exceed the 64 MiB preview limit.')
                    readme = before.get('README.md')
                    if readme and readme['hash'] != sources[prefix + catalog['source_overrides'].get('README.md', 'README.md')]['hash']:
                        before['ACCELERATOR.md'] = _read(target_fd, 'ACCELERATOR.md', target_dirs)
                        if before['ACCELERATOR.md'] is not None:
                            total += before['ACCELERATOR.md']['bytes']
                            _write_stage(staged_target, 'ACCELERATOR.md', before['ACCELERATOR.md'])
                    if total > MAX_BYTES:
                        raise SessionError('The selected source and project files exceed the preview limit.')
                    script = private / 'installer.py'
                    script.write_bytes(code['body'])
                    result, output, errors = self._installer(script, staged_source, staged_target, edition, tools)
                    collisions = {}
                    if result == 2:
                        for line in errors.splitlines():
                            fields = line.split('\t')
                            if len(fields) == 4 and fields[0] == 'COLLISION' and fields[2] in before:
                                collisions[fields[2]] = fields[3].split(':', 1)[0]
                        if not collisions:
                            raise SessionError('The existing installer refused this selection.')
                        # Obtain the installer's exact resolutions for the other
                        # files. Conflicting paths remain blocked in the receipt.
                        for name in collisions:
                            source = sources[prefix + catalog['source_overrides'].get(name, name)]
                            destination = 'ACCELERATOR.md' if name == 'README.md' and readme else name
                            _write_stage(staged_target, destination, source)
                        result, output, _ = self._installer(script, staged_source, staged_target, edition, tools)
                    if result != 0:
                        raise SessionError('The existing installer could not prepare this selection. No project files were changed.')
                    actions = {}
                    for line in output.splitlines():
                        fields = line.split('\t')
                        if len(fields) >= 3 and fields[0] in ('COPY', 'MERGE', 'COPY_AS', 'UNCHANGED'):
                            actions[fields[2]] = (fields[0].lower().replace('_', '-'), fields[3] if len(fields) > 3 else fields[2])
                    if set(actions) != {name for _, name in selected}:
                        raise SessionError('The staged installer returned an incomplete file plan.')
                    files, payloads = [], {}
                    stage_fd = _root_fd(staged_target)
                    try:
                        for _, name in selected:
                            action, destination = actions[name]
                            if name == 'README.md' and readme and 'ACCELERATOR.md' in before:
                                destination = 'ACCELERATOR.md'
                            value = _read(stage_fd, destination, required=True)
                            previous = before.get(destination)
                            change, truncated = _diff(previous['body'] if previous else b'', value['body'], destination)
                            entry = {'path': destination, 'source_path': name,
                                     'action': 'collision' if name in collisions else action,
                                     'before_bytes': previous['bytes'] if previous else 0, 'after_bytes': value['bytes'],
                                     'diff': change, 'diff_truncated': truncated}
                            if name in collisions:
                                entry['reason'] = collisions[name]
                            files.append(entry)
                            payloads[destination] = value
                    finally:
                        os.close(stage_fd)
                    if sum(value['bytes'] for value in payloads.values()) > MAX_BYTES:
                        raise SessionError('The merged accelerator payload exceeds the 64 MiB preview limit.')
                self.previews = {key: item for key, item in self.previews.items() if time.monotonic() - item['created'] < PREVIEW_TTL}
                while len(self.previews) >= MAX_PREVIEWS:
                    self.previews.pop(next(iter(self.previews)))
                preview_id = uuid.uuid4().hex
                self.previews[preview_id] = {'created': time.monotonic(), 'project_id': project['id'], 'root': root,
                    'identity': _identity(os.fstat(target_fd)), 'source_identity': _identity(os.fstat(source_fd)),
                    'source_dirs': source_dirs, 'target_dirs': target_dirs, 'edition': edition, 'tools': list(tools),
                    'release': catalog['release'], 'before': {key: _metadata(value) for key, value in before.items()},
                    'sources': {key: _metadata(value) for key, value in sources.items()}, 'code': _metadata(code),
                    'payloads': payloads, 'files': files, 'can_install': not collisions}
                changed = sum(item['action'] != 'unchanged' for item in files)
                return {'preview_id': preview_id, 'project_id': project['id'], 'edition': edition, 'tools': list(tools),
                    'expires_at': time.time() + PREVIEW_TTL, 'files': [dict(item) for item in files], 'file_count': len(files), 'changed_count': changed,
                    'collisions': [{'path': item['path'], 'reason': item['reason']} for item in files if item['action'] == 'collision'],
                    'can_install': not collisions, 'summary': f'{len(files)} files; {changed} changes; {len(collisions)} collisions. '
                    + ('Review the changes before installing.' if not collisions else 'Resolve collisions and preview again.')}
            finally:
                for descriptor in (target_fd, source_fd, code_fd):
                    if descriptor is not None:
                        os.close(descriptor)

    @staticmethod
    def _check_directories(root_fd, expected):
        for name, identity in expected.items():
            descriptor = os.dup(root_fd)
            try:
                for part in name.split('/'):
                    child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=descriptor)
                    os.close(descriptor)
                    descriptor = child
                if _identity(os.fstat(descriptor)) != identity:
                    raise SessionError('A selected directory changed. Preview again.')
            except OSError as error:
                raise SessionError('A selected directory changed or became unsafe. Preview again.') from error
            finally:
                os.close(descriptor)

    def _receipt(self, project_id, value=None):
        descriptor = _root_fd(self.sessions.state_dir)
        name = 'setup-' + hashlib.sha256(project_id.encode('utf-8')).hexdigest() + '.json'
        try:
            if value is None:
                snapshot = _read(descriptor, name)
                return json.loads(snapshot['body']) if snapshot else None
            temporary = '.setup-' + uuid.uuid4().hex
            output = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=descriptor)
            try:
                with os.fdopen(output, 'wb') as handle:
                    handle.write(json.dumps(value, ensure_ascii=False).encode('utf-8'))
                    handle.flush(); os.fsync(handle.fileno())
                _read(descriptor, name)  # Refuse an unexpected link/nonregular prior receipt.
                os.replace(temporary, name, src_dir_fd=descriptor, dst_dir_fd=descriptor)
            finally:
                try:
                    os.unlink(temporary, dir_fd=descriptor)
                except FileNotFoundError:
                    pass
        finally:
            os.close(descriptor)

    def install(self, data):
        if not isinstance(data, dict) or set(data) != {'preview_id'} or not isinstance(data['preview_id'], str):
            raise SessionError('Install using the reviewed preview ID.')
        with self._operation(), self.sessions.lock:
            preview = self.previews.pop(data['preview_id'], None)
            if preview is None or time.monotonic() - preview['created'] >= PREVIEW_TTL:
                raise SessionError('The preview expired or was already used. Preview again.')
            if not preview['can_install']:
                raise SessionError('Resolve the preview collisions before installing.')
            if self.sessions.stopping.is_set() or self.sessions.db.execute(
                    "SELECT 1 FROM sessions WHERE status IN ('queued','running') LIMIT 1").fetchone():
                raise SessionError('Wait for active sessions to finish before installing the accelerator.')
            _, root, target_fd = self._project(preview['project_id'])
            source_fd, code_fd = None, None
            installed, merged, unchanged = [], [], []
            writing = False
            try:
                self._check_overlap(root)
                source_fd, code_fd = _root_fd(self.source_root), _root_fd(ROOT)
                if (root != preview['root'] or _identity(os.fstat(target_fd)) != preview['identity']
                        or _identity(os.fstat(source_fd)) != preview['source_identity']):
                    raise SessionError('The project or source directory changed. Preview again.')
                self._check_directories(source_fd, preview['source_dirs'])
                self._check_directories(target_fd, preview['target_dirs'])
                for name, expected in preview['sources'].items():
                    if _metadata(_read(source_fd, name, required=True)) != expected:
                        raise SessionError('Accelerator sources changed. Preview again.')
                if _metadata(_read(code_fd, 'scripts/install_accelerator.py', required=True)) != preview['code']:
                    raise SessionError('The accelerator installer changed. Preview again.')
                for name, expected in preview['before'].items():
                    if _metadata(_read(target_fd, name)) != expected:
                        raise SessionError('Project files changed. Preview again.')
                for entry in preview['files']:
                    if self.closed.is_set() or self.sessions.stopping.is_set():
                        raise SessionError('Accelerator setup stopped during installation.')
                    name, action = entry['path'], entry['action']
                    if action == 'unchanged':
                        unchanged.append(name)
                        continue
                    writing = True
                    self._check_directories(target_fd, preview['target_dirs'])
                    current_root = _root_fd(root)
                    try:
                        if _identity(os.fstat(current_root)) != preview['identity']:
                            raise SessionError('The project directory changed during installation.')
                    finally:
                        os.close(current_root)
                    parent = os.dup(target_fd)
                    temporary = None
                    try:
                        parts = name.split('/')
                        for index, part in enumerate(parts[:-1]):
                            try:
                                os.mkdir(part, mode=0o755, dir_fd=parent)
                            except FileExistsError:
                                pass
                            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
                            os.close(parent); parent = child
                            prefix = '/'.join(parts[:index + 1])
                            identity = _identity(os.fstat(parent))
                            if prefix in preview['target_dirs'] and preview['target_dirs'][prefix] != identity:
                                raise SessionError('An installation directory changed.')
                            preview['target_dirs'][prefix] = identity
                        if _metadata(_read(target_fd, name)) != preview['before'].get(name):
                            raise SessionError('An installation file changed after preflight.')
                        value = preview['payloads'][name]
                        temporary = '.harness-setup-' + uuid.uuid4().hex
                        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                                             value['mode'] & 0o777, dir_fd=parent)
                        with os.fdopen(descriptor, 'wb') as handle:
                            handle.write(value['body'])
                            os.fchmod(handle.fileno(), value['mode'] & 0o777)
                            handle.flush(); os.fsync(handle.fileno())
                        self._check_directories(target_fd, preview['target_dirs'])
                        if action == 'merge':
                            if name not in {'AGENTS.md', '.gitignore', '.gitattributes'}:
                                raise SessionError('The installer proposed an unsupported merge.')
                            if _metadata(_read(target_fd, name)) != preview['before'].get(name):
                                raise SessionError('A supported merge target changed after preflight.')
                            os.replace(temporary, parts[-1], src_dir_fd=parent, dst_dir_fd=parent)
                            merged.append(name)
                        else:
                            os.link(temporary, parts[-1], src_dir_fd=parent, dst_dir_fd=parent, follow_symlinks=False)
                            installed.append(name)
                    finally:
                        if temporary is not None:
                            try:
                                os.unlink(temporary, dir_fd=parent)
                            except FileNotFoundError:
                                pass
                        os.close(parent)
                fingerprints = {}
                self._check_directories(target_fd, preview['target_dirs'])
                current_root = _root_fd(root)
                try:
                    if _identity(os.fstat(current_root)) != preview['identity']:
                        raise SessionError('The project directory changed during installation.')
                finally:
                    os.close(current_root)
                for name, value in preview['payloads'].items():
                    actual = _read(target_fd, name, required=True)
                    if actual['hash'] != value['hash'] or actual['mode'] & 0o777 != value['mode'] & 0o777:
                        raise SessionError('Installed files did not match the reviewed payload.')
                    fingerprints[name] = {'hash': actual['hash'], 'mode': actual['mode']}
                if self.closed.is_set() or self.sessions.stopping.is_set():
                    raise SessionError('Accelerator setup stopped before application verification completed.')
                self._receipt(preview['project_id'], {'edition': preview['edition'], 'release': preview['release'],
                    'tools': preview['tools'], 'root': str(root), 'identity': list(preview['identity']), 'files': fingerprints})
            except (OSError, SessionError) as error:
                if writing:
                    return {'ok': False, 'installed': installed, 'merged': merged, 'unchanged': unchanged,
                            'error': 'Installation stopped and may be partial. Refresh the project and preview again.',
                            'summary': f'{len(installed)} files added and {len(merged)} merged before installation stopped.'}
                if isinstance(error, SessionError):
                    raise
                raise SessionError('Installation preflight failed. No project files were written; preview again.') from error
            finally:
                for descriptor in (target_fd, source_fd, code_fd):
                    if descriptor is not None:
                        os.close(descriptor)
            return {'ok': True, 'installed': installed, 'merged': merged, 'unchanged': unchanged,
                    'summary': f'{len(installed)} files installed, {len(merged)} merged, {len(unchanged)} unchanged. Reviewed payload hashes verified.'}

    def status(self, project_id):
        project = self.sessions.project(project_id)
        result = {'project_id': project_id, 'path': project['path'], 'available': False,
            'editions': [{'id': name, 'name': name, 'release': None} for name in installer.EDITIONS],
            'tools': [{'id': key, 'name': label, 'installed': False} for key, label in TOOL_NAMES.items()],
            'installed_edition': None, 'readiness': {'policy': False, 'memory_runtime': False, 'brain_runtime': False},
            'payload_verified': False, 'git': {'is_git': False},
            'providers': [{key: value for key, value in provider.items() if key in ('id', 'name', 'available', 'detail')}
                          for provider in self.sessions.providers.values()],
            'diagnostics': ['Provider availability means the CLI was found; authentication is not checked.']}
        source_fd = None
        try:
            source_fd = _root_fd(self.source_root)
            for edition in result['editions']:
                snapshot = _read(source_fd, 'install/inventories/' + installer.inventory_path(edition['id']).name)
                if snapshot:
                    value = json.loads(snapshot['body']).get('release')
                    if isinstance(value, str) and len(value) <= 100:
                        edition['release'] = value
        except (SessionError, ValueError, TypeError, AttributeError):
            result['diagnostics'].append('Some accelerator release metadata is unavailable.')
        finally:
            if source_fd is not None:
                os.close(source_fd)
        descriptor = None
        try:
            descriptor = _root_fd(Path(project['path']))
            result['available'] = True
            def present(name):
                value = _read(descriptor, name)
                return bool(value and value['bytes'])
            result['readiness']['policy'] = present('AGENTS.md')
            result['readiness']['memory_runtime'] = all(present(name) for name in (
                'memory-bank/scripts/context.py', 'memory-bank/scripts/brain_runtime.py', 'memory-bank/scripts/context_retrieval.py'))
            config = _read(descriptor, 'project-brain/config/runtime.json')
            configuration = json.loads(config['body']) if config else None
            result['readiness']['brain_runtime'] = (result['readiness']['memory_runtime'] and isinstance(configuration, dict)
                and configuration.get('mode', 'governed') in ('governed', 'legacy')
                and present('project-brain/PROTOCOL.md'))
            for item in result['tools']:
                names = {'claude': ('.claude/settings.json',), 'codex': ('.codex/config.toml', '.agents/skills/SKILL FLOW.md'),
                         'cursor': ('.cursor/hooks.json', '.cursor/rules/accelerator.mdc')}[item['id']]
                item['installed'] = any(present(name) for name in names)
            receipt = self._receipt(project_id)
            if isinstance(receipt, dict) and receipt.get('edition') in installer.EDITIONS:
                if receipt.get('root') == project['path'] and receipt.get('identity') == list(_identity(os.fstat(descriptor))):
                    result['installed_edition'] = receipt['edition']
                    fingerprints = receipt.get('files')
                    if isinstance(fingerprints, dict) and len(fingerprints) <= MAX_FILES:
                        result['payload_verified'] = bool(fingerprints)
                        present_tools = set()
                        for name, expected in fingerprints.items():
                            actual = _read(descriptor, name)
                            if actual and actual['bytes']:
                                present_tools.add(installer.component_for(name))
                            if (not isinstance(expected, dict) or not actual or actual['hash'] != expected.get('hash')
                                    or actual['mode'] != expected.get('mode')):
                                result['payload_verified'] = False
                        for item in result['tools']:
                            if item['id'] in receipt.get('tools', ()) and item['id'] in present_tools:
                                item['installed'] = True
                        if not result['payload_verified']:
                            result['diagnostics'].append('Installed files differ from the last successful Setup receipt.')
            try:
                result['git'] = git_details(Path(project['path']), include_status=False)
                result['diagnostics'].append('Git branch metadata only; working-tree changes were not inspected.')
            except SessionError:
                result['diagnostics'].append('Git metadata is unavailable. Project execution can still use local files.')
            result['diagnostics'].append('Readiness is a static file check; project scripts and hooks were not executed.')
        except (SessionError, ValueError, TypeError, AttributeError):
            result['diagnostics'].append('The project is missing, unsafe, or contains invalid setup metadata. Restore it before setup.')
        finally:
            if descriptor is not None:
                os.close(descriptor)
        return result

    def close(self):
        self.closed.set()
        process = self.process
        if process is not None:
            self.sessions._signal(process, signal.SIGTERM)
        with self.lock:
            self.previews.clear()
