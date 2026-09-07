"""Browser access to existing Project Brain records and context CLI operations."""
from __future__ import annotations

from contextlib import contextmanager
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import selectors
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time
import uuid
import zipfile

from .sessions import SessionError, open_project_path, read_context

RECORD_TYPES = ('task', 'finding', 'bug', 'incident', 'decision', 'event')
FOLDERS = tuple('dynamic/' + name for name in ('tasks', 'findings', 'bugs', 'incidents', 'decisions', 'events')) + (
    'control/handoffs', 'control/promotions', 'control/retrieval-manifests',
) + tuple('archive/' + name for name in (*RECORD_TYPES, 'handoffs'))
PHASES = ('understanding', 'planning', 'implementation', 'verification', 'finalization')
READ_LIMIT = 256 * 1024
OUTPUT_LIMIT = 4 * 1024 * 1024
EXPORT_LIMIT = 32 * 1024 * 1024
EXECUTION_TIMEOUT = 60
ACTION_FIELDS = {
    'status': (), 'index': (), 'validate': (), 'bank-audit': (), 'reindex-bank': (), 'compact': (),
    'search': ('query', 'layer'),
    'retrieve': ('query', 'task_id', 'paths'),
    'refresh': ('query', 'task_id'),
    'rebind': ('task_id', 'record_id'),
    'bank-reverify': ('memory_id', 'review_after'),
    'bank-retire': ('memory_id', 'valid_to', 'superseded_by', 'reason'),
    'start': ('task_id', 'goal', 'files', 'sources'),
    'brain-create': ('record_type', 'external_id', 'title', 'goal', 'files', 'sources', 'privacy', 'authority'),
    'brain-update': ('record_id', 'revision', 'progress', 'next_steps', 'phase', 'transition', 'reason', 'authority'),
    'complete': ('task_id', 'revision', 'outcome', 'verification', 'sources'),
    'promote-propose': ('source_ids', 'title', 'content'),
    'promote-review': ('promotion_id', 'reviewer', 'reject'),
    'promote-apply': ('promotion_id',),
    'export': ('include_archive', 'include_superseded'),
}

# Run the installed runtime's own policy helpers in its guarded subprocess.
# Project code is never imported into the HTTP server's Python process.
BRAIN_INSPECT = r'''
import json, sys
from pathlib import Path
root = Path(sys.argv[1])
sys.path.insert(0, str(root / 'memory-bank/scripts'))
from brain_runtime import (load_config, find_record, get_task, iter_records,
    iter_promotions, validate_record, validate_promotion_record, sources_are_fresh,
    promotion_content, promoted_source_ids, PROMOTABLE_STATES)
request = json.loads(sys.argv[2])
config = load_config(root)
privacy = set(config.get('allowed_privacy', ['public', 'team'])) & {'public', 'team'}
def eligible(record):
    validate_record(record)
    return (record['status'] in PROMOTABLE_STATES.get(record['type'], set())
        and record['authority'] == 'verified' and record['privacy'] in privacy
        and sources_are_fresh(root, record) and promotion_content(record) is not None)
if request.get('check'):
    source_ids = request.get('source_ids', [])
    if request.get('promotion_id'):
        proposal = next((item for key, item in iter_promotions(root)
                         if key == request['promotion_id']), None)
        if proposal is None:
            raise ValueError('Promotion is unavailable.')
        validate_promotion_record(root, proposal, expected_id=request['promotion_id'])
        source_ids = [source['id'] for source in proposal['source_records']]
    if not source_ids:
        raise ValueError('Select eligible source records.')
    for identifier in source_ids:
        _, record, _ = find_record(root, identifier, include_archive=True)
        if not eligible(record):
            print(json.dumps({'eligible': False}))
            raise SystemExit(0)
    print(json.dumps({'eligible': True}))
elif request.get('task_only'):
    print(json.dumps({'task': get_task(root, request['task_id'])}, ensure_ascii=False))
else:
    records, candidates = [], []
    already = promoted_source_ids(root)
    for _, record, _ in iter_records(root, include_archive=True):
        validate_record(record)
        if len(records) >= 500:
            raise ValueError('Too many Brain records for this view.')
        records.append(record)
        if record['id'] not in already and eligible(record):
            candidates.append({**record, 'promotion_content': promotion_content(record)})
    promotions = [item for _, item in iter_promotions(root)]
    if len(promotions) > 500:
        raise ValueError('Too many promotions for this view.')
    print(json.dumps({'task': get_task(root, request['task_id']) if request.get('task_id') else None,
        'records': records, 'eligible_sources': candidates, 'promotions': promotions,
        'automatic_promotion': bool(config.get('automatic_promotion'))}, ensure_ascii=False))
'''


