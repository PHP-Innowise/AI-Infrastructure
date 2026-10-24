"""One bounded native Stop continuation; only structured, guarded results persist.

No transcript or assistant prose is parsed into knowledge. The same agent is
asked once to submit its result through memory_results. Local challenges are
metadata only, disposable, scoped to one project/owner/host/session and prompt.
Every prompt, Stop and save also leaves one metadata line in a local journal,
so how far the saves get can be counted from disk (see _journal).
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
from brain_runtime import (BrainError, BrainRecordNotFound, atomic_json, atomic_write_bytes,
                           brain_root, linked_reparse, load_config, mutation_lock)
from memory_results import (PROOF_PROFILE, _text, _attempt, _read_attempt, _read_receipt,
                            check_task_id, normalize, prepared_scope, record_result,
                            refusal_code, request_digest, writable_task)

PROFILE = 'native-result-challenge-v1'
REASON = 'Automatic native result; agent-attested, not reviewed by a person'
MAX_INPUT = 1024 * 1024
MAX_STATES = 128
HEX = re.compile(r'[0-9a-f]{64}')
TOKEN = re.compile(r'[0-9a-f]{32}')
FIELDS = {'profile','owner','host','session','project','task_id','token','phase',
          'task_uuid','revision','scope','digest','updated','checkout','adapter'}
PHASES = {'armed','requested','saved','held','omitted','failed','refused','declined'}
HOSTS = {'claude','codex','cursor'}
JOURNAL = 'native-results.ndjson'
# A fully exercised turn writes about six lines - armed; requested and the
# Stop hook's exit; the save; the closing Stop and its exit - and Codex one
# more for its continuation prompt. Eight per refresh-health record keeps at
# least the turns that series keeps, so both read over one window.
JOURNAL_LINES_PER_TURN = 8
# Trimmed back to the kept lines once the file passes this many bytes a kept
# line: an append stays one write, and a trim is rare.
JOURNAL_LINE_BYTES = 128


def _journal(root, host, stage, event, code=None, **counts):
    """Append one line of the save funnel, or quietly nothing.

    The challenge file is rewritten every turn and pruned to MAX_STATES, and
    a refused save's code reached only the agent's tool output, so nothing on
    disk said how many saves were requested, made or refused, or why not.
    A line holds when, which host, which step, a fixed code and two counts -
    never the payload, the result, a path, a task or a session. Beside
    refresh-health.ndjson and like it: its directory is followed as the
    runtime's other local files are, the file itself never through a link,
    and a journal write never fails the step it records. A trim racing
    another session's append may drop that one line.
    """
    record={'at':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),
            'host':host if host in HOSTS else None,'stage':stage,'event':event}
    if code is not None: record['code']=code
    record.update(counts)
    try:
        path=Path(root)/'memory-bank'/'local'/JOURNAL
        if os.path.lexists(path):
            image=path.lstat()
            if stat.S_ISLNK(image.st_mode) or linked_reparse(image): return
        path.parent.mkdir(parents=True,exist_ok=True)
        descriptor=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_APPEND|getattr(os,'O_NOFOLLOW',0)
                           |getattr(os,'O_BINARY',0),0o666)
        try:
            os.write(descriptor,(json.dumps(record,separators=(',',':'))+'\n').encode('utf-8'))
            size=os.fstat(descriptor).st_size
        finally:
            os.close(descriptor)
        from context_retrieval import refresh_health_retention
        keep=refresh_health_retention(load_config(root))*JOURNAL_LINES_PER_TURN
        if size>keep*JOURNAL_LINE_BYTES:
            lines=path.read_bytes().splitlines(keepends=True)
            if len(lines)>keep: atomic_write_bytes(path,b''.join(lines[-keep:]))
    except (OSError,ValueError,BrainError,ImportError):
        return


def _peek_host(root, key):
    """The host a challenge was armed for, best effort, for a refusal's line."""
    try:
        state=_read(root,key)
    except (BrainError,OSError,ValueError,TypeError):
        return None
    return state['host'] if state else None


