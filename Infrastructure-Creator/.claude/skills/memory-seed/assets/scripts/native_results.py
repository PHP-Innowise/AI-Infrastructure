"""One bounded native Stop continuation; only structured, guarded results persist.

No transcript or assistant prose is parsed into knowledge. The same agent is
asked once to submit its result through memory_results. Local challenges are
metadata only, disposable, scoped to one project/owner/host/session and prompt.
"""
from __future__ import annotations

import hashlib
import json
import math
import os
from pathlib import Path
import re
import shlex
import shutil
import stat
import subprocess
import sys
import time
from uuid import uuid4

import workspace_roots
from brain_runtime import (BrainError, BrainRecordNotFound, atomic_json, brain_root,
                           linked_reparse, mutation_lock)
from memory_results import (PROOF_PROFILE, _text, _attempt, _read_attempt, _read_receipt,
                            check_task_id, normalize, prepared_scope, record_result,
                            request_digest, writable_task)

PROFILE = 'native-result-challenge-v1'
REASON = 'Automatic native result; agent-attested, not reviewed by a person'
MAX_INPUT = 1024 * 1024
MAX_STATES = 128
HEX = re.compile(r'[0-9a-f]{64}')
TOKEN = re.compile(r'[0-9a-f]{32}')
FIELDS = {'profile','owner','host','session','project','task_id','token','phase',
          'task_uuid','revision','scope','digest','updated','checkout','adapter'}
PHASES = {'armed','requested','saved','held','omitted','failed'}


def read_input(stream):
    raw = stream.read(MAX_INPUT + 1)
    if len(raw) > MAX_INPUT:
        raise BrainError('native-result:input-too-large')
    try:
        value = json.loads(raw)
    except (ValueError, UnicodeError, RecursionError) as error:
        raise BrainError('native-result:invalid-input') from error
    if not isinstance(value, dict):
        raise BrainError('native-result:invalid-input')
    return value


def _enabled(mode):
    return mode == 'governed' and not any(os.environ.get(name) == '1' for name in
        ('CONTEXT_MEMORY_RECOVERY','CONTEXT_CAPSULE_DELIVERED'))


def _project(root):
    return hashlib.sha256(str(workspace_roots.project_root(root).resolve()).encode()).hexdigest()


def _key(root, host, payload, task_id, owner):
    session = payload.get('conversation_id' if host == 'cursor' else 'session_id')
    if not isinstance(session, str) or not re.fullmatch(r'[A-Za-z0-9._:-]{1,160}', session):
        return None
    return _identity(_project(root),owner,host,hashlib.sha256(session.encode()).hexdigest(),task_id)


def _identity(project, owner, host, session, task_id):
    return hashlib.sha256(json.dumps([project,owner,host,session,task_id]).encode()).hexdigest()


def _file(root, key):
    if not isinstance(key, str) or not HEX.fullmatch(key):
        raise BrainError('native-result:invalid-request')
    directory = brain_root(root) / 'local' / 'native-results'
    for parent in (directory.parent.parent, directory.parent, directory):
        if os.path.lexists(parent):
            image = parent.lstat()
            if not stat.S_ISDIR(image.st_mode) or linked_reparse(image):
                raise BrainError('native-result:unsafe-storage')
    destination = directory / (key + '.json')
    if os.path.lexists(destination):
        image = destination.lstat()
        if not stat.S_ISREG(image.st_mode) or image.st_nlink != 1 or linked_reparse(image):
            raise BrainError('native-result:unsafe-storage')
    return destination


def _read(root, key):
    value = _read_receipt(_file(root, key))
    if value is None:
        if os.path.lexists(_file(root,key)):
            raise BrainError('native-result:invalid-state')
        return None
    if (set(value) != FIELDS or value['profile'] != PROFILE
            or not isinstance(value['phase'],str) or value['phase'] not in PHASES
            or not isinstance(value['host'],str) or value['host'] not in {'claude','codex','cursor'}
            or not isinstance(value['token'],str) or not TOKEN.fullmatch(value['token'])
            or type(value['updated']) not in (int,float) or not math.isfinite(value['updated'])
            or value['updated'] < 0
            or value['revision'] is not None and (type(value['revision']) is not int or value['revision'] < 1)
            or any(not isinstance(value[field],str) or not HEX.fullmatch(value[field]) for field in ('project','session'))
            or any(value[field] is not None and (not isinstance(value[field],str) or not HEX.fullmatch(value[field]))
                   for field in ('scope','digest','checkout','adapter'))
            or value['task_uuid'] is not None and (not isinstance(value['task_uuid'],str) or not re.fullmatch(
                r'[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}',value['task_uuid']))):
        raise BrainError('native-result:invalid-state')
    _text(value['owner'],'owner',200)
    check_task_id(value['task_id'])
    if (value['project'] != _project(root) or _identity(value['project'],value['owner'],value['host'],
            value['session'],value['task_id']) != key):
        raise BrainError('native-result:scope-changed')
    return value