def _text(value, label, limit=16000):
    try:
        size = len(value.encode('utf-8')) if isinstance(value, str) else limit + 1
    except UnicodeError as error:
        raise SessionError('Enter valid ' + label + '.') from error
    if (not isinstance(value, str) or not value.strip() or size > limit
            or any(ord(char) < 32 and char not in '\n\t' for char in value) or '\x7f' in value):
        raise SessionError('Enter valid ' + label + '.')
    return value


def _path(value):
    value = _text(value, 'relative path', 1024)
    path = PurePosixPath(value)
    if (value == '.' or path.is_absolute() or str(path) != value or '..' in path.parts or '\\' in value
            or any(ord(char) < 32 for char in value)):
        raise SessionError('Use a relative project path without parent traversal.')
    return value


def _metadata(content, suffix):
    try:
        if suffix == '.json':
            value = json.loads(content)
        elif content.startswith('---\n'):
            end = content.find('\n---', 4)
            value = json.loads(content[4:end]) if end >= 0 else {}
        else:
            value = {}
        # A malformed local JSON scalar must not break the browser's JSON envelope.
        json.dumps(value, ensure_ascii=False, allow_nan=False).encode('utf-8')
        return value if isinstance(value, dict) else {}
    except (ValueError, RecursionError):
        return {}


def _entry(path, size, content):
    metadata = _metadata(content, Path(path).suffix)
    title = metadata.get('title') or metadata.get('objective')
    if not isinstance(title, str) or not title.strip():
        title = next((line[2:] for line in content.splitlines() if line.startswith('# ')), Path(path).name)
    result = {'path': path, 'title': title[:200], 'bytes': size,
              'category': path.rsplit('/', 1)[0]}
    for field in ('type', 'status', 'id', 'external_id', 'owner', 'phase'):
        result[field] = metadata.get(field)[:200] if isinstance(metadata.get(field), str) else None
    result['revision'] = metadata.get('revision') if type(metadata.get('revision')) is int else None
    return result, metadata