class _Step:
    """The one journal line of one prompt, Stop or save call.

    The call names its outcome as it returns - `return step(result, event,
    code)` - and an exception names itself on the way out, a refusal by its
    machine code and anything else by its type, and still propagates. A call
    that names no outcome leaves no line.
    """

    def __init__(self, root, stage, host=None, key=None):
        self.root,self.stage,self.host,self.key=root,stage,host,key
        self.outcome=None

    def __call__(self, result, event, code=None, **counts):
        self.outcome=(event,code,counts)
        return result

    def __enter__(self):
        return self

    def __exit__(self, kind, error, trace):
        if kind is None:
            if self.outcome is not None:
                event,code,counts=self.outcome
                _journal(self.root,self.host,self.stage,event,code,**counts)
        elif issubclass(kind,Exception):
            host=self.host if self.host is not None else _peek_host(self.root,self.key)
            if isinstance(error,BrainError):
                _journal(self.root,host,self.stage,'refused' if self.stage=='save' else 'error',
                         refusal_code(error))
            else:
                _journal(self.root,host,self.stage,'error',type(error).__name__)
        return False


def read_input(stream, root=None, request=None):
    """One JSON object from `stream`, or BrainError.

    `root` and `request` name the save the object is for: an input refused
    here never reaches save(), so its refusal is journaled here.
    """
    if root is None: return _parse_input(stream)
    with _Step(root,'save',key=request):
        return _parse_input(stream)


def _invalid_input(detail):
    """The refusal of an input that is not one JSON object: the journal counts
    its code, the agent reads what to correct in its one retry."""
    error = BrainError('native-result:invalid-input: ' + detail)
    error.code = 'native-result:invalid-input'
    return error


def _overtaken(code, what):
    """The refusal of a save another writer overtook - in practice a second
    session on the branch saving first. The bare code left the agent to
    guess; its instruction says to stop and report the held save."""
    error = BrainError(f'{code}: {what} after this save was requested; nothing was written: '
                       'report the held save, do not retry it')
    error.code = code
    return error


JSON_KINDS = {list: 'an array', str: 'a string', int: 'a number', float: 'a number', bool: 'true or false',
              type(None): 'null'}


def _parse_input(stream):
    raw = stream.read(MAX_INPUT + 1)
    if len(raw) > MAX_INPUT:
        raise BrainError('native-result:input-too-large')
    try:
        value = json.loads(raw)
    except json.JSONDecodeError as error:
        where = f'line {error.lineno} column {error.colno}'
        if error.msg == 'Extra data':
            raise _invalid_input(f'more than one JSON object (more after {where})') from error
        # json's message for a control character already ends in "at".
        detail = f'not one JSON object ({error.msg.removesuffix(" at")} at {where})'
        # A shell's echo turns the \n of a JSON string into a real newline:
        # the agent did write \n, so the hint names the echo too.
        if error.msg.startswith('Invalid control character'):
            detail += ('; inside a string, write a newline as \\n and a tab as \\t (a plain echo in zsh turns '
                       '\\n into a line break: pipe with echo -E)')
        raise _invalid_input(detail) from error
    except (ValueError, UnicodeError, RecursionError) as error:
        raise _invalid_input('not one UTF-8 JSON object') from error
    if not isinstance(value, dict):
        raise _invalid_input('one JSON object expected, got ' + JSON_KINDS.get(type(value), 'another value'))
    return value


def _enabled(mode):
    return mode == 'governed' and not any(os.environ.get(name) == '1' for name in
        ('CONTEXT_MEMORY_RECOVERY','CONTEXT_CAPSULE_DELIVERED'))


