"""Browser checkpoints for the existing Infrastructure-Creator workflows."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sys
import threading
import uuid

from .sessions import SessionError, model_settings, now, DEFAULT_AGENT_COUNT, MAX_AGENTS, git_details, validate_budgets, DEFAULT_BUDGETS
from .setup import _read, _root_fd, _identity, _diff, _metadata

ROOT = Path(__file__).resolve().parents[3]
GENERATOR = ROOT / 'Infrastructure-Creator'
SCRIPTS = GENERATOR / '.agents/skills/bootstrap-verifier/scripts'
REGISTRY = GENERATOR / '.agents/skills/skill-forge/references/candidate-registry.json'
TASK = 'tasks/TASK-001'
REPORTS = ('infra-scan-project-profile.md', 'skill-generation-plan.json', 'skill-plan-quality-report.json',
           'infra-scan-rejection-report.md', 'harness-questions.md', 'infra-generate-report.md', 'infra-validate-review.json')
PLAN_NAMES = ('infra-generate-publication-plan.txt', 'infra-generate-removal-plan.txt', 'infra-generate-watch-plan.txt')
ACTIVE = ('queued', 'running')


def load_file(root, name, required=True):
    fd = _root_fd(Path(root))
    try:
        return _read(fd, name, required=required)
    finally:
        os.close(fd)


def write_json(path, value):
    temporary = path.with_name('.' + uuid.uuid4().hex)
    try:
        temporary.write_text(json.dumps(value, ensure_ascii=False))
        temporary.chmod(0o600)
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def sandbox_command(workspace, target, command, provider=None, runtime_cache=False):
    executable = shutil.which('bwrap')
    if not executable:
        raise SessionError('Creator requires bubblewrap on Linux to keep the target read-only during agent runs.')
    args = [executable, '--die-with-parent', '--new-session', '--unshare-pid', '--ro-bind', '/', '/',
            '--proc', '/proc', '--dev', '/dev', '--tmpfs', '/tmp']
    if (target/'.git').is_file():
        common = git_details(target).get('common_dir')
        if common: args += ['--ro-bind', common, common]
    # Native account state stays with the native CLI; never copy credentials.
    if provider:
        executable_dir = Path(command[0]).resolve().parent
        args += ['--ro-bind', str(executable_dir), str(executable_dir)]
        homes = {'codex': [Path(os.environ.get('CODEX_HOME', str(Path.home()/'.codex')))],
                 'claude': [Path.home()/'.claude'],
                 'cursor': [Path.home()/'.cursor', Path.home()/'.config/cursor-agent']}
        for home in homes[provider]:
            if home.is_dir() and not home.is_symlink():
                args += ['--bind', str(home), str(home)]
    args += ['--ro-bind', str(workspace.parent), str(workspace.parent),
             '--bind', str(workspace), str(workspace), '--ro-bind', str(target), str(target),
             '--chdir', str(workspace)]
    if runtime_cache:
        cache = target/'memory-bank/local'
        fd = _root_fd(cache); os.close(fd)
        args += ['--tmpfs', str(cache)]
    return args + ['--', *command]


def read_plan(task, name, optional=False):
    value = load_file(task, name, required=not optional)
    paths = [] if value is None else value['body'].decode('utf-8').splitlines()
    paths = [path.strip() for path in paths if path.strip()]
    if len(paths) > 5000 or len(paths) != len(set(paths)):
        raise SessionError('Publication plans must contain distinct bounded paths.')
    for path in paths:
        if (not path or Path(path).is_absolute() or '..' in Path(path).parts or str(Path(path)) != path
                or '\\' in path or any(ord(c) < 32 for c in path)):
            raise SessionError('An output plan contains an unsafe path.')
    return paths


def allowed_output(path):
    return path in ('AGENTS.md', 'CLAUDE.md', 'DOD.md', 'GOLDEN-PRINCIPLES.md', 'STABILIZATION.md',
                    '.gitignore', '.infra-manifest.json', 'spec-desc.md') or path.split('/')[0] in (
                        '.agents', '.claude', '.codex', '.cursor', 'memory-bank', 'project-brain', 'specs', 'tasks')


class CreatorManager:
    def __init__(self, sessions):
        self.sessions = sessions
        self.root = sessions.state_dir / 'creator'
        self.root.mkdir(mode=0o700, exist_ok=True)
        fd = _root_fd(self.root); os.close(fd)
        self.lock = threading.RLock()
        with sessions.lock:
            sessions.db.execute('CREATE TABLE IF NOT EXISTS creator_runs (id TEXT PRIMARY KEY, data TEXT NOT NULL)')
            sessions.db.commit()

    def _save(self, run):
        with self.sessions.lock:
            self.sessions.db.execute('INSERT OR REPLACE INTO creator_runs VALUES (?,?)', (run['id'], json.dumps(run)))
            self.sessions.db.commit()

    def _get(self, rid):
        if not isinstance(rid, str) or not re.fullmatch('[a-f0-9]{32}', rid):
            raise SessionError('Unknown Creator run.')
        with self.sessions.lock:
            row = self.sessions.db.execute('SELECT data FROM creator_runs WHERE id=?', (rid,)).fetchone()
        if row is None:
            raise SessionError('Unknown Creator run.')
        run = json.loads(row[0])
        run.setdefault('budgets', dict(DEFAULT_BUDGETS))
        if run.get('session_id'):
            session = self.sessions.get(run['session_id'])
            if run['status'] in ('scanning', 'generating', 'applying', 'rolling_back') and session['status'] not in ACTIVE:
                value = load_file(self.root / rid, run['result_file'], required=False)
                try:
                    outcome = json.loads(value['body']) if value else {}
                except (ValueError, UnicodeError):
                    outcome = {}
                if not isinstance(outcome, dict) or outcome.get('status') not in ('review','preview','needs_input','complete','rolled_back','failed'):
                    outcome = {}
                run['status'] = outcome.get('status', 'failed') if session['status'] == 'completed' else session['status']
                run['message'] = outcome.get('message', 'Inspect the session events and retry this phase.')
                run['revision'] += 1
                self._save(run)
        run['recovery_available'] = False
        if run['status'] not in ('applying','rolling_back'):
            journal = load_file(self.root/rid, 'journal/journal.json', required=False)
            if not journal and (self.root/rid/'journal').is_dir():
                run['recovery_available'] = True
            if journal:
                try: journal_status = json.loads(journal['body']).get('status')
                except (ValueError, AttributeError): journal_status = 'damaged'
                run['recovery_available'] = journal_status not in ('verified','rolled-back')
        return run

    def list(self, project_id):
        self.sessions.project(project_id)
        with self.lock, self.sessions.lock:
            ids = [row[0] for row in self.sessions.db.execute('SELECT id FROM creator_runs ORDER BY rowid DESC LIMIT 200')]
            runs = [self._get(rid) for rid in ids]
        return {'project_id': project_id, 'available': bool(shutil.which('bwrap')), 'runs': [run for run in runs if run['project_id'] == project_id]}

    def get(self, rid):
        with self.lock:
            run = self._get(rid)
            work = self.root / rid / 'agent'
            reports = []
            for name in REPORTS:
                try:
                    value = load_file(work / TASK, name, required=False)
                except SessionError:
                    if run['status'] in ('scanning','generating'): continue
                    reports.append({'name':name,'text':'Report is missing, changing, or unsafe to read.','truncated':False})
                    continue
                if value:
                    reports.append({'name': name, 'text': value['body'][:192000].decode('utf-8', errors='replace'),
                                    'truncated': value['bytes'] > 192000, 'hash': value['hash']})
            with self.sessions.lock:
                rows = self.sessions.db.execute('SELECT id,data FROM events WHERE session_id=? ORDER BY id DESC LIMIT 100', (run.get('session_id'),)).fetchall()
            result = {'run':run,'reports':reports,'session':self.sessions.get(run['session_id']) if run.get('session_id') else None,'events':[{**json.loads(row['data']),'id':row['id']} for row in reversed(rows)]}
            if run['status'] == 'preview':
                result['preview'] = self.preview(rid)
            return result

    def start(self, data):
        allowed = {'project_id', 'provider', 'model', 'thinking_effort', 'agents_enabled', 'agent_count',
                   'tools', 'operation', 'workspace', 'worktree_branch', 'goal', 'budgets'}
        if not isinstance(data, dict) or set(data) - allowed:
            raise SessionError('Invalid Creator options.')
        project = self.sessions.project(data.get('project_id'))
        provider = self.sessions.providers.get(data.get('provider')) if isinstance(data.get('provider'), str) else None
        if not provider or not provider['available']:
            raise SessionError('Choose an available native provider.')
        model, effort = model_settings(provider['id'], data)
        budgets = validate_budgets(data.get('budgets', {}), provider['id'])
        tools = data.get('tools')
        if not isinstance(tools, list) or not tools or any(not isinstance(x,str) or x not in ('claude','codex','cursor') for x in tools) or len(set(tools)) != len(tools):
            raise SessionError('Choose distinct target tools.')
        operation = data.get('operation', 'generate')
        if operation not in ('generate', 'update'):
            raise SessionError('Choose generation or update.')
        enabled, count = data.get('agents_enabled', False), data.get('agent_count', DEFAULT_AGENT_COUNT)
        if type(enabled) is not bool or type(count) is not int or not 1 <= count <= MAX_AGENTS:
            raise SessionError('Invalid additional-agent settings.')
        goal = data.get('goal', '')
        if not isinstance(goal, str) or len(goal.encode()) > 8000 or '\x00' in goal:
            raise SessionError('Use a goal of at most 8000 UTF-8 bytes.')
        source = Path(project['path'])
        for boundary in (ROOT, self.sessions.state_dir):
            if source == boundary or source in boundary.parents or boundary in source.parents:
                raise SessionError('Choose a target outside the Harness source and runner state.')
        if not (source/'composer.json').is_file() and not any(source.glob('*.php')) and not any(source.glob('src/*.php')) and not any(source.glob('app/*.php')):
            raise SessionError('No PHP project entry point detected. Use Infrastructure-Creator stack adaptation separately for a non-PHP target.')
        # Environment prerequisite last: input and target problems are reported first.
        if not shutil.which('bwrap'):
            raise SessionError('Creator requires bubblewrap on Linux.')
        rid = uuid.uuid4().hex
        with self.lock, self.sessions.lock:
            if self.sessions.jobs.full() or self.sessions.stopping.is_set():
                raise SessionError('The run queue is full or the server is stopping.')
            target, workspace, branch, common = self.sessions._new_workspace(project, data, rid)
            fd = _root_fd(target)
            try:
                identity = list(_identity(os.fstat(fd)))
                manifest = _read(fd, '.infra-manifest.json')
                if operation == 'update' and manifest is None:
                    raise SessionError('Update requires the existing .infra-manifest.json; no ownership is inferred.')
            finally:
                os.close(fd)
            directory = self.root / rid; directory.mkdir(mode=0o700)
            agent = directory/'agent'; agent.mkdir()
            # Generator content is a working copy; independent gates always use the canonical scripts.
            for name in ('AGENTS.md','VERSION','README.md','.agents','.claude','.cursor','.codex'):
                path = GENERATOR/name
                if path.is_dir():
                    if any(p.is_symlink() for p in path.rglob('*')):
                        raise SessionError('Generator sources contain links; restore them before running.')
                    shutil.copytree(path, agent/name, ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
                else:
                    shutil.copy2(path, agent/name)
            (agent/TASK).mkdir(parents=True)
            run = {'id':rid,'project_id':project['id'],'target':str(target),'identity':identity,
                   'workspace':workspace,'branch':branch,'operation':operation,'tools':tools,'provider':provider['id'],
                   'model':model,'thinking_effort':effort,'agents_enabled':enabled,'agent_count':count,
                   'budgets':budgets,'goal':goal,'created_at':now(),'revision':1,'status':'new','answers':'', 'phase':'scan'}
            return self._launch(run, 'scan')

    def _launch(self, run, phase):
        nonce = uuid.uuid4().hex
        run.pop('message', None)
        journal = self.root/run['id']/'journal'
        if phase == 'apply' and journal.exists():
            if json.loads(load_file(journal,'journal.json')['body']).get('status') != 'rolled-back':
                raise SessionError('Recover the previous publication before applying again.')
            journal.rename(journal.with_name('journal-'+nonce))
        run.update(phase=phase, result_file='result-'+nonce+'.json', status={'scan':'scanning','generate':'generating','apply':'applying','rollback':'rolling_back'}[phase])
        request = {**run, 'directory': str(self.root/run['id']), 'executable': self.sessions.providers[run['provider']]['executable']}
        write_json(self.root/run['id']/('request-'+nonce+'.json'), request)
        session = self.sessions.create({'project_id':run['project_id'],'provider':run['provider'],
            'prompt':f"Creator {phase}: {Path(run['target']).name}", 'mode':'edit','workflow':'native',
            'model':run['model'],'thinking_effort':run['thinking_effort'],
            'agents_enabled':run['agents_enabled'],'agent_count':run['agent_count'],
            'budgets':run.get('budgets', {}) if phase in ('scan','generate') else {}},
            _creator={'run_id':run['id'],'nonce':nonce,'phase':phase})
        run['session_id'] = session['id']; self._save(run)
        return self.get(run['id'])

    def act(self, rid, data):
        if not isinstance(data, dict) or set(data) - {'action','revision','answers','preview_id','replace','budgets'}:
            raise SessionError('Invalid Creator action.')
        with self.lock:
            run = self._get(rid)
            if type(data.get('revision')) is not int or data['revision'] != run['revision']:
                raise SessionError('This Creator view is stale. Refresh before continuing.')
            action = data.get('action')
            if action == 'cancel' and run['status'] in ('scanning','generating','applying','rolling_back'):
                if run['status'] in ('applying','rolling_back'):
                    raise SessionError('Publication is completing its verification/rollback. Wait for its result.')
                self.sessions.cancel(run['session_id']); return self.get(rid)
            if run['status'] in ('scanning','generating','applying','rolling_back'):
                raise SessionError('Wait for the current phase to finish.')
            if action == 'rollback' and run['recovery_available']:
                run['revision'] += 1
                return self._launch(run, 'rollback')
            if run['recovery_available']:
                raise SessionError('Recover the interrupted publication before continuing.')
            if action == 'budgets':
                if set(data) != {'action','revision','budgets'}:
                    raise SessionError('Provide only budgets and the current revision.')
                run['budgets'] = validate_budgets(data['budgets'], run['provider'])
                run['revision'] += 1
                self._save(run)
                return self.get(rid)
            if 'budgets' in data:
                raise SessionError('Save budgets before starting a phase.')
            if action == 'rescan' and run['status'] != 'complete':
                run.pop('approved', None)
                run['answers'] = data.get('answers', '')
                if not isinstance(run['answers'],str) or len(run['answers'].encode())>16000 or '\x00' in run['answers']:
                    raise SessionError('Invalid scan corrections.')
                run['revision'] += 1
                return self._launch(run, 'scan')
            if action == 'generate' and run['status'] == 'review':
                run['approved'] = self._profile_hashes(run)
                run['revision'] += 1
                return self._launch(run, 'generate')
            if action == 'revise' and run['status'] in ('review','needs_input','preview','failed','interrupted','cancelled','rolled_back'):
                answers = data.get('answers', '')
                if not isinstance(answers,str) or not answers.strip() or len(answers.encode()) > 16000 or '\x00' in answers:
                    raise SessionError('Enter corrections or answers (up to 16000 UTF-8 bytes).')
                run['answers'] = answers
                phase = 'scan' if run['phase'] == 'scan' else 'generate'
                if phase == 'generate' and not run.get('approved'):
                    phase = 'scan'
                run['revision'] += 1
                return self._launch(run, phase)
            if action == 'apply' and run['status'] == 'preview':
                preview = self.preview(rid)
                replacements = data.get('replace', [])
                if (data.get('preview_id') != preview['preview_id'] or not isinstance(replacements,list)
                        or any(not isinstance(x,str) for x in replacements) or len(replacements)!=len(set(replacements))
                        or set(replacements) != {f['path'] for f in preview['files'] if f['requires_decision']}):
                    raise SessionError('Review the current diff and explicitly approve each replacement, or request revisions to keep files.')
                if not preview['can_apply']:
                    raise SessionError('Publication is blocked by the reported conflicts.')
                write_json(self.root/rid/'approved-preview.json', preview)
                run['revision'] += 1
                return self._launch(run, 'apply')
            raise SessionError('This action is not available in the current Creator phase.')

    def _profile_hashes(self, run):
        return profile_hashes(self.root/run['id'])

    def preview(self, rid):
        return build_preview(self._get(rid), self.root/rid)


def profile_hashes(directory):
    return {name:load_file(directory/'agent'/TASK,name)['hash'] for name in REPORTS[:3]}


def build_preview(run, directory):
    rid = run['id']
    if run['status'] != 'preview':
        raise SessionError('The generated bundle has not passed verification.')
    task = directory/'agent'/TASK
    staging = task/'infra-generate-staging'; target = Path(run['target'])
    paths = read_plan(task, PLAN_NAMES[0])
    if '.infra-manifest.json' not in paths: paths.append('.infra-manifest.json')
    removals = read_plan(task, PLAN_NAMES[1], True); watched = read_plan(task, PLAN_NAMES[2], True)
    if set(paths)&set(removals) or set(watched)&(set(paths)|set(removals)):
        raise SessionError('Publication, removal and watch plans overlap.')
    if not all(allowed_output(p) for p in paths+removals):
        raise SessionError('Creator output cannot overwrite application source files.')
    old_manifest = load_file(target,'.infra-manifest.json',False)
    members = json.loads(old_manifest['body']).get('files',{}) if old_manifest else {}
    if not isinstance(members,dict): raise SessionError('Existing manifest files must be an object.')
    entries, before, payloads, dirs = [], {}, {}, {}
    fd = _root_fd(target); stage_fd = _root_fd(staging)
    try:
        if list(_identity(os.fstat(fd))) != run['identity']: raise SessionError('Target directory changed.')
        budget, diff_budget = 0, 256000
        for path in paths+removals+watched:
            old = _read(fd,path,dirs); before[path] = _metadata(old)
            new = _read(stage_fd,path,required=True) if path in paths else None
            if new:
                if new['mode'] & 0o7000: raise SessionError('Generated files cannot use special permission bits.')
                payloads[path] = _metadata(new)
            budget += (old['bytes'] if old else 0)+(new['bytes'] if new else 0)
            if budget > 64*1024*1024: raise SessionError('Creator preview exceeds 64 MiB.')
            if path in watched: continue
            diff,truncated = _diff(old['body'] if old else b'',new['body'] if new else b'',path)
            if len(diff.encode()) > diff_budget: diff=diff.encode()[:diff_budget].decode('utf-8',errors='ignore'); truncated=True
            diff_budget -= len(diff.encode())
            equal = bool(old and new and old['hash']==new['hash'] and old['mode']==new['mode'])
            owned = members.get(path,{})
            digest = owned.get('sha256') if isinstance(owned,dict) else owned
            decision = not equal and path!='.infra-manifest.json' and (old is not None or path in members) and (run['operation']!='update' or old is None or digest!=old['hash'])
            entries.append({'path':path,'action':'unchanged' if equal else 'remove' if new is None else 'replace' if old else 'add',
                'requires_decision':decision,'diff':diff,'diff_truncated':truncated,
                'before_bytes':old['bytes'] if old else 0,'after_bytes':new['bytes'] if new else 0})
    finally:
        os.close(fd); os.close(stage_fd)
    receipt = {'run_id':rid,'identity':run['identity'],'before':before,'payloads':payloads,'directories':dirs,
               'profile':profile_hashes(directory),'paths':paths,'removals':removals,'watched':watched}
    receipt['preview_id'] = hashlib.sha256(json.dumps(receipt,sort_keys=True).encode()).hexdigest()
    conflict = ''
    try: publication_helpers().verify_plan_ownership(target, staging, paths, removals)
    except ValueError as error: conflict = str(error)
    return {**receipt,'files':entries,'can_apply':not conflict,'conflict':conflict}


def publication_helpers():
    sys.path.insert(0,str(SCRIPTS))
    import publish_staging
    return publish_staging
