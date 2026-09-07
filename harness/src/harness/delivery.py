"""Explicit local Git delivery: immutable previews, selected files, no remotes."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import stat
import tempfile
import time
import uuid

from .sessions import SessionError, git_details
from .setup import _root_fd, _read, _metadata, _safe_path

TTL = 900
MAX_FILES = 500


def digest(value):
    return hashlib.sha256(value).hexdigest()


class Delivery:
    def __init__(self, sessions):
        self.sessions = sessions
        self.previews = {}
        self.folder = sessions.state_dir / 'delivery'
        self.folder.mkdir(exist_ok=True, mode=0o700)
        if self.folder.is_symlink(): raise SessionError('Delivery storage must not be a link.')
        self.blocked = {}
        for path in self.folder.glob('transaction-*.json'):
            try: self._recover(path)
            except Exception:
                try: self.blocked[json.loads(path.read_text())['sid']] = 'An interrupted Git commit needs inspection before another delivery.'
                except Exception: raise SessionError('An interrupted delivery journal cannot be read.') from None

    def _git(self, root, *args, env=None, allowed=(0,), cleanup=False):
        result = self.sessions.results.capture(
            ['git', '--no-optional-locks', '--literal-pathspecs', '-c', 'core.hooksPath=/dev/null',
             '-c', 'core.fsmonitor=false', '-c', 'commit.gpgSign=false', '-c', 'color.ui=false',
             '-c', 'merge.autoStash=false', '-C', str(root), *args], root, timeout=30,
            env_extra={'GIT_NO_REPLACE_OBJECTS':'1', **(env or {})}, cleanup=cleanup)
        if result['reason']: raise SessionError('Git stopped or exceeded the preview limit. Refresh and inspect the workspace.')
        if result['exit_code'] not in allowed:
            raise SessionError('Git could not complete the operation: ' + result['output'][:2000])
        return result

    def _text(self, root, *args, **kwargs):
        return self._git(root, *args, **kwargs)['output'].strip()

    def _idle(self):
        if self.sessions.stopping.is_set() or self.sessions.db.execute("SELECT 1 FROM sessions WHERE status IN ('queued','running') LIMIT 1").fetchone():
            raise SessionError('Wait for active or queued sessions and checks before Git delivery.')

    def _source(self, sid):
        session = self.sessions.get(sid)
        if session['creator'] or session['workspace'] != 'worktree':
            raise SessionError('Delivery is available for agent sessions in a Git worktree.')
        if not (session['result_base'] or {}).get('head'):
            raise SessionError('This older session has no recorded baseline. Send a new turn before using worktree delivery.')
        if sid in self.blocked: raise SessionError(self.blocked[sid])
        root = self.sessions._workspace(session)
        info = git_details(root, include_status=False)
        ref=self._text(root,'symbolic-ref','HEAD',allowed=(0,1))
        info['branch']=ref[len('refs/heads/'):] if ref.startswith('refs/heads/') else None
        if not info['head'] or not info['branch'] or info['branch'] != session['branch']:
            raise SessionError('The session branch changed. Restore its original worktree branch first.')
        if self._text(root, 'config', '--bool', 'core.sparseCheckout', allowed=(0,1)) == 'true':
            raise SessionError('Delivery does not support sparse worktrees.')
        self._settled(root)
        return session, root, info

    def _git_path(self, root, name):
        value = Path(self._text(root, 'rev-parse', '--git-path', name))
        return value if value.is_absolute() else root / value

    def _settled(self, root):
        for name in ('MERGE_HEAD','CHERRY_PICK_HEAD','REVERT_HEAD','rebase-merge','rebase-apply','sequencer'):
            if self._git_path(root, name).exists():
                raise SessionError('Finish the existing merge, rebase or cherry-pick before delivery.')
        if self._text(root, 'ls-files', '--unmerged'):
            raise SessionError('Resolve the existing index conflicts before delivery.')

    def _index(self, root):
        path = self._git_path(root, 'index')
        try:
            fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
            try:
                meta = os.fstat(fd)
                if not stat.S_ISREG(meta.st_mode) or meta.st_nlink != 1 or meta.st_size > 64*1024*1024:
                    raise SessionError('The Git index is unsafe or exceeds the delivery limit.')
                with os.fdopen(os.dup(fd), 'rb') as stream: body = stream.read(64*1024*1024+1)
                if len(body) > 64*1024*1024: raise SessionError('The Git index exceeds the delivery limit.')
            finally: os.close(fd)
        except FileNotFoundError: body = b''
        except OSError: raise SessionError('The Git index is unavailable or linked.') from None
        return path, body

    def _names(self, root, before, after=None):
        args = ['diff','--no-ext-diff','--no-textconv','--relative','--no-renames','--name-status','-z',before]
        if after: args.append(after)
        parts = self._git(root, *args, '--', '.')['output'].split('\0')
        return [{'status':parts[i],'path':parts[i+1]} for i in range(0,len(parts)-1,2) if parts[i]]

    def _pending(self, root, info):
        files = self._names(root, info['head'])
        by_path={item['path']:item for item in files}
        for name in self._git(root,'ls-files','--others','--exclude-standard','-z','--','.')['output'].split('\0'):
            if name: by_path[name]={'status':'M' if name in by_path else '?','path':name}
        files=list(by_path.values())
        if len(files)>MAX_FILES: raise SessionError('Delivery supports up to 500 pending files. Narrow the worktree first.')
        fingerprints=[]; size=0; fd=_root_fd(root)
        try:
            for item in files:
                if not _safe_path(item['path']): raise SessionError('A pending file has an unsupported path.')
                value=_read(fd,item['path']); fingerprints.append((item,_metadata(value)))
                size += value['bytes'] if value else 0
                if size>64*1024*1024: raise SessionError('Pending files exceed the 64 MiB delivery limit.')
        finally: os.close(fd)
        _,index=self._index(root); meta=root.stat()
        key=digest(json.dumps([info['head'],info['branch'],str(root),meta.st_dev,meta.st_ino,digest(index),fingerprints],sort_keys=True).encode())
        return {'snapshot_id':key,'files':files}

    def _branches(self, root):
        lines=self._git(root,'for-each-ref','--format=%(refname:strip=2)%00%(objectname)','refs/heads/')['output'].splitlines()
        return [dict(zip(('name','head'),line.split('\0'))) for line in lines]

    def _commits(self, session, root, info):
        base=(session['result_base'] or {}).get('head')
        if not base: return []
        if self._git(root,'merge-base','--is-ancestor',base,info['head'],allowed=(0,1))['exit_code']: return []
        lines=self._git(root,'log','--max-count=100','--no-merges','--format=%H%x00%s',base+'..'+info['head'])['output'].splitlines()
        return [dict(zip(('sha','subject'),line.split('\0',1))) for line in lines]

    def get(self, sid):
        # ponytail: the existing global session lock serializes local Git operations.
        with self.sessions.lock:
            self.sessions.get(sid)
            try:
                self._idle(); session,root,info=self._source(sid); pending=self._pending(root,info)
            except SessionError as error: return {'available':False,'reason':str(error),'preview':self.current_preview(sid)}
            return {'available':True,'preview':self.current_preview(sid),'workspace':str(root),'branch':info['branch'],'head':info['head'],**pending,
                    'commits':self._commits(session,root,info),
                    'branches':[b for b in self._branches(root) if b['name']!=info['branch']],
                    'notice':'Pending files are compared with current HEAD. Whole-file selection includes staged and unstaged content; unselected index entries and files are preserved. Local Git only.'}

    def _checks(self, sid):
        snapshot=self.sessions.results.snapshot(sid)
        return [{k:check.get(k) for k in ('status','command','exit_code')} | {'current':bool(snapshot['complete'] and check['snapshot_complete'] and snapshot['id']==check['snapshot_id'] and not check.get('workspace_changed'))}
                for check in self.sessions.results.history(sid)['checks'][:10] if check.get('kind')!='target']

    def _remember(self, sid, preview):
        self.previews={key:item for key,item in self.previews.items() if item['sid']!=sid and time.monotonic()-item['created']<TTL}
        while len(self.previews)>=4: self.previews.pop(next(iter(self.previews)))
        key=uuid.uuid4().hex; self.previews[key]={'sid':sid,'created':time.monotonic(),**preview}
        return {k:v for k,v in preview.items() if not k.startswith('_')} | {'preview_id':key}

    def _take(self, sid, data, kind):
        if not isinstance(data,dict) or set(data)!={'preview_id'} or not isinstance(data['preview_id'],str):
            raise SessionError('Use the reviewed preview ID.')
        preview=self.previews.get(data['preview_id'])
        if not preview or preview['sid']!=sid or preview['kind']!=kind or time.monotonic()-preview['created']>=TTL:
            raise SessionError('The preview expired or was already used. Preview again.')
        if not preview['can_apply']: raise SessionError('Resolve the preview conflicts before applying.')
        return preview

    def _patch(self, root, before, after):
        return self._git(root,'diff','--no-ext-diff','--no-textconv','--no-renames','--full-index',before,after,'--','.')['output']

    def preview_commit(self, sid, data):
        if not isinstance(data,dict) or set(data)!={'snapshot_id','paths','message'}:
            raise SessionError('Select files, a current snapshot and a commit message.')
        paths=data['paths']; message=data['message']
        if not isinstance(paths,list) or not 1<=len(paths)<=MAX_FILES or any(not isinstance(p,str) or not _safe_path(p) for p in paths) or len(set(paths))!=len(paths):
            raise SessionError('Select distinct supported file paths.')
        if not isinstance(message,str) or not message.strip() or len(message.encode())>4000 or any(ord(c)<32 and c not in '\n\t' for c in message):
            raise SessionError('Enter a commit message of 1–4000 UTF-8 bytes.')
        with self.sessions.lock:
            self._idle(); session,root,info=self._source(sid); pending=self._pending(root,info)
            if data['snapshot_id']!=pending['snapshot_id']: raise SessionError('Workspace changed. Refresh pending files before committing.')
            if not set(paths)<={f['path'] for f in pending['files']}: raise SessionError('Select only current pending files.')
            with tempfile.TemporaryDirectory(dir=self.folder,prefix='index-') as folder:
                env={'GIT_INDEX_FILE':str(Path(folder)/'index')}
                self._git(root,'read-tree',info['head'],env=env)
                self._git(root,'add','--',*paths,env=env)
                tree=self._text(root,'write-tree',env=env)
            if self._pending(root,git_details(root,include_status=False))['snapshot_id']!=pending['snapshot_id']:
                raise SessionError('Workspace changed while preparing the commit. Preview again.')
            if tree==self._text(root,'rev-parse',info['head']+'^{tree}'): raise SessionError('Selected files have no changes to commit.')
            return self._remember(sid,{'kind':'commit','can_apply':True,'source_head':info['head'],
                'message':message.strip(),'files':self._names(root,info['head'],tree),'diff':self._patch(root,info['head'],tree),
                'checks':self._checks(sid),'notice':'Only the reviewed file contents will be committed. Unselected changes stay in the worktree. Git hooks and signing are disabled; use the independent checks above.',
                '_snapshot':pending['snapshot_id'],'_tree':tree,'_paths':paths,'_branch':info['branch']})

    def _recover(self, journal):
        value=json.loads(journal.read_text()); session=self.sessions.get(value['sid'])
        root=self.sessions._workspace(session); index,body=self._index(root); lock=Path(str(index)+'.lock')
        if str(index)!=value['index']: raise SessionError('Git index location changed during delivery.')
        head=self._text(root,'rev-parse','refs/heads/'+value['branch'])
        if not lock.exists():
            if head==value['commit'] and digest(body)==value['after']:
                journal.unlink(); return
            raise SessionError('The interrupted index lock is missing; inspect Git before continuing.')
        meta=lock.lstat()
        if lock.is_symlink() or (meta.st_dev,meta.st_ino)!=tuple(value['identity']) or digest(lock.read_bytes())!=value['after'] or digest(body)!=value['before']:
            raise SessionError('The Git index changed during recovery; inspect it before continuing.')
        if head==value['commit']:
            os.replace(lock,index)
            self.sessions._event(value['sid'],{'kind':'status','text':'Recovered the index for delivered commit '+head+'.'})
        elif head==value['old']: lock.unlink()
        else: raise SessionError('The branch moved during recovery; inspect the interrupted commit.')
        journal.unlink()

    def commit(self, sid, data):
        with self.sessions.lock:
            self._idle(); preview=self._take(sid,data,'commit'); self.previews.pop(data['preview_id']); session,root,info=self._source(sid)
            if self._pending(root,info)['snapshot_id']!=preview['_snapshot']: raise SessionError('Workspace changed. Preview the commit again.')
            index,original=self._index(root); lock=Path(str(index)+'.lock'); fd=None; journal=None
            try:
                try: fd=os.open(lock,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
                except FileExistsError: raise SessionError('Git is busy: index.lock already exists.') from None
                if self._pending(root,git_details(root,include_status=False))['snapshot_id']!=preview['_snapshot']:
                    raise SessionError('Workspace changed. Preview the commit again.')
                with tempfile.TemporaryDirectory(dir=self.folder,prefix='commit-') as folder:
                    folder=Path(folder); msg=folder/'message';msg.write_text(preview['message']+'\n')
                    commit=self._text(root,'commit-tree',preview['_tree'],'-p',info['head'],'-F',str(msg))
                    temp_index=folder/'index'
                    if original: temp_index.write_bytes(original)
                    env={'GIT_INDEX_FILE':str(temp_index)}
                    if not original: self._git(root,'read-tree','--empty',env=env)
                    self._git(root,'reset','--quiet',commit,'--',*preview['_paths'],env=env)
                    updated=temp_index.read_bytes()
                with os.fdopen(os.dup(fd),'wb') as stream: stream.write(updated);stream.flush();os.fsync(stream.fileno())
                meta=os.fstat(fd)
                value={'sid':sid,'index':str(index),'branch':info['branch'],'old':info['head'],'commit':commit,
                       'before':digest(original),'after':digest(updated),'identity':[meta.st_dev,meta.st_ino]}
                journal=self.folder/('transaction-'+uuid.uuid4().hex+'.json')
                with journal.open('x') as stream: json.dump(value,stream);stream.flush();os.fsync(stream.fileno())
                if self._text(root,'symbolic-ref','HEAD')!='refs/heads/'+info['branch']: raise SessionError('The source branch changed.')
                self._git(root,'update-ref','-m','Harness: '+preview['message'].splitlines()[0],
                          'refs/heads/'+info['branch'],commit,info['head'])
                os.close(fd);fd=None
                self._recover(journal);journal=None
                self.sessions._event(sid,{'kind':'status','text':'Committed selected files: '+commit+'.'})
                return {'ok':True,'commit':commit,'detail':'Selected files committed; unselected edits and staging are preserved.'}
            finally:
                if fd is not None: os.close(fd)
                if journal and journal.exists():
                    try: self._recover(journal)
                    except Exception: self.blocked[sid]='An interrupted Git commit needs inspection before another delivery.'
                elif fd is not None: lock.unlink(missing_ok=True)

    def _target(self, root, branch):
        if not isinstance(branch,str) or branch not in {b['name'] for b in self._branches(root)}:
            raise SessionError('Select an existing local target branch.')
        head=self._text(root,'rev-parse','refs/heads/'+branch)
        fields=self._git(root,'worktree','list','--porcelain','-z')['output'].split('\0'); entry={}; matches=[]
        for field in fields:
            if not field:
                if entry.get('branch')=='refs/heads/'+branch: matches.append(entry)
                entry={}
            else:
                key,_,value=field.partition(' ');entry[key]=value
        if len(matches)>1: raise SessionError('The target branch is checked out more than once.')
        target=matches[0].get('worktree') if matches else None
        if target:
            path=Path(target); info=git_details(path)
            if not info['is_git'] or info['head']!=head or self._text(path,'symbolic-ref','HEAD')!='refs/heads/'+branch:
                raise SessionError('The target checkout changed. Preview again.')
            self._settled(path)
            if info['dirty']: raise SessionError('The target worktree has uncommitted changes. Commit or move them before delivery.')
        return {'branch':branch,'head':head,'workspace':target}

    def _remove_worktree(self, root, folder):
        self._git(root,'worktree','remove','--force',str(folder),allowed=(0,128),cleanup=True)

    def preview_transfer(self, sid, data):
        if not isinstance(data,dict) or set(data) not in ({'commit','target_branch'},{'commit','target_branch','check'}):
            raise SessionError('Select one commit and a local target branch.')
        command=data.get('check')
        if command is not None: self.sessions.results.check_command(command)
        commit=data['commit']
        if not isinstance(commit,str) or not re.fullmatch(r'[0-9a-f]{40}|[0-9a-f]{64}',commit): raise SessionError('Select a full commit ID.')
        with self.sessions.lock:
            self._idle();session,root,info=self._source(sid);pending=self._pending(root,info)
            if commit not in {c['sha'] for c in self._commits(session,root,info)}: raise SessionError('Select a single-parent commit created after this session baseline.')
            if data['target_branch']==info['branch']: raise SessionError('Select a different target branch.')
            target=self._target(root,data['target_branch'])
            with tempfile.TemporaryDirectory(dir=self.folder,prefix='transfer-') as folder:
                work=Path(folder)/'worktree'
                self._git(root,'worktree','add','--quiet','--detach',str(work),target['head'])
                try:
                    picked=self._git(work,'cherry-pick','--no-edit',commit,allowed=(0,1,128))
                    conflicts=self._text(work,'diff','--name-only','--diff-filter=U').splitlines()
                    candidate=self._text(work,'rev-parse','HEAD') if picked['exit_code']==0 else None
                    # Always show the whole commit; a session may select a repository subdirectory.
                    parent=self._text(root,'rev-parse',commit+'^')
                    display_root=Path(info['root'])
                    files=self._names(display_root,target['head'],candidate) if candidate else self._names(display_root,parent,commit)
                    patch=self._patch(display_root,target['head'],candidate) if candidate else self._patch(display_root,parent,commit)
                    notice='Source checks do not certify the resulting target commit. If a target check is selected, it must pass before Apply. Apply fast-forwards only the reviewed target branch.' if candidate else 'This commit cannot be transferred cleanly. Target files are unchanged. '+picked['output'][:2000]
                finally: self._remove_worktree(root,work)
            return self._remember(sid,{'kind':'transfer','can_apply':bool(candidate),'commit':commit,'source_head':info['head'],
                'target_branch':target['branch'],'target_head':target['head'],'candidate':candidate,'check':command,'files':files,'diff':patch,'conflicts':conflicts,
                'checks':self._checks(sid),'notice':notice,'_candidate':candidate,'_target':target,'_snapshot':pending['snapshot_id']})

    def current_preview(self, sid):
        for key,preview in self.previews.items():
            if preview['sid']==sid and preview['kind']=='transfer' and time.monotonic()-preview['created']<TTL:
                value={k:v for k,v in preview.items() if not k.startswith('_') and k not in ('sid','created')}
                value['preview_id']=key
                value['target_check']=self._target_check(preview)
                if (value['target_check'] or {}).get('status')=='passed':
                    try: self._validate_transfer(sid,preview)
                    except SessionError as error:
                        value['can_apply']=False; value['notice']=str(error)
                return value
        return None

    def _target_check(self, preview):
        row=self.sessions.db.execute('SELECT data FROM checks WHERE id=?',(preview.get('_check_id'),)).fetchone()
        return json.loads(row[0]) if row else None

    def _validate_transfer(self, sid, preview):
        session,root,info=self._source(sid)
        if self._pending(root,info)['snapshot_id']!=preview['_snapshot']:
            raise SessionError('Source workspace changed. Preview the transfer again.')
        target=self._target(root,preview['target_branch'])
        if target!=preview['_target']: raise SessionError('Target branch or checkout changed. Preview the transfer again.')
        if self._text(root,'rev-parse',preview['_candidate']+'^')!=target['head']:
            raise SessionError('The preview commit is unavailable or changed.')
        return session,root,target

    def start_check(self, sid, data):
        with self.sessions.lock:
            self._idle(); preview=self._take(sid,data,'transfer')
            self._validate_transfer(sid,preview)
            if preview['check'] is None: raise SessionError('Select a check command and prepare a new preview.')
            check=self.sessions.results.queue_check(sid,preview['check'],{
                'kind':'target','preview_id':data['preview_id'],'candidate':preview['_candidate'],
                'target_branch':preview['target_branch'],'target_head':preview['target_head'],
                'snapshot_id':None,'snapshot_complete':False})
            preview['_check_id']=check['id']
            return {'session':self.sessions.get(sid)}

    def run_check(self, check, update):
        sid=check['session_id']
        with self.sessions.lock:
            preview=self._take(sid,{'preview_id':check['preview_id']},'transfer')
            if preview.get('_check_id')!=check['id']: raise SessionError('This target check was superseded.')
            session,root,target=self._validate_transfer(sid,preview)
            relative=root.relative_to(Path(git_details(root,include_status=False)['root']))
        with tempfile.TemporaryDirectory(dir=self.folder,prefix='check-') as folder:
            work=Path(folder)/'worktree'
            try:
                self._git(root,'worktree','add','--quiet','--detach',str(work),check['candidate'])
                info=git_details(work,include_status=False); before=self._pending(work,info)['snapshot_id']
                check_root=work/relative
                if check_root.resolve()!=check_root or not check_root.is_dir():
                    raise SessionError('The project directory is missing or linked in the target commit.')
                result=self.sessions.results.capture(check['argv'],check_root,check['timeout'],sid,update)
                check.update(result)
                if result['reason']: return result
                check['workspace_changed']=self._pending(work,git_details(work,include_status=False))['snapshot_id']!=before
                if check['workspace_changed'] and result['reason'] is None:
                    result['reason']='workspace_changed'
                with self.sessions.lock:
                    self._take(sid,{'preview_id':check['preview_id']},'transfer')
                    self._validate_transfer(sid,preview)
                return result
            finally: self._remove_worktree(root,work)

    def apply(self, sid, data):
        with self.sessions.lock:
            self._idle();preview=self._take(sid,data,'transfer')
            session,root,target=self._validate_transfer(sid,preview)
            check=self._target_check(preview)
            if preview['check'] is not None and (not check or check['status']!='passed' or
                    check.get('workspace_changed') or check.get('candidate')!=preview['_candidate']):
                raise SessionError('Run the selected target check successfully before applying this preview.')
            self.previews.pop(data['preview_id'])
            candidate=preview['_candidate']
            with tempfile.TemporaryDirectory(dir=self.folder,prefix='apply-') as folder:
                work=Path(target['workspace']) if target['workspace'] else Path(folder)/'worktree'
                created=not target['workspace']
                if created: self._git(root,'worktree','add','--quiet',str(work),target['branch'])
                try:
                    current=self._target(root,target['branch'])
                    if current['head']!=target['head'] or current['workspace']!=str(work): raise SessionError('Target changed while preparing delivery.')
                    self.sessions._event(sid,{'kind':'status','text':'Applying reviewed commit '+preview['commit']+' to '+target['branch']+' at '+target['head']+'.'})
                    self._git(work,'merge','--ff-only','--no-edit','--no-overwrite-ignore',candidate)
                    if self._text(work,'rev-parse','HEAD')!=candidate: raise SessionError('Inspect the target: Git did not finish at the reviewed commit.')
                finally:
                    if created: self._remove_worktree(root,work)
            detail='Selected target check passed for this commit.' if check else 'No target check was selected.'
            self.sessions._event(sid,{'kind':'status','text':'Delivered '+candidate+' to '+target['branch']+'. '+detail})
            return {'ok':True,'commit':candidate,'branch':target['branch'],'detail':'Reviewed commit applied locally. '+detail}

    def act(self, sid, data):
        if not isinstance(data,dict): raise SessionError('Invalid delivery action.')
        actions={'preview_commit':self.preview_commit,'commit':self.commit,'preview_transfer':self.preview_transfer,'apply':self.apply,'check':self.start_check}
        action=data.get('action')
        if not isinstance(action,str) or action not in actions: raise SessionError('Invalid delivery action.')
        return actions[action](sid,{k:v for k,v in data.items() if k!='action'})
