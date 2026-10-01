"""Reviewed system plans and dispatches through the existing Harness queue."""
from __future__ import annotations

from functools import wraps
import json
import os
from pathlib import Path
import re
import sys
import uuid

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'scripts'))
from ai_system_lib import System, SystemError, open_directory
import ai_system_execution as execution
from .sessions import SessionError, ACTIVE, open_project_path, now


def boundary(function):
    @wraps(function)
    def checked(*args, **kwargs):
        try:
            return function(*args, **kwargs)
        except SystemError as error:
            raise SessionError(str(error)) from None
        except OSError:
            raise SessionError('System files are unavailable or use an unsafe path.') from None
    return checked


def options(data, required, optional=()):
    if not isinstance(data, dict) or not set(required) <= set(data) or set(data) - set(required) - set(optional):
        raise SessionError('Invalid system orchestration options.')


def run_id(value):
    if not isinstance(value, str) or not re.fullmatch('[a-f0-9]{32}', value):
        raise SessionError('Invalid system run ID.')
    return value


class SystemManager:
    def __init__(self, sessions):
        self.sessions = sessions
        self.root = sessions.state_dir / 'ai-system'
        self.root.mkdir(mode=0o700, exist_ok=True)
        fd = open_directory(self.root)
        os.close(fd)
        with sessions.lock:
            sessions.db.execute('CREATE TABLE IF NOT EXISTS system_runs (id TEXT PRIMARY KEY, data TEXT NOT NULL)')
            columns = {row[1] for row in sessions.db.execute('PRAGMA table_info(system_runs)')}
            if 'project_id' not in columns:
                sessions.db.execute('ALTER TABLE system_runs ADD COLUMN project_id TEXT')
                for row in sessions.db.execute('SELECT id,data FROM system_runs').fetchall():
                    sessions.db.execute('UPDATE system_runs SET project_id=? WHERE id=?',
                                        (json.loads(row[1])['project_id'], row[0]))
            sessions.db.execute('CREATE INDEX IF NOT EXISTS system_runs_project ON system_runs(project_id)')
            sessions.db.commit()
            for row in sessions.db.execute('SELECT id FROM system_runs').fetchall():
                self._record(row[0])

    def _system(self, project_id, config_path):
        project = self.sessions.project(project_id)
        fd = open_project_path(Path(project['path']), config_path)
        os.close(fd)
        # A manifest declares relationships, never host filesystem permissions.
        roots = [p['path'] for p in self.sessions.projects.values()]
        return System(Path(project['path']) / config_path, roots), roots

    def _record(self, rid):
        run_id(rid)
        row = self.sessions.db.execute('SELECT data FROM system_runs WHERE id=?', (rid,)).fetchone()
        if row is None:
            raise SessionError('Unknown system run.')
        record = json.loads(row[0])
        nonce = record.get('pending_nonce')
        if nonce:
            # The request/authorization is durable before enqueue. Recover the
            # association if the server died after committing the session.
            for session in self.sessions.db.execute('SELECT id,system_run FROM sessions WHERE system_run IS NOT NULL').fetchall():
                metadata = json.loads(session[1])
                if metadata == {'run_id': rid, 'nonce': nonce}:
                    record['session_id'] = session[0]
                    break
            record['pending_nonce'] = None
            self._save(record)
        return record

    def _save(self, record):
        self.sessions.db.execute('INSERT OR REPLACE INTO system_runs (id,data,project_id) VALUES (?,?,?)',
                                 (record['id'], json.dumps(record, ensure_ascii=False), record['project_id']))
        self.sessions.db.commit()

    @boundary
    def catalog(self, data):
        options(data, ('project_id', 'config_path'))
        with self.sessions.lock:
            system, _ = self._system(data['project_id'], data['config_path'])
            return {'catalog': system.catalog(), 'diagram': system.diagram(), 'root': str(system.root)}

    @boundary
    def prepare(self, data):
        options(data, ('project_id', 'config_path', 'task', 'change_id'), ('services', 'contracts', 'budget', 'depth'))
        if not isinstance(data['task'], str) or len(data['task']) > 2000:
            raise SessionError('Describe the task in at most 2000 characters.')
        task = data['task'].replace('\r\n', ' ').replace('\n', ' ').replace('\r', ' ')
        for name in ('services', 'contracts'):
            values = data.get(name, [])
            if not isinstance(values, list) or len(values) > 128 or any(not isinstance(v, str) for v in values):
                raise SessionError('Service and contract selections must be bounded lists of IDs.')
        with self.sessions.lock:
            system, _ = self._system(data['project_id'], data['config_path'])
            plan = system.plan(task, data['change_id'], data.get('services', []),
                               data.get('contracts', []), data.get('budget', 8000), data.get('depth', 32))
            rid = uuid.uuid4().hex
            directory = self.root / rid
            directory.mkdir(mode=0o700)
            execution.save(directory, 'approved-plan.json', plan, new=True)
            record = {'id': rid, 'revision': 0, 'created_at': now(), 'project_id': data['project_id'],
                      'config_path': data['config_path'], 'system': plan['system'], 'change_id': plan['change_id'],
                      'session_id': None, 'mode': None, 'timeout': None}
            self._save(record)
            return self.get(rid)

    @boundary
    def list(self, project_id):
        with self.sessions.lock:
            self.sessions.project(project_id)
            rows = self.sessions.db.execute('SELECT id FROM system_runs WHERE project_id=? ORDER BY rowid DESC LIMIT 200',
                                           (project_id,)).fetchall()
            return {'runs': [self._view(self._record(row[0]), detail=False) for row in rows]}

    def _view(self, record, detail=True):
        result = dict(record)
        session = self.sessions.get(record['session_id']) if record['session_id'] else None
        result['session_status'] = session['status'] if session else None
        result['active'] = bool(session and session['status'] in ACTIVE)
        directory = self.root / record['id']
        journal = directory / 'execution'
        result['status'] = session['status'] if session else 'needs_review'
        if (journal / 'run.json').exists():
            state = execution.load(journal, 'run.json')
            result['status'] = state['status']
            if result['active']:
                result['status'] = session['status']
            elif result['status'] in ('prepared', 'running'):
                result['status'] = 'interrupted'
            if detail:
                result['execution'] = execution.run_summary(state, journal)
                receipts = []
                for step in state['steps']:
                    if step['attempt'] and (journal / execution.receipt_name(step)).exists():
                        receipt = execution.load(journal, execution.receipt_name(step))
                        receipts.append({'phase': step['id'], 'service': step['service'],
                                         'attempt': step['attempt'], 'ok': receipt['ok'],
                                         'error': receipt['error'], 'report': receipt['report']})
                result['receipts'] = receipts
                if (journal / 'handoff.json').exists():
                    result['handoff'] = execution.load(journal, 'handoff.json')
        elif session and not result['active']:
            result['status'] = 'interrupted'
        if detail:
            result['plan'] = execution.load(directory, 'approved-plan.json')
            result['events'] = self.sessions.events(record['session_id'])[-30:] if session else []
            result['codex_available'] = bool(self.sessions.providers.get('codex', {}).get('available'))
        return result

    @boundary
    def get(self, rid):
        with self.sessions.lock:
            return self._view(self._record(rid))

    @boundary
    def act(self, rid, data):
        options(data, ('action', 'revision'), ('mode', 'timeout', 'retry_step', 'accept_source_changes'))
        with self.sessions.lock:
            record = self._record(rid)
            if type(data['revision']) is not int or data['revision'] != record['revision']:
                raise SessionError('Run changed in another view. Refresh before acting.')
            action = data['action']
            allowed = {'execute': {'action', 'revision', 'mode', 'timeout'},
                       'resume': {'action', 'revision', 'retry_step', 'accept_source_changes'},
                       'cancel': {'action', 'revision'}}
            if action not in allowed or set(data) - allowed[action]:
                raise SessionError('Invalid system run action.')
            view = self._view(record)
            if action == 'cancel':
                if view['active']:
                    self.sessions.cancel(record['session_id'])
                return self._view(record)
            if view['active']:
                raise SessionError('Wait for this launch to stop before continuing.')
            system, roots = self._system(record['project_id'], record['config_path'])
            directory = self.root / rid
            plan = execution.load(directory, 'approved-plan.json')
            if action == 'execute':
                if record['session_id'] is not None:
                    raise SessionError('This plan was already launched. Use explicit recovery.')
                execution.selected_plan(system, plan)
                if not system.verify(plan)['fresh']:
                    raise SessionError('Plan is stale. Prepare and review a new plan.')
                mode, timeout = data.get('mode', 'read-only'), data.get('timeout', 900)
                if mode not in ('read-only', 'edit') or type(timeout) is not int or not 1 <= timeout <= 86400:
                    raise SessionError('Choose read-only/edit and a worker timeout of 1..86400 seconds.')
            else:
                if record['session_id'] is None or view['status'] == 'completed':
                    raise SessionError('Only unfinished launches can be resumed.')
                mode, timeout = record['mode'], record['timeout']
                if type(data.get('accept_source_changes', False)) is not bool:
                    raise SessionError('Source change acknowledgement must be a boolean.')
                retry = data.get('retry_step')
                if retry is not None and (not isinstance(retry, str) or len(retry) > 100):
                    raise SessionError('Choose the interrupted or blocked dispatch to retry.')
                if (directory / 'execution' / 'run.json').exists():
                    state = execution.validate_state(system, directory / 'execution')
                    if retry is not None and retry not in {s['id'] for s in state['steps'] if s['status'] in ('running', 'interrupted', 'blocked')}:
                        raise SessionError('Only an unfinished dispatch may be retried.')
                elif not system.verify(plan)['fresh']:
                    raise SessionError('Plan is stale. Prepare and review a new plan.')
            provider = self.sessions.providers.get('codex', {})
            if not provider.get('available'):
                raise SessionError('System execution currently requires an available Codex CLI.')
            nonce = uuid.uuid4().hex
            request = {'directory': str(directory), 'system_file': str(system.config), 'allowed_roots': roots,
                       'executable': provider['executable'], 'mode': mode, 'timeout': timeout,
                       'retry_step': data.get('retry_step'), 'accept_source_changes': data.get('accept_source_changes', False)}
            execution.save(directory, 'request-' + nonce + '.json', request, new=True)
            record.update(pending_nonce=nonce, mode=mode, timeout=timeout, revision=record['revision'] + 1)
            self._save(record)
            session = self.sessions.create({'project_id': record['project_id'], 'provider': 'codex',
                       'workflow': 'native', 'mode': 'edit' if mode == 'edit' else 'plan',
                       'prompt': 'System orchestration: ' + plan['context']['task'], 'project_context': False,
                       'agents_enabled': False, 'budgets': {'seconds': None, 'tokens': None, 'usd': None}},
                       _system_run={'run_id': rid, 'nonce': nonce})
            record.update(session_id=session['id'], pending_nonce=None)
            self._save(record)
            return self._view(record)