def _write(root, key, value):
    atomic_json(_file(root,key), value)


def _prune(root, key):
    directory = _file(root,key).parent
    candidates=[]
    for item in directory.glob('*.json'):
        if HEX.fullmatch(item.stem) and item.stem != key:
            info=item.lstat()
            if stat.S_ISREG(info.st_mode) and info.st_nlink == 1 and not linked_reparse(info):
                candidates.append((info.st_mtime_ns,item))
    for _,item in sorted(candidates)[:max(0,len(candidates)-(MAX_STATES-1))]:
        item.unlink()


def begin(root, task_id, host, payload, *, owner, mode):
    task_id=check_task_id(task_id)
    if not _enabled(mode): return {'armed':False}
    key=_key(root,host,payload,task_id,owner)
    if key is None: return {'armed':False}
    with mutation_lock(root):
        previous=_read(root,key)
        # Codex submits the Stop reason through UserPromptSubmit too. It is
        # bookkeeping for this prompt, never a new generation of user work.
        prompt=payload.get('prompt')
        if (previous and previous['phase']=='requested' and isinstance(prompt,str)
                and prompt.lstrip().startswith('INTERNAL_NATIVE_MEMORY_RESULT '+previous['token'])):
            return {'armed':False,'continuation':True}
        warning = bool(previous and previous['phase'] in {'requested','failed','held'})
        state={'profile':PROFILE,'owner':owner,'host':host,
               'session':hashlib.sha256(str(payload.get('conversation_id' if host=='cursor' else 'session_id')).encode()).hexdigest(),
               'project':_project(root),'task_id':task_id,'token':uuid4().hex,
               'phase':'armed','task_uuid':None,'revision':None,'scope':None,'digest':None,
               'checkout':None,'adapter':None,'updated':time.time()}
        _write(root,key,state);_prune(root,key)
    return {'armed':True,'request':key,**({'warning':'Previous native memory result was not saved completely.'} if warning else {})}


def recovery(host, payload):
    return (type(payload.get('loop_count')) is int and payload['loop_count'] > 0
            if host=='cursor' else payload.get('stop_hook_active') is True)


def _checkout(root):
    try:
        result=subprocess.run(['git','-C',str(workspace_roots.project_root(root)),
            'rev-parse','HEAD','--symbolic-full-name','HEAD'],stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,timeout=2,check=False)
    except (OSError,subprocess.TimeoutExpired) as error:
        raise BrainError('native-result:checkout-unavailable') from error
    # A project without Git has no checkout identity to pin. The project
    # root still participates in the challenge's identity and prepared scope.
    return hashlib.sha256(result.stdout if result.returncode==0 else b'no-git-head').hexdigest()


def _adapter():
    return hashlib.sha256(Path(__file__).read_bytes()).hexdigest()


def _instruction(root,key,state):
    tooling=Path(__file__).resolve().parents[2]
    interpreter=['python3']
    if os.name == 'nt':
        interpreter=['python'] if shutil.which('python') else ['py','-3']
    # Keep the documented CLI spelling covered by the installed Bash
    # permission rules, even when the session's cwd is a subdirectory.
    command='(cd '+shlex.quote(str(tooling))+' && '+shlex.join([
        *interpreter,'memory-bank/scripts/context.py','--root',str(root),'native-result',
        '--request',key,'--token',state['token'],'--input','-','--json'])+')'
    return ('INTERNAL_NATIVE_MEMORY_RESULT '+state['token']+'\n'
        'This is an internal continuation: do not announce routine memory operations. Save only the result of the work already performed in this session. Do not repeat the task, '
        'edit application files, run tests, delegate, or mark the task completed. '
        'Use your shell tool to pipe one JSON object into this exact command: '+command+'\n'
        'JSON: progress (summary, at most 1000 characters), optional next_steps '
        '(at most 3 strings of 300 characters; omit to preserve the current plan), '
        'learnings (at most 3 objects with type finding|decision, title, consequence, sources). '
        'Exact schema example (replace placeholders, never save the example): '
        '{"progress":"summary","verified":true,"learnings":[{"type":"finding",'
        '"title":"checked rule","consequence":"reusable consequence","sources":["checked-file.php"]}]}. '
        'verified is a TOP-LEVEL boolean, NEVER a field inside a learning. '
        'Only NEW or corrected reusable consequences belong in learnings. If this task only '
        'recalls or rechecks an already recorded rule, use learnings=[] and save progress only; '
        'never create another finding merely to say that an old rule was verified again. '
        'Use only canonical project-relative source files you actually checked. Set verified=true '
        'only when you personally checked every learning against current sources; this is agent '
        'attestation, never human review. Empty learnings is valid. No raw requests, transcripts, '
        'logs, credentials or personal data. If no project result is safe to save, use the same '
        'command replacing --input - with --skip. Check the command result. After a successful save or skip, '
        'return the substantive answer to the original user request, retaining its facts, source citations '
        'and requested format; do not replace that answer with a save acknowledgement or mention routine '
        'memory operations. If a save is refused, retain the substantive answer and briefly report the held save. '
        'If JSON/schema is refused before any write, correct its format once in this continuation. '
        'On stale revision, scope or ownership refusal, stop and report the held save; never override '
        'newer work or repeat the user task.')


