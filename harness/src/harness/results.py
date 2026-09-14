"""Workspace diffs and explicit checks, sharing the session queue and owner guard."""
from __future__ import annotations

import difflib
import hashlib
import json
import os
from pathlib import Path
import selectors
import shlex
import signal
import subprocess
import sys
import time
import uuid

from .sessions import ACTIVE, SessionError, git_details, now, read_context

OUTPUT_LIMIT = 512 * 1024


class Results:
    def __init__(self, sessions):
        self.sessions = sessions
        with sessions.lock:
            sessions.db.executescript('''
                CREATE TABLE IF NOT EXISTS launches (
                    id TEXT PRIMARY KEY, session_id TEXT NOT NULL, kind TEXT NOT NULL,
                    status TEXT NOT NULL, started_at TEXT NOT NULL, finished_at TEXT,
                    settings TEXT NOT NULL, usage TEXT);
                CREATE INDEX IF NOT EXISTS launches_session ON launches(session_id,started_at);
                CREATE TABLE IF NOT EXISTS checks (
                    id TEXT PRIMARY KEY, session_id TEXT NOT NULL, data TEXT NOT NULL);
            ''')
            # Old sessions do not contain enough information to reconstruct every turn.
            sessions.db.execute("UPDATE launches SET status='interrupted',finished_at=? WHERE status='running'", (now(),))
            rows=sessions.db.execute('SELECT id,data FROM checks').fetchall()
            for row in rows:
                data=json.loads(row['data'])
                if data['status'] in ACTIVE:
                    data.update(status='interrupted',finished_at=now())
                    self.save_check(data)
                    sessions.db.execute("UPDATE sessions SET status=? WHERE id=? AND status='interrupted'", (data['previous_status'],data['session_id']))
            sessions.db.commit()

    def baseline(self, session):
        if session.get('result_base') is not None: return
        try:
            value=git_details(self.sessions._workspace(session))
            value={key:value.get(key) for key in ('head','branch','dirty','is_git')}
        except SessionError:
            value={'is_git':False}
        with self.sessions.lock:
            self.sessions.db.execute('UPDATE sessions SET result_base=? WHERE id=? AND result_base IS NULL',
                                     (json.dumps(value),session['id']))
            self.sessions.db.commit()

    def start_launch(self, sid, generation):
        session=self.sessions.get(sid)
        settings={k:session[k] for k in ('provider','model','thinking_effort','agents_enabled','agent_count','budgets','agent_budget_plan','sdd','mode','model_routing','clash')}
        kind='fleet' if session['fleet'] else 'clash' if session['clash'] else 'creator-'+session['creator']['phase'] if session['creator'] else 'native'
        with self.sessions.lock:
            self.sessions.db.execute('INSERT INTO launches VALUES (?,?,?,?,?,?,?,NULL)',
                (generation,sid,kind,'running',now(),None,json.dumps(settings)))
            self.sessions.db.commit()

    def finish_launch(self, generation, status):
        with self.sessions.lock:
            self.sessions.db.execute('UPDATE launches SET status=?,finished_at=? WHERE id=? AND status=?',
                (status,now(),generation,'running'))
            self.sessions.db.commit()

    def history(self, sid):
        session=self.sessions.get(sid); ids=[sid]
        with self.sessions.lock:
            if session['creator']:
                ids=[row['id'] for row in self.sessions.db.execute('SELECT id,creator FROM sessions WHERE creator IS NOT NULL')
                     if json.loads(row['creator'])['run_id']==session['creator']['run_id']]
            placeholders=','.join('?' for _ in ids)
            rows=self.sessions.db.execute(f'SELECT * FROM launches WHERE session_id IN ({placeholders}) ORDER BY started_at,rowid',ids).fetchall()
            records=[{**dict(row),'settings':json.loads(row['settings']),'usage':json.loads(row['usage']) if row['usage'] else None} for row in rows]
            checks=[json.loads(row[0]) for row in self.sessions.db.execute('SELECT data FROM checks WHERE session_id=? ORDER BY rowid DESC LIMIT 50',(sid,))]
        totals={}
        for key in ('tokens','cost_usd','seconds'):
            known=[(r['usage'] or {}).get(key) for r in records]
            totals[key]={'reported':sum(x for x in known if x is not None), 'unknown_launches':sum(x is None for x in known)}
        return {'launches':records,'totals':totals,'checks':checks,
                'notice':'History begins with launches recorded by this server version. Older turns are not reconstructed; unknown usage is not zero.'}

    def capture(self, command, root, timeout=10, sid=None, update=None, env_extra=None, cleanup=False):
        """Bound output during reading; the same guard owns Git helpers and checks."""
        store=self.sessions
        if not cleanup and (store.stopping.is_set() or sid in store.cancelled):
            return {'output':'','exit_code':None,'reason':'interrupted' if store.stopping.is_set() else 'cancelled','seconds':0}
        read_fd,write_fd=os.pipe(); process=None; selector=selectors.DefaultSelector()
        output=bytearray(); reason=None; started=time.monotonic()
        try:
            env={k:v for k,v in os.environ.items() if not k.startswith('GIT_')}
            env.update(GIT_TERMINAL_PROMPT='0',PYTHONDONTWRITEBYTECODE='1')
            if env_extra: env.update(env_extra)
            process=subprocess.Popen([sys.executable,str(Path(__file__).with_name('process_guard.py')),str(read_fd),'--',*command],
                cwd=root,env=env,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,
                start_new_session=True,pass_fds=(read_fd,store.runner_lock))
            os.close(read_fd); read_fd=None
            if sid:
                with store.lock: store.active[sid]=process
            selector.register(process.stdout,selectors.EVENT_READ)
            while selector.get_map():
                if not cleanup and (store.stopping.is_set() or sid in store.cancelled):
                    reason='interrupted' if store.stopping.is_set() else 'cancelled'; break
                if time.monotonic()-started > timeout: reason='timed_out'; break
                for key,_ in selector.select(.1):
                    chunk=os.read(key.fileobj.fileno(),65536)
                    if not chunk: selector.unregister(key.fileobj); continue
                    output.extend(chunk[:OUTPUT_LIMIT-len(output)])
                    if update: update(output.decode('utf-8',errors='replace'))
                    if len(output)>=OUTPUT_LIMIT: reason='output_limit'; break
                if reason: break
            if not cleanup and (store.stopping.is_set() or sid in store.cancelled):
                reason='interrupted' if store.stopping.is_set() else 'cancelled'
            if reason is None:
                try: process.wait(timeout=max(.1,timeout-(time.monotonic()-started)))
                except subprocess.TimeoutExpired: reason='timed_out'
        finally:
            if read_fd is not None: os.close(read_fd)
            os.close(write_fd)
            if process:
                store._signal(process,signal.SIGKILL)
                process.wait(timeout=3); process.stdout.close()
            selector.close()
            if sid:
                with store.lock: store.active.pop(sid,None)
        return {'output':output.decode('utf-8',errors='replace'),'exit_code':process.returncode,
                'reason':reason,'seconds':round(time.monotonic()-started,3)}

    def snapshot(self, sid):
        session=self.sessions.get(sid); root=self.sessions._workspace(session)
        info=git_details(root,include_status=False)
        if not info['is_git'] or not info['head']:
            return {'available':False,'id':None,'complete':False,'message':'A Git repository with a commit is required for a diff. Checks can still run in this workspace.'}
        base=session.get('result_base') or {}; head=base.get('head') or info['head']
        command=['git','--no-optional-locks','-c','core.hooksPath=/dev/null','-c','core.fsmonitor=false','-c','color.ui=false']
        def git(*args):
            value=self.capture(command+list(args),root)
            if value['exit_code'] and not value['reason']: raise SessionError('Could not read the workspace diff; its original commit may be unavailable.')
            return value
        patch=git('diff','--no-ext-diff','--no-textconv','--relative','--no-renames',head,'--','.')
        names=git('diff','--no-ext-diff','--no-textconv','--relative','--no-renames','--name-status','-z',head,'--','.')
        unknown=git('ls-files','--others','--exclude-standard','-z','--','.')
        parts=names['output'].split('\0'); files=[]
        for i in range(0,len(parts)-1,2):
            if parts[i]: files.append({'status':parts[i],'path':parts[i+1]})
        complete=not any(v['reason'] for v in (patch,names,unknown)) and 'Binary files ' not in patch['output'] and 'Subproject commit ' not in patch['output']
        text=patch['output']; untracked=unknown['output'].split('\0')
        for path in filter(None,untracked):
            files.append({'status':'?','path':path})
            if len(files)>500 or len(text.encode())>=OUTPUT_LIMIT: complete=False; continue
            value=read_context(root,path,65537)
            if value is None or value[0]>65536 or '\0' in value[1]:
                text+='\nUntracked binary, large or linked file (content omitted): '+path+'\n'; complete=False; continue
            text+=''.join(difflib.unified_diff([],value[1].splitlines(keepends=True),fromfile='/dev/null',tofile=path))
        if len(text.encode())>OUTPUT_LIMIT: text=text.encode()[:OUTPUT_LIMIT].decode(errors='replace'); complete=False
        digest=hashlib.sha256(json.dumps([str(root),head,info['head'],text,files],ensure_ascii=True).encode()).hexdigest()
        return {'available':True,'id':digest,'base':head,'head':info['head'],'branch':info['branch'],
                'files':files[:500],'diff':text,'complete':complete,'preexisting_changes':base.get('dirty'),
                'baseline_recorded':bool(base.get('head')),'message':'Current workspace versus the launch baseline. Includes pre-existing and external edits; this is not proof of agent authorship.'}

    def get(self, sid, include_diff=True):
        session=self.sessions.get(sid)
        result={'session_id':sid,**self.history(sid),'workspace':session['project_path']}
        if include_diff:
            result['snapshot']=self.snapshot(sid) if session['status'] not in ACTIVE else None
        return result

    def save_check(self, check):
        with self.sessions.lock:
            self.sessions.db.execute('INSERT OR REPLACE INTO checks VALUES (?,?,?)',(check['id'],check['session_id'],json.dumps(check)))
            self.sessions.db.commit()

    @staticmethod
    def check_command(data):
        if not isinstance(data,dict) or set(data)!={'command','timeout'}:
            raise SessionError('Provide a check command and timeout.')
        command=data['command']; timeout=data['timeout']
        if not isinstance(command,str) or not command.strip() or len(command.encode())>4000 or any(ord(c)<32 for c in command):
            raise SessionError('Enter one command of at most 4000 bytes.')
        try: argv=shlex.split(command)
        except ValueError: raise SessionError('Check command quoting.') from None
        if not argv or len(argv)>100 or any(arg in ('|','||','&&',';','>','>>','<') for arg in argv):
            raise SessionError('Run one command at a time; shell operators are not expanded.')
        if type(timeout) is not int or not 1<=timeout<=3600: raise SessionError('Use a timeout of 1–3600 seconds.')
        return argv

    def queue_check(self, sid, data, metadata):
        argv=self.check_command(data); store=self.sessions
        with store.lock:
            session=store.get(sid)
            if session['status'] in ACTIVE or store.stopping.is_set() or store.jobs.full():
                raise SessionError('Wait for the current launch or queue to finish.')
            check={'id':str(uuid.uuid4()),'session_id':sid,**data,'argv':argv,**metadata,
                   'status':'queued','output':'','exit_code':None,'created_at':now(),
                   'started_at':None,'finished_at':None,'seconds':None,'previous_status':session['status']}
            self.save_check(check)
            store.cancelled.discard(sid); generation=store.generations[sid]=uuid.uuid4().hex
            store._status(sid,'queued'); store.jobs.put_nowait((sid,{'check_id':check['id']},generation))
        return check

    def start_check(self, sid, data):
        if not isinstance(data,dict) or set(data)!={'command','timeout','snapshot_id'}:
            raise SessionError('Provide a command, timeout and the reviewed snapshot.')
        command={k:data[k] for k in ('command','timeout')}
        self.check_command(command)
        store=self.sessions
        with store.lock:
            session=store.get(sid)
            if session['creator']: raise SessionError('Creator runs its canonical checks at its own checkpoints.')
            if session['status'] in ACTIVE or store.stopping.is_set() or store.jobs.full():
                raise SessionError('Wait for the current launch or queue to finish.')
            snapshot=self.snapshot(sid)
            if data['snapshot_id']!=snapshot['id']: raise SessionError('Workspace changed. Refresh the diff before running a check.')
            self.queue_check(sid,command,{'snapshot_id':snapshot['id'],'snapshot_complete':snapshot['complete']})
        return store.get(sid)

    def run_check(self, sid, cid, generation):
        store=self.sessions
        with store.lock:
            check=json.loads(store.db.execute('SELECT data FROM checks WHERE id=? AND session_id=?',(cid,sid)).fetchone()[0])
            if store.stopping.is_set() or sid in store.cancelled or store.generations.get(sid)!=generation:
                check.update(status='cancelled',finished_at=now()); self.save_check(check)
                if store.generations.get(sid)==generation: store._status(sid,check['previous_status'])
                return
            store._status(sid,'running'); check.update(status='running',started_at=now()); self.save_check(check)
        try:
            def update(output):
                check['output']=output; self.save_check(check)
            if check.get('kind')=='target':
                result=store.delivery.run_check(check,update)
            else:
                snapshot=self.snapshot(sid)
                if snapshot['id']!=check['snapshot_id']: raise SessionError('Workspace changed while queued. Refresh and run the check again.')
                result=self.capture(check['argv'],store._workspace(store.get(sid)),check['timeout'],sid,update)
                after=self.snapshot(sid)
                check['workspace_changed']=after['id']!=check['snapshot_id']
            check.update(result); check['status']=result['reason'] or ('passed' if result['exit_code']==0 else 'failed')
        except Exception as error:
            check.update(status='failed',output=check['output']+'\n'+(str(error) if isinstance(error,SessionError) else 'Could not start the command. Check its executable and workspace.'))
        finally:
            check['finished_at']=now(); self.save_check(check)
            with store.lock:
                if store.generations.get(sid)==generation:
                    store._status(sid,check['previous_status'])
                    store._event(sid,{'kind':'status','text':f"Check {check['status']}: {check['command']}. This is separate from the agent result."})