# Whether a Stop asks the agent to save its result: `every-turn`, the default,
# arms every real prompt; `off` arms none, and results reach memory only
# through record-result, the MCP tool or the Harness. The request costs one
# more model call per answer - on 21 Codex runs a median of 10 s, 383 output
# and 59k (mostly cached) input tokens, about half the answer's own - which a
# team may not want on every turn. CONTEXT_NATIVE_RESULT_REQUEST overrides
# `native_result_request` in runtime.json.
NATIVE_REQUESTS = ('every-turn', 'off')


def configured_request(root):
    """The native result request setting, or BrainError for an unknown one."""
    value = (os.environ.get('CONTEXT_NATIVE_RESULT_REQUEST')
             or load_config(root).get('native_result_request') or 'every-turn')
    if value not in NATIVE_REQUESTS:
        raise BrainError('native-result:invalid-request-setting')
    return value


# How the continuation ends once the save is made. A headless run - `codex
# exec`, `claude -p` - hands its caller only the final message, so there the
# agent returns its answer again. In a session a person reads as it goes the
# answer is already on screen above the continuation, and the repeat doubled
# it: on nine Codex runs the message after the save was 93 % of the answer's
# length, 21 % before the repeat was asked for. `auto`, the default, ends a
# session its host marks as attended with at most one short line and repeats
# everywhere else; `repeat` always repeats, `brief` never does.
# CONTEXT_NATIVE_RESULT_ANSWER overrides `native_result_answer` in runtime.json.
NATIVE_ANSWERS = ('auto', 'repeat', 'brief')
# Claude Code marks the processes of a session a person attends with
# CLAUDE_CODE_SESSION_ATTENDED=1. A `claude -p` started from such a session
# inherits its entrypoint, but it is a child session, which Claude Code never
# marks attended. The entrypoint then names an interface that shows the whole
# conversation; Slack and Teams sessions are attended too, but may show less.
ATTENDED_ENTRYPOINTS = frozenset({'cli', 'claude-vscode', 'claude-desktop'})
# Codex writes where a session came from into the first line of its rollout:
# `vscode` for the app, the IDE extension and the terminal UI (`cli` before
# them), `exec` for `codex exec`, an object for a subagent.
ATTENDED_SOURCES = frozenset({'cli', 'vscode'})
ROLLOUT_HEAD_BYTES = 256 * 1024
# The current turn's context is near the rollout's end; one that lies
# further back is unknown, and an unknown turn is asked as before.
ROLLOUT_TAIL_BYTES = 512 * 1024
# Claude Code records on a tool call's result how it refused the call. A save
# the host refused - a person said no, a rule or auto mode denied it, or `-p`
# had no one to ask, as in a project whose trust was never accepted, whose
# allow rules Claude Code ignores - is refused again after every answer,
# each time for two model calls. Such a session is not asked again for a day.
# Auto mode's classifier failing (`automode-unavailable`, `-parsing-error`) is
# "not a judgment that the action is unsafe", and the next answer may pass.
CLAUDE_REFUSALS = frozenset({'user-rejected','permission-rule','automode-blocked'})
REFUSED_SECONDS = 24 * 60 * 60
CLAUDE_TAIL_BYTES = 512 * 1024


# The rules a save step carries - the schema, what belongs in learnings - are
# the same after every answer: about 1,150 of its 2,700 characters, which
# Claude Code's terminal also shows whole each time. A step whose
# conversation still holds a full one carries the command and points back to
# it. The full step is sent again once a compaction has dropped it, or when
# the transcript cannot say. `full` sends it every time;
# CONTEXT_NATIVE_RESULT_INSTRUCTION overrides `native_result_instruction`.
NATIVE_INSTRUCTIONS = ('auto', 'full')
FULL_RULES = 'Exact schema example (replace placeholders, never save the example)'
RULES_TAIL_BYTES = 2 * 1024 * 1024
COMPACTION_RECORDS = (b'"subtype":"compact_boundary"', b'"type":"compacted"')


def configured_instruction(root):
    """Whether a save step may point back to the full one; an unknown value sends it whole."""
    value = (os.environ.get('CONTEXT_NATIVE_RESULT_INSTRUCTION')
             or load_config(root).get('native_result_instruction') or 'auto')
    return value if value in NATIVE_INSTRUCTIONS else 'full'