class KnowledgeManager:
    def __init__(self, sessions):
        self.sessions = sessions
        self.lock = threading.Lock()
        self.closed = threading.Event()
        self.process = None
        self.temporary = tempfile.TemporaryDirectory(prefix='knowledge-', dir=str(sessions.state_dir))
        self.exports = {}

    @contextmanager
    def _operation(self):
        if not self.lock.acquire(blocking=False):
            raise SessionError('Another knowledge operation is running. Wait for it to finish.')
        try:
            yield
        finally:
            self.lock.release()

    def info(self, project_id, bank=None, *, _root=None):
        listing = self.sessions.memory(project_id, bank=bank, **({'_root': _root} if _root is not None else {}))
        selected = listing['bank_id']
        root = str(PurePosixPath(selected).parent) if selected else ''
        root = '' if root == '.' else root
        project = Path(_root) if _root is not None else Path(self.sessions.project(project_id)['path'])
        prefix = root + '/' if root else ''
        available = selected is not None and read_context(project, selected + '/scripts/context.py') is not None
        brain_available = False
        if selected is not None:
            try:
                descriptor = open_project_path(project, prefix + 'project-brain', directory=True)
                os.close(descriptor)
                brain_available = True
            except OSError:
                pass
        mode = None
        if selected is not None:
            mode = 'governed'
            config = read_context(project, prefix + 'project-brain/config/runtime.json', 65536)
            if config is not None and config[0] <= 65536:
                settings = _metadata(config[1], '.json')
                if settings.get('mode') in ('governed', 'lightweight'):
                    mode = settings['mode']
            # Match configured_mode() in the selected CLI instead of presenting
            # a governed editor while the runtime would create local tasks.
            override = os.environ.get('PROJECT_BRAIN_MODE')
            if override:
                mode = override if override in ('governed', 'lightweight') else None
        return {'project_id': project_id, 'banks': listing['banks'], 'bank_id': selected,
                'runtime_available': available, 'brain_available': brain_available, 'root': root, 'mode': mode}

    def brain(self, project_id, bank=None, path=None, *, _root=None):
        info = self.info(project_id, bank, _root=_root)
        project = Path(_root) if _root is not None else Path(self.sessions.project(project_id)['path'])
        prefix = (info['root'] + '/' if info['root'] else '') + 'project-brain/'
        if path is not None:
            path = _path(path)
            if path.rsplit('/', 1)[0] not in FOLDERS or Path(path).suffix not in ('.md', '.json'):
                raise SessionError('Select a supported Project Brain record.')
            content = read_context(project, prefix + path, READ_LIMIT) if info['brain_available'] else None
            if content is None:
                raise SessionError('Project Brain record is unavailable.')
            entry, metadata = _entry(path, *content)
            return {**info, **entry, 'content': content[1], 'metadata': metadata, 'truncated': content[0] > READ_LIMIT}
        entries, scanned, consumed, truncated = [], 0, 0, False
        if info['brain_available']:
            for folder in FOLDERS:
                try:
                    descriptor = open_project_path(project, prefix + folder, directory=True)
                except OSError:
                    continue
                try:
                    with os.scandir(descriptor) as names:
                        for item in names:
                            scanned += 1
                            if scanned > 5000 or len(entries) >= 500 or consumed >= 8 * 1024 * 1024:
                                truncated = True
                                break
                            if item.name.startswith('.') or Path(item.name).suffix not in ('.md', '.json'):
                                continue
                            relative = folder + '/' + item.name
                            content = read_context(project, prefix + relative, 65536)
                            if content is None:
                                continue
                            consumed += min(content[0], 65536)
                            entries.append(_entry(relative, *content)[0])
                finally:
                    os.close(descriptor)
                if truncated:
                    break
        entries.sort(key=lambda item: (item['category'], item['title'].casefold(), item['path']))
        return {**info, 'entries': entries, 'truncated': truncated}

    def _preflight(self, root):
        """Reject redirected existing runtime stores before asking their CLI to write."""
        scanned = 0
        for folder in ('memory-bank', 'project-brain'):
            try:
                descriptor = open_project_path(root, folder, directory=True)
            except FileNotFoundError:
                continue
            except OSError as error:
                raise SessionError('Knowledge storage must use ordinary project directories.') from error
            pending = [descriptor]
            try:
                while pending:
                    current = pending.pop()
                    try:
                        with os.scandir(current) as names:
                            for item in names:
                                scanned += 1
                                if scanned > 20000:
                                    raise SessionError('Knowledge storage exceeds the supported file count.')
                                metadata = item.stat(follow_symlinks=False)
                                if stat.S_ISDIR(metadata.st_mode):
                                    pending.append(os.open(item.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=current))
                                elif not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1:
                                    raise SessionError('Knowledge storage contains links or unsupported file types.')
                    finally:
                        os.close(current)
            except OSError as error:
                raise SessionError('Knowledge storage changed or contains unsafe paths.') from error
            finally:
                for descriptor in pending:
                    os.close(descriptor)

    def _arguments(self, action, data):
        arguments = [action]
        required = {
            'search': ('query',), 'bank-reverify': ('memory_id',),
            'retrieve': ('query', 'task_id'), 'refresh': ('query', 'task_id'), 'rebind': ('task_id',),
            'bank-retire': ('memory_id', 'valid_to'), 'start': ('task_id', 'goal'),
            'brain-create': ('record_type', 'external_id', 'title'),
            'brain-update': ('record_id', 'revision'), 'complete': ('task_id', 'revision', 'outcome'),
            'promote-propose': ('source_ids', 'title', 'content'),
            'promote-review': ('promotion_id', 'reviewer'), 'promote-apply': ('promotion_id',),
        }.get(action, ())
        if any(field not in data for field in required):
            raise SessionError('Required knowledge operation fields are missing.')
        if action == 'brain-create':
            if data['record_type'] not in RECORD_TYPES[1:]:
                raise SessionError('Select finding, bug, incident, decision or event.')
            arguments.append(data['record_type'])
        query = None
        if action in ('search', 'retrieve'):
            query = _text(data['query'], 'context query', 4000)
            if 'layer' in data:
                if data['layer'] not in ('procedural', 'semantic', 'episodic'):
                    raise SessionError('Select a supported context layer.')
                arguments.extend(['--layer', data['layer']])
        lists = {'files': 'file', 'sources': 'source', 'next_steps': 'next-step', 'verification': 'verification',
                 'paths': 'path', 'source_ids': 'source-id'}
        names = {'memory_id': 'id', 'record_id': 'record-id', 'task_id': 'task-id',
                 'external_id': 'external-id', 'review_after': 'review-after',
                 'valid_to': 'valid-to', 'superseded_by': 'superseded-by'}
        if action == 'rebind':
            names['record_id'] = 'record'
        for field in ACTION_FIELDS[action]:
            if field not in data or field in ('record_type', 'layer') or (field == 'query' and query is not None):
                continue
            value = data[field]
            if field in ('include_archive', 'include_superseded', 'reject'):
                if type(value) is not bool:
                    raise SessionError('Operation options must be booleans.')
                if value:
                    arguments.append('--' + field.replace('_', '-'))
            elif field == 'revision':
                if type(value) is not int or not 1 <= value <= 2147483647:
                    raise SessionError('Revision must be a positive integer from the displayed record.')
                arguments.extend(['--revision', str(value)])
            elif field in lists:
                if not isinstance(value, list) or len(value) > 50:
                    raise SessionError('Enter at most 50 values for ' + field + '.')
                if field == 'source_ids' and (not value or len(value) > 20):
                    raise SessionError('Select between 1 and 20 source records.')
                for item in value:
                    item = _path(item) if field in ('files', 'paths') else _text(item, field, 4000)
                    if field == 'source_ids' and not re.fullmatch(r'[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}', item):
                        raise SessionError('Select source records with UUIDv4 IDs.')
                    arguments.append('--' + lists[field] + '=' + item)
                if field == 'source_ids' and len(set(value)) != len(value):
                    raise SessionError('Select distinct source records.')
            else:
                value = _text(value, field.replace('_', ' '), 16000 if field in ('goal', 'progress', 'outcome', 'content', 'query') else 2000)
                if field in ('task_id', 'external_id'):
                    value = value.strip()
                    if (not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._/-]{0,199}', value)
                            or '..' in value.split('/')):
                        raise SessionError('Use a task ID with letters, digits, dots, underscores, slashes or hyphens.')
                if field in ('record_id', 'promotion_id'):
                    value = value.strip()
                    if not re.fullmatch(r'[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}', value):
                        raise SessionError('Select a Project Brain record with a UUIDv4 ID.')
                if field == 'phase' and value not in PHASES:
                    raise SessionError('Select a supported task phase.')
                if field == 'privacy' and value not in ('public', 'team', 'restricted', 'private'):
                    raise SessionError('Select a supported record privacy level.')
                if field == 'authority' and value not in ('inferred', 'observed', 'verified'):
                    raise SessionError('Select a supported evidence authority.')
                if field in ('memory_id', 'superseded_by') and not re.fullmatch(r'MEM-(?:\d+|\d{8}-[0-9a-f]{8})', value):
                    raise SessionError('Select a valid Memory Bank ID.')
                if field in ('review_after', 'valid_to'):
                    from datetime import date
                    try:
                        if date.fromisoformat(value).isoformat() != value:
                            raise ValueError()
                    except ValueError as error:
                        raise SessionError('Use a date in YYYY-MM-DD format.') from error
                arguments.append('--' + names.get(field, field.replace('_', '-')) + '=' + value)
        if action == 'brain-update' and not any(field in data for field in ('progress', 'next_steps', 'phase', 'transition', 'authority')):
            raise SessionError('Choose a record field or lifecycle transition to update.')
        if query is not None:
            # Positional queries follow --, so leading hyphens stay literal.
            return arguments + ['--limit', '20' if action == 'search' else '3', '--json', '--', query]
        return arguments + ['--json']

    def _execute(self, command, root):
        read_fd, write_fd = os.pipe()
        process = None
        selector = selectors.DefaultSelector()
        output = [bytearray(), bytearray()]
        try:
            guarded = [sys.executable, str(Path(__file__).with_name('process_guard.py')), str(read_fd), '--', *command]
            environment = {key: value for key, value in os.environ.items() if key not in ('PYTHONPATH', 'PYTHONHOME')}
            environment['PYTHONDONTWRITEBYTECODE'] = '1'
            process = subprocess.Popen(guarded, cwd=root, env=environment, stdin=subprocess.DEVNULL,
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True,
                                       pass_fds=(read_fd, self.sessions.runner_lock))
            self.process = process
            os.close(read_fd)
            read_fd = None
            selector.register(process.stdout, selectors.EVENT_READ, 0)
            selector.register(process.stderr, selectors.EVENT_READ, 1)
            started = time.monotonic()
            while selector.get_map():
                if self.closed.is_set() or self.sessions.stopping.is_set():
                    raise SessionError('Knowledge operation was stopped. Refresh the records before retrying; changes may have been applied.')
                if time.monotonic() - started > EXECUTION_TIMEOUT:
                    raise SessionError('Knowledge operation timed out. Refresh the records before retrying; changes may have been applied.')
                ready = selector.select(.1)
                if not ready and process.poll() is not None:
                    break
                for key, _ in ready:
                    chunk = os.read(key.fileobj.fileno(), 65536)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    output[key.data].extend(chunk)
                    if len(output[0]) > OUTPUT_LIMIT or len(output[1]) > 65536:
                        raise SessionError('Knowledge runtime output exceeded its limit. Refresh the records before retrying; changes may have been applied.')
            return process.wait(timeout=3), bytes(output[0]), bytes(output[1])
        finally:
            if read_fd is not None:
                os.close(read_fd)
            os.close(write_fd)
            if process is not None:
                try:
                    process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    pass
                self.sessions._signal(process, signal.SIGKILL)
                process.wait(timeout=3)
                process.stdout.close()
                process.stderr.close()
            self.process = None
            selector.close()

    def _zip(self, destination):
        stream, total, count = io.BytesIO(), 0, 0
        with zipfile.ZipFile(stream, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
            for folder, directories, files in os.walk(destination, followlinks=False):
                for name in directories:
                    if (Path(folder) / name).is_symlink():
                        raise SessionError('Export contains an unsafe directory.')
                for name in files:
                    path = Path(folder) / name
                    relative = _path(path.relative_to(destination).as_posix())
                    descriptor = open_project_path(destination, relative)
                    try:
                        metadata = os.fstat(descriptor)
                        count += 1
                        total += metadata.st_size
                        if (not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1
                                or count > 5000 or total > EXPORT_LIMIT):
                            raise SessionError('Export exceeds supported bounds or contains unsafe files.')
                        data = bytearray()
                        while len(data) <= metadata.st_size:
                            chunk = os.read(descriptor, min(65536, metadata.st_size + 1 - len(data)))
                            if not chunk:
                                break
                            data.extend(chunk)
                        if len(data) != metadata.st_size:
                            raise SessionError('Export changed while it was being read.')
                        archive.writestr(relative, data)
                    finally:
                        os.close(descriptor)
        return stream.getvalue()

    def _inspect_runtime(self, root, request):
        code, stdout, _ = self._execute([sys.executable, '-c', BRAIN_INSPECT, str(root), json.dumps(request)], root)
        try:
            result = json.loads(stdout)
            json.dumps(result, ensure_ascii=False, allow_nan=False).encode('utf-8')
        except (ValueError, RecursionError) as error:
            raise SessionError('The installed Brain runtime could not inspect its records.') from error
        if code != 0 or not isinstance(result, dict):
            raise SessionError('The installed Brain runtime could not inspect its records. Check record validity and source paths.')
        return result

    def inspect(self, project_id, bank=None, *, _root=None, task_id=None, task_only=False):
        with self._operation():
            if self.closed.is_set() or self.sessions.stopping.is_set():
                raise SessionError('The knowledge manager is stopping.')
            info = self.info(project_id, bank, _root=_root)
            if not info['runtime_available'] or info['mode'] != 'governed':
                raise SessionError('An installed governed Brain runtime is required.')
            project = Path(_root) if _root is not None else Path(self.sessions.project(project_id)['path'])
            root = project / info['root']
            self._preflight(root)
            return {**info, **self._inspect_runtime(root, {'task_id': task_id, 'task_only': task_only})}

    def run(self, project_id, data, *, _root=None, _ephemeral=False, _gate=None):
        if not isinstance(data, dict) or not isinstance(data.get('action'), str) or data['action'] not in ACTION_FIELDS:
            raise SessionError('Select a supported knowledge operation.')
        action = data['action']
        if set(data) - {'action', 'bank', *ACTION_FIELDS[action]}:
            raise SessionError('Unknown knowledge operation field.')
        arguments = self._arguments(action, data)
        with self._operation():
            if self.closed.is_set() or self.sessions.stopping.is_set():
                raise SessionError('The knowledge manager is stopping.')
            info = self.info(project_id, data.get('bank'), _root=_root)
            if not info['runtime_available']:
                raise SessionError('The selected Memory Bank does not have an installed context runtime.')
            if action in ('start', 'brain-create', 'brain-update', 'complete', 'rebind',
                          'promote-propose', 'promote-review', 'promote-apply') and info['mode'] != 'governed':
                raise SessionError('Project Brain editing requires the project to use governed mode.')
            project = Path(_root) if _root is not None else Path(self.sessions.project(project_id)['path'])
            root = project / info['root']
            self._preflight(root)
            if action in ('retrieve', 'refresh'):
                options = []
                if _ephemeral:
                    options.append('--ephemeral')
                if _gate is not None:
                    if _gate not in ('off', 'shadow', 'enforce'):
                        raise SessionError('Invalid internal retrieval gate.')
                    options.append('--gate=' + _gate)
                arguments[1:1] = options
            export_id, destination = None, None
            if action == 'export':
                export_id = str(uuid.uuid4())
                destination = Path(self.temporary.name) / export_id
                arguments.extend(['--destination', str(destination)])
            command = [sys.executable, str(root / 'memory-bank/scripts/context.py'), '--root', str(root), *arguments]
            try:
                if action in ('promote-propose', 'promote-apply'):
                    eligibility = self._inspect_runtime(root, {'check': True, **{
                        field: data[field] for field in ('source_ids', 'promotion_id') if field in data}})
                    if eligibility.get('eligible') is not True:
                        raise SessionError('Promote only verified resolved findings or bugs, closed incidents, or accepted decisions with allowed privacy, reusable progress and unchanged sources. Tasks and session transcripts are not durable knowledge.')
                code, stdout, stderr = self._execute(command, root)
                result = json.loads(stdout) if stdout.strip() else None
                if not isinstance(result, (dict, list)):
                    result = None
                json.dumps(result, ensure_ascii=False, allow_nan=False).encode('utf-8')
            except (OSError, ValueError, RecursionError, subprocess.TimeoutExpired, SessionError) as error:
                if destination is not None:
                    shutil.rmtree(destination, ignore_errors=True)
                return {'ok': False, 'action': action, 'result': None,
                        'error': str(error) if isinstance(error, SessionError) else 'The context runtime could not finish this operation. Refresh before retrying; changes may have been applied.'}
            response = {'ok': code == 0, 'action': action, 'result': result}
            if code != 0:
                detail = stderr.decode('utf-8', errors='replace')
                message = 'The context runtime rejected this operation. Check record fields, sources and project validation.'
                if 'Stale ' in detail and 'revision' in detail:
                    message = 'This record changed. Reload it and repeat the edit using its current revision.'
                elif 'Owner is not authorized' in detail:
                    message = 'The configured Project Brain owner is not authorized to edit this record.'
                elif 'binding' in detail.lower() or 'Working task not found' in detail:
                    message = 'The task is not bound to this local runtime. Restore its binding with the project context CLI first.'
                elif 'Illegal ' in detail and 'transition' in detail:
                    message = 'This lifecycle transition is not allowed from the current record state.'
                elif 'secret' in detail.lower():
                    message = 'The runtime refused content matching its secret-protection rules.'
                response['error'] = message
            elif result is None:
                response.update(ok=False, error='The context runtime did not return a valid JSON result. Refresh before retrying; changes may have been applied.')
            elif destination is not None:
                try:
                    payload = self._zip(destination)
                except (OSError, SessionError) as error:
                    response.update(ok=False, error=str(error) if isinstance(error, SessionError) else 'The export could not be packaged safely.')
                else:
                    # Bound retained downloads as well as each individual bundle.
                    if len(self.exports) >= 8:
                        self.exports.pop(next(iter(self.exports)))
                    self.exports[export_id] = payload
                    response['download_id'] = export_id
                    # The CLI destination is temporary staging, not the downloadable artifact.
                    if isinstance(result, dict):
                        response['result'] = {key: value for key, value in result.items() if key != 'destination'}
            if destination is not None:
                shutil.rmtree(destination, ignore_errors=True)
            return response

    def download(self, export_id):
        if not isinstance(export_id, str) or not re.fullmatch(r'[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}', export_id):
            raise SessionError('Export is unavailable.')
        with self.lock:
            if self.closed.is_set() or export_id not in self.exports:
                raise SessionError('Export is unavailable. Create a new export.')
            return self.exports[export_id]

    def close(self):
        self.closed.set()
        process = self.process
        if process is not None:
            self.sessions._signal(process, signal.SIGTERM)
        with self.lock:
            self.exports.clear()
            self.temporary.cleanup()