def _proven(root,state):
    if not state['task_uuid'] or not state['digest']: return False
    proof=_read_attempt(_attempt(root,state['task_uuid'],'native-'+state['token'],state['digest']))
    return (proof is not None and proof.get('profile') == PROOF_PROFILE
            and proof.get('state') == 'complete' and proof.get('task_id') == state['task_uuid']
            and proof.get('result_id') == 'native-'+state['token'] and proof.get('digest') == state['digest']
            and proof.get('owner') == state['owner'] and proof.get('attestation') == 'agent')


def stop(root, task_id, host, payload, *, owner, mode):
    if not _enabled(mode): return {}
    task_id=check_task_id(task_id);key=_key(root,host,payload,task_id,owner)
    if key is None: return {}
    if host=='cursor' and payload.get('status') != 'completed': return {}
    if host!='cursor' and payload.get('hook_event_name','Stop') != 'Stop': return {}
    with mutation_lock(root):
        state=_read(root,key)
        if state is None: return {}
        if recovery(host,payload):
            if state['phase']=='requested':
                state['phase']='held' if _proven(root,state) else 'failed'
                _write(root,key,state)
            return {}
        if state['phase'] != 'armed': return {}
        try: task=writable_task(root,task_id,owner)
        except BrainRecordNotFound as error:
            if error.identifier != task_id: raise
            task=None
        if task is not None:
            if task['status'] in {'completed','cancelled'}: return {}
            state.update(task_uuid=task['id'],revision=task['revision'])
        state.update(phase='requested',scope=prepared_scope(root,owner,REASON,'agent'),
                     checkout=_checkout(root),adapter=_adapter(),updated=time.time())
        _write(root,key,state)
        instruction=_instruction(root,key,state)
    return {'followup_message':instruction} if host=='cursor' else {'decision':'block','reason':instruction}


def save(root,key,token,data,*,owner,mode,provision,skip=False):
    if not _enabled(mode): raise BrainError('native-result:governed-session-required')
    if not isinstance(token,str) or not TOKEN.fullmatch(token): raise BrainError('native-result:invalid-token')
    with mutation_lock(root):
        state=_read(root,key)
        if (state is None or state['token'] != token or state['owner'] != owner
                or state['project'] != _project(root)
                or state['phase'] not in {'requested','saved','held','failed'}):
            raise BrainError('native-result:request-unavailable')
        begun=state['digest'] is not None
        if state['phase'] != 'requested' and not begun:
            raise BrainError('native-result:request-unavailable')
        if begun and (state['task_uuid'] is None or state['revision'] is None):
            raise BrainError('native-result:invalid-state')
        if (time.time()-state['updated'] > 24*60*60 or state['updated'] > time.time()+60
                or state['checkout'] != _checkout(root) or state['adapter'] != _adapter()
                or state['scope'] != prepared_scope(root,owner,REASON,'agent')):
            raise BrainError('native-result:scope-changed')
        if skip:
            if state['digest'] is not None: raise BrainError('native-result:save-already-started')
            state['phase']='omitted';_write(root,key,state)
            return {'state':'omitted','attestation':'agent'}
        # A completed replay may outlive its sources or active task alias.
        # Only the canonical result protocol can admit such a replay; local
        # challenge metadata alone must never authorize a fresh write.
        normalized=normalize(root,data,check_files=not begun)
        if (state['task_uuid'] is None and normalized['progress'] is None
                and not normalized['next_steps'] and not normalized['learnings']):
            raise BrainError('native-result:nothing-to-record')
        if state['task_uuid'] is None:
            try: writable_task(root,state['task_id'],owner)
            except BrainRecordNotFound as error:
                if error.identifier != state['task_id']: raise
            else: raise BrainError('native-result:task-changed')
            provision(state['task_id'])
            task=writable_task(root,state['task_id'],owner)
            state.update(task_uuid=task['id'],revision=task['revision'])
            _write(root,key,state)
        if not begun:
            task=writable_task(root,state['task_id'],owner)
            if task['id'] != state['task_uuid']:
                raise BrainError('native-result:task-changed')
        digest=request_digest(state['task_uuid'],normalized)
        if state['digest'] is not None and state['digest'] != digest:
            raise BrainError('native-result:content-changed')
        state['digest']=digest;_write(root,key,state)
        result=record_result(root,state['task_uuid'],'native-'+token,state['revision'],data,
                             owner=owner,reason=REASON,attestation='agent',prepared=state['scope'],
                             replay_only=begun)
        held=any(item['state'] in {'differs','archived-differs'} for item in result['records'])
        state['phase']='held' if held else 'saved';_write(root,key,state)
        return result