def _rules_in_context(payload):
    """Whether the conversation still holds a full save step: its rules are
    in the transcript's last 2 MiB, after the last compaction record."""
    path = payload.get('transcript_path')
    if not isinstance(path, str) or not path:
        return False
    try:
        if not stat.S_ISREG(os.stat(path).st_mode):
            return False
        with open(path, 'rb') as handle:
            handle.seek(0, os.SEEK_END)
            handle.seek(max(0, handle.tell() - RULES_TAIL_BYTES))
            tail = handle.read(RULES_TAIL_BYTES)
    except OSError:
        return False
    start = max(tail.rfind(record) for record in COMPACTION_RECORDS)
    # The model holds what follows the compaction record's own line: Codex's
    # record carries a retained context with earlier steps in it, which the
    # model no longer sees.
    begin = 0 if start < 0 else (tail.find(b'\n', start) + 1 or len(tail))
    return tail.find(FULL_RULES.encode(), begin) >= 0


def configured_answer(root):
    """The native result answer setting, or BrainError for an unknown one."""
    value = (os.environ.get('CONTEXT_NATIVE_RESULT_ANSWER')
             or load_config(root).get('native_result_answer') or 'auto')
    if value not in NATIVE_ANSWERS:
        raise BrainError('native-result:invalid-answer-setting')
    return value


def _rollout_source(payload):
    """Where the Codex session behind this Stop came from, or None."""
    path = payload.get('transcript_path')
    if not isinstance(path, str) or not path:
        return None
    try:
        if not stat.S_ISREG(os.stat(path).st_mode):
            return None
        with open(path, 'rb') as handle:
            head = handle.readline(ROLLOUT_HEAD_BYTES + 1)
        record = json.loads(head) if len(head) <= ROLLOUT_HEAD_BYTES else None
    except (OSError, ValueError, RecursionError):
        return None
    meta = record.get('payload') if isinstance(record, dict) and record.get('type') == 'session_meta' else None
    source = meta.get('source') if isinstance(meta, dict) else None
    return source if isinstance(source, str) else None


def _rollout_turn(payload):
    """The Codex turn behind this Stop: its collaboration mode and sandbox
    type from the rollout's turn_context record with the Stop's turn_id,
    or Nones. Codex's permission_mode follows the approval policy only."""
    path, turn = payload.get('transcript_path'), payload.get('turn_id')
    if not isinstance(path, str) or not path or not isinstance(turn, str) or not turn:
        return None, None
    try:
        if not stat.S_ISREG(os.stat(path).st_mode):
            return None, None
        with open(path, 'rb') as handle:
            handle.seek(0, os.SEEK_END)
            handle.seek(max(0, handle.tell() - ROLLOUT_TAIL_BYTES))
            tail = handle.read(ROLLOUT_TAIL_BYTES)
    except OSError:
        return None, None
    for line in reversed(tail.split(b'\n')):
        if b'turn_context' not in line:
            continue
        try:
            record = json.loads(line)
        except (ValueError, RecursionError):
            continue  # the tail's first line may be cut
        context = record.get('payload') if isinstance(record, dict) and record.get('type') == 'turn_context' else None
        if not isinstance(context, dict):
            continue
        if context.get('turn_id') != turn:
            return None, None
        mode = context.get('collaboration_mode')
        sandbox = context.get('sandbox_policy')
        mode = mode.get('mode') if isinstance(mode, dict) else None
        sandbox = sandbox.get('type') if isinstance(sandbox, dict) else None
        return (mode if isinstance(mode, str) else None), (sandbox if isinstance(sandbox, str) else None)
    return None, None


def _claude_refusal(payload, token):
    """Whether Claude Code refused this request's save command: the denial
    kind it recorded on the command's result in the transcript tail."""
    path = payload.get('transcript_path')
    if not isinstance(path, str) or not path:
        return False
    try:
        if not stat.S_ISREG(os.stat(path).st_mode):
            return False
        with open(path, 'rb') as handle:
            handle.seek(0, os.SEEK_END)
            handle.seek(max(0, handle.tell() - CLAUDE_TAIL_BYTES))
            tail = handle.read(CLAUDE_TAIL_BYTES)
    except OSError:
        return False
    calls, refused = set(), False
    for line in tail.split(b'\n'):
        if token.encode() not in line and b'tool_result' not in line:
            continue
        try:
            record = json.loads(line)
        except (ValueError, RecursionError):
            continue  # the tail's first line may be cut
        message = record.get('message') if isinstance(record, dict) else None
        content = message.get('content') if isinstance(message, dict) else None
        for block in content if isinstance(content, list) else ():
            if not isinstance(block, dict):
                continue
            if block.get('type') == 'tool_use' and token in json.dumps(block.get('input')):
                calls.add(block.get('id'))
            elif block.get('type') == 'tool_result' and block.get('tool_use_id') in calls:
                # The last attempt decides: a retry the host let through began a save.
                refused = record.get('toolDenialKind') in CLAUDE_REFUSALS
    return refused


def attended(root, host, payload):
    """Whether a person reads this Stop's session as it goes, by the setting
    or, under `auto`, by what the host itself says about the session."""
    setting = configured_answer(root)
    if setting != 'auto':
        return setting == 'brief'
    if host == 'claude':
        return (os.environ.get('CLAUDE_CODE_SESSION_ATTENDED') == '1'
                and os.environ.get('CLAUDE_CODE_ENTRYPOINT') in ATTENDED_ENTRYPOINTS)
    if host == 'codex':
        return _rollout_source(payload) in ATTENDED_SOURCES
    return False


def _disabled(mode):
    """Why _enabled is false, as the code a journal line carries."""
    if mode != 'governed': return 'not-governed'
    return 'capsule-delivered' if os.environ.get('CONTEXT_CAPSULE_DELIVERED') == '1' else 'recovery-session'


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
    with _Step(root,'prompt',host) as step:
        return _begin(step,root,task_id,host,payload,owner=owner,mode=mode)


def _begin(step, root, task_id, host, payload, *, owner, mode):
    task_id=check_task_id(task_id)
    if not _enabled(mode): return step({'armed':False},'skipped',_disabled(mode))
    if configured_request(root) == 'off': return step({'armed':False},'skipped','switched-off')
    key=_key(root,host,payload,task_id,owner)
    if key is None: return step({'armed':False},'skipped','no-session-id')
    with mutation_lock(root):
        previous=_read(root,key)
        # Codex submits the Stop reason through UserPromptSubmit too. It is
        # bookkeeping for this prompt, never a new generation of user work.
        prompt=payload.get('prompt')
        if (previous and previous['phase']=='requested' and isinstance(prompt,str)
                and prompt.lstrip().startswith('INTERNAL_NATIVE_MEMORY_RESULT '+previous['token'])):
            return step({'armed':False,'continuation':True},'continuation')
        # A person who says no in Claude Code's dialog ends the turn before
        # any Stop runs, so the request is still pending. And Claude Code
        # writes its transcript in batches: a refusal just before the
        # recovery Stop may not be on disk yet, and that Stop calls the
        # request failed. Either way the refusal is in the transcript the
        # prompt hook is handed.
        if (previous and previous['phase'] in {'requested','failed'} and host=='claude' and not previous['digest']
                and _claude_refusal(payload,previous['token'])):
            previous.update(phase='refused',updated=time.time())
        # A session whose host refused the save is not asked again for a day;
        # the first prompt after says so as any unsaved result does.
        if (previous and previous['phase'] in {'refused','declined'}
                and 0 <= time.time()-previous['updated'] < REFUSED_SECONDS):
            if previous['phase'] == 'declined': return step({'armed':False},'skipped','host-refused')
            previous.update(phase='declined',updated=time.time());_write(root,key,previous)
            return step({'armed':False,'warning':'Previous native memory result was not saved completely.'},
                        'skipped','host-refused')
        warning = bool(previous and previous['phase'] in {'requested','failed','held','refused'})
        state={'profile':PROFILE,'owner':owner,'host':host,
               'session':hashlib.sha256(str(payload.get('conversation_id' if host=='cursor' else 'session_id')).encode()).hexdigest(),
               'project':_project(root),'task_id':task_id,'token':uuid4().hex,
               'phase':'armed','task_uuid':None,'revision':None,'scope':None,'digest':None,
               'checkout':None,'adapter':None,'updated':time.time()}
        _write(root,key,state);_prune(root,key)
    # The warning is counted by what the last request became.
    return step({'armed':True,'request':key,**({'warning':'Previous native memory result was not saved completely.'} if warning else {})},
                'armed','previous-'+previous['phase'] if warning else None)


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


def _instruction(root,key,state,*,brief=False,host=None,full=True):
    tooling=Path(__file__).resolve().parents[2]
    interpreter=['python3']
    if os.name == 'nt':
        interpreter=['python'] if shutil.which('python') else ['py','-3']
    # Keep the documented CLI spelling covered by the installed Bash
    # permission rules, even when the session's cwd is a subdirectory.
    def cli(*tail):
        return shlex.join([*interpreter,'memory-bank/scripts/context.py','--root',str(root),'native-result',
            '--request',key,'--token',state['token'],*tail])
    enter='cd '+shlex.quote(str(tooling))+' && '
    if host=='claude':
        # Claude Code asks before any command holding a subshell or a { }
        # group, whatever the allow rules say, so (cd ROOT && ...) asked after
        # every answer. It also asks before a heredoc whose body holds braces
        # and quotes, and before a printf operand holding [ with $ or a
        # backtick, which a JSON with code in it has. A cd into the project,
        # echo and the allowed CLI pass; -E keeps the JSON's backslashes in
        # zsh, whose plain echo turns them into characters, as in bash.
        ask=('Use your shell tool to run this exact command: '+enter+"echo -E '<JSON>' | "
             +cli('--input','-','--json')+'\n'
             "Put one JSON object in place of <JSON>, writing each ' inside it as '\\''. ")
        skip='run this command instead: '+enter+cli('--skip','--json')+'\n'
    else:
        ask='Use your shell tool to pipe one JSON object into this exact command: (' \
            +enter+cli('--input','-','--json')+')\n'
        skip='use the same command replacing --input - with --skip. '
    rules=('JSON: progress (summary, at most 1000 characters), optional next_steps '
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
        'logs, credentials or personal data. ') if full else (
        # The conversation holds the full step from an earlier answer.
        'The JSON follows the rules of the full save step earlier in this conversation: progress, optional '
        'next_steps, learnings with checked sources, and verified as a top-level boolean. ')
    body=('This is an internal continuation: do not announce routine memory operations. Save only the result of the work already performed in this session. Do not repeat the task, '
        'edit application files, run tests, delegate, or mark the task completed. '
        +ask+rules+'If no project result is safe to save, '+skip+
        'Check the command result. '
        +('The person has already read your answer above this continuation: after a successful save or skip, '
          'do not repeat it or mention routine memory operations; end with no further text or one short line. '
          'If a save is refused, briefly report the held save. ' if brief else
          'After a successful save or skip, '
          'return the substantive answer to the original user request, retaining its facts, source citations '
          'and requested format; do not replace that answer with a save acknowledgement or mention routine '
          'memory operations. If a save is refused, retain the substantive answer and briefly report the held save. ')
        +'If JSON/schema is refused before any write, correct its format once in this continuation. '
        'On stale revision, scope or ownership refusal, stop and report the held save; never override '
        'newer work or repeat the user task.')
    marker='INTERNAL_NATIVE_MEMORY_RESULT '+state['token']
    # Codex and Cursor send the continuation back as a prompt, known by the
    # marker that opens it. Claude Code shows the first lines of a Stop hook's
    # reason under "Ran N stop hooks" after every answer: there the marker
    # closes the text, and the line a person reads first says what this is.
    return body+'\n'+marker if host=='claude' else marker+'\n'+body


def _proven(root,state):
    if not state['task_uuid'] or not state['digest']: return False
    proof=_read_attempt(_attempt(root,state['task_uuid'],'native-'+state['token'],state['digest']))
    return (proof is not None and proof.get('profile') == PROOF_PROFILE
            and proof.get('state') == 'complete' and proof.get('task_id') == state['task_uuid']
            and proof.get('result_id') == 'native-'+state['token'] and proof.get('digest') == state['digest']
            and proof.get('owner') == state['owner'] and proof.get('attestation') == 'agent')


def stop(root, task_id, host, payload, *, owner, mode):
    with _Step(root,'stop',host) as step:
        return _stop(step,root,task_id,host,payload,owner=owner,mode=mode)


def _stop(step, root, task_id, host, payload, *, owner, mode):
    if not _enabled(mode): return step({},'skipped',_disabled(mode))
    # A prompt armed before the switch went off asks nothing either.
    if configured_request(root) == 'off': return step({},'skipped','switched-off')
    task_id=check_task_id(task_id);key=_key(root,host,payload,task_id,owner)
    if key is None: return step({},'skipped','no-session-id')
    if host=='cursor' and payload.get('status') != 'completed': return step({},'skipped','not-completed')
    if host!='cursor' and payload.get('hook_event_name','Stop') != 'Stop': return step({},'skipped','not-stop-event')
    # An unknown answer setting costs the turn its short ending, never its save.
    try: brief,code=attended(root,host,payload),None
    except BrainError as error: brief,code=False,refusal_code(error)
    # Codex reports Plan mode and a read-only sandbox only in the rollout.
    collaboration,sandbox=_rollout_turn(payload) if host=='codex' else (None,None)
    with mutation_lock(root):
        state=_read(root,key)
        if state is None: return step({},'skipped','not-armed')
        if recovery(host,payload):
            if state['phase']=='requested':
                # Held: a finished save the state never took. Failed: no
                # save began, or one began and did not finish. Refused:
                # Claude Code refused the save command itself.
                if _proven(root,state): state['phase']='held'
                elif host=='claude' and not state['digest'] and _claude_refusal(payload,state['token']):
                    state['phase']='refused'
                else: state['phase']='failed'
                _write(root,key,state)
                return step({},'held' if state['phase']=='held' else 'failed',
                            'unconfirmed' if state['phase']=='held' else 'host-refused' if state['phase']=='refused'
                            else 'incomplete' if state['digest'] else 'unsaved')
            return step({},'skipped','recovery-'+state['phase'])
        if state['phase'] != 'armed': return step({},'skipped','phase-'+state['phase'])
        # The host tells a plan-mode turn to change nothing, overriding any
        # other instruction: asked to save, the agent refuses after every
        # answer, or writes against the person's choice. A save asked
        # before the switch is still settled above.
        if payload.get('permission_mode') == 'plan' or collaboration == 'plan': return step({},'skipped','plan-mode')
        # A read-only sandbox lets no save write the project.
        if sandbox == 'read-only': return step({},'skipped','read-only-sandbox')
        try: task=writable_task(root,task_id,owner)
        except BrainRecordNotFound as error:
            if error.identifier != task_id: raise
            task=None
        if task is not None:
            if task['status'] in {'completed','cancelled'}: return step({},'skipped','task-'+task['status'])
            state.update(task_uuid=task['id'],revision=task['revision'])
        state.update(phase='requested',scope=prepared_scope(root,owner,REASON,'agent'),
                     checkout=_checkout(root),adapter=_adapter(),updated=time.time())
        _write(root,key,state)
        full=configured_instruction(root)=='full' or not _rules_in_context(payload)
        instruction=_instruction(root,key,state,brief=brief,host=host,full=full)
    # Claude Code labels a block a hook error, notifies one after every
    # answer and hands the model the reason twice. Its Stop additionalContext
    # (2.1.163 and later) continues the turn as hook feedback, once, through
    # the same stop_hook_active and continuation cap.
    if host=='claude': output={'hookSpecificOutput':{'hookEventName':'Stop','additionalContext':instruction}}
    elif host=='cursor': output={'followup_message':instruction}
    else: output={'decision':'block','reason':instruction}
    return step(output,'requested',code,answer='brief' if brief else 'repeat')


def save(root,key,token,data,*,owner,mode,provision,skip=False):
    with _Step(root,'save',key=key) as step:
        return _save(step,root,key,token,data,owner=owner,mode=mode,provision=provision,skip=skip)


def _save(step,root,key,token,data,*,owner,mode,provision,skip):
    if not _enabled(mode): raise BrainError('native-result:governed-session-required')
    if not isinstance(token,str) or not TOKEN.fullmatch(token): raise BrainError('native-result:invalid-token')
    with mutation_lock(root):
        state=_read(root,key)
        if state is not None: step.host=state['host']
        if (state is None or state['token'] != token or state['owner'] != owner
                or state['project'] != _project(root)
                or state['phase'] not in {'requested','saved','held','failed'}):
            raise BrainError('native-result:request-unavailable')
        begun=state['digest'] is not None
        # For the journal: a repeat of a finished save, or a begun save
        # finished after the recovery Stop marked it failed.
        repeat={'saved':'repeat','held':'repeat','failed':'late'}.get(state['phase'])
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
            return step({'state':'omitted','attestation':'agent'},'omitted')
        # A completed replay may outlive its sources or active task alias.
        # Only the canonical result protocol can admit such a replay; local
        # challenge metadata alone must never authorize a fresh write.
        normalized=normalize(root,data,check_files=not begun)
        if (state['task_uuid'] is None and normalized['progress'] is None
                and not normalized['next_steps'] and not normalized['learnings']):
            error=BrainError('native-result:nothing-to-record: no task yet and nothing to start one with; give a '
                             'progress summary or a learning, or run the command with --skip')
            error.code='native-result:nothing-to-record'
            raise error
        if state['task_uuid'] is None:
            try: writable_task(root,state['task_id'],owner)
            except BrainRecordNotFound as error:
                if error.identifier != state['task_id']: raise
            else: raise _overtaken('native-result:task-changed','the branch task was created')
            provision(state['task_id'])
            task=writable_task(root,state['task_id'],owner)
            state.update(task_uuid=task['id'],revision=task['revision'])
            _write(root,key,state)
        if not begun:
            task=writable_task(root,state['task_id'],owner)
            if task['id'] != state['task_uuid']:
                raise _overtaken('native-result:task-changed','the branch task was replaced')
        digest=request_digest(state['task_uuid'],normalized)
        if state['digest'] is not None and state['digest'] != digest:
            raise BrainError('native-result:content-changed')
        state['digest']=digest;_write(root,key,state)
        try:
            result=record_result(root,state['task_uuid'],'native-'+token,state['revision'],data,
                                 owner=owner,reason=REASON,attestation='agent',prepared=state['scope'],
                                 replay_only=begun)
        except BrainError as error:
            if str(error) != 'memory-result-v1:prepared_stale_no_write': raise
            raise _overtaken(str(error),'another session or a person saved the task') from error
        held=any(item['state'] in {'differs','archived-differs'} for item in result['records'])
        state['phase']='held' if held else 'saved';_write(root,key,state)
        return step(result,state['phase'],repeat,
                    learnings=sum(item['state'] in {'created','completed','updated'} for item in result['records']),
                    promoted=len((result['promotion'] or {}).get('promoted') or ()))
