"""One bounded native Stop continuation; only structured, guarded results persist.

No transcript or assistant prose is parsed into knowledge. The same agent is
asked once to submit its result through memory_results. Local challenges are
metadata only, disposable, scoped to one project/owner/host/session and prompt.
Every prompt, Stop and save also leaves one metadata line in a local journal,
so how far the saves get can be counted from disk (see _journal).
"""
from __future__ import annotations

import errno
import hashlib
import json
import math
import os
from pathlib import Path
from itertools import islice
import re
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile
import time
from uuid import uuid4

import workspace_roots
from brain_runtime import (BrainError, BrainRecordNotFound, atomic_json, atomic_write_bytes,
                           brain_root, linked_reparse, load_config, mutation_lock)
from memory_results import (PROOF_PROFILE, _text, _attempt, _read_attempt, _read_receipt,
                            check_task_id, normalize, prepared_scope, record_result,
                            refusal_code, request_digest, writable_task, learning_id)

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
# A save the agent's sandbox cannot write to the project - Codex started in a
# subdirectory writes only below it - waits in a folder of this user's in a
# temporary directory such a sandbox lets a command write, until the
# session's next hook, which runs outside the sandbox, saves it.
SPOOL_PROFILE = 'native-result-spool-v1'
SPOOL_NAME = 'ai-accelerator-native-spool'
# How many hooks may start one spooled save before it is given up (drain).
DRAIN_ATTEMPTS = 3
UNWRITABLE = frozenset({errno.EROFS, errno.EACCES, errno.EPERM})
# A fully exercised turn writes about six lines - armed; requested and the
# Stop hook's exit; the save; the closing Stop and its exit - and Codex one
# more for its continuation prompt. Eight per refresh-health record keeps at
# least the turns that series keeps, so both read over one window.
JOURNAL_LINES_PER_TURN = 8
# Trimmed back to the kept lines once the file passes this many bytes a kept
# line: an append stays one write, and a trim is rare.
JOURNAL_LINE_BYTES = 256


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
    """The refusal of a save another writer overtook in a way it cannot
    follow: the branch task replaced since the request, a task created or
    moved on since a request older than IN_FLIGHT_SECONDS, or a begun save
    whose task moved on before it finished. The bare code left the agent to
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
# A prompt sent while the agent still writes the save moves the pending
# request aside when it was asked this recently (see _begin); an older one
# is reported unsaved at once.
IN_FLIGHT_SECONDS = 10 * 60


# The rules a save step carries - the schema, what belongs in learnings - are
# the same after every answer: about 1,150 of its 2,700 characters, which
# Claude Code's terminal also shows whole each time. A step whose
# conversation still holds a full one carries the command and points back to
# it. The full step is sent again once a compaction has dropped it, or when
# the transcript cannot say. `full` sends it every time;
# CONTEXT_NATIVE_RESULT_INSTRUCTION overrides `native_result_instruction`.
NATIVE_INSTRUCTIONS = ('auto', 'full')
FULL_RULES = 'Native result reference schema v1.'
RULES_TAIL_BYTES = 2 * 1024 * 1024
COMPACTION_RECORDS = (b'"subtype":"compact_boundary"', b'"type":"compacted"')


def configured_instruction(root):
    """Whether a save step may point back to the full one; an unknown value sends it whole."""
    value = (os.environ.get('CONTEXT_NATIVE_RESULT_INSTRUCTION')
             or load_config(root).get('native_result_instruction') or 'auto')
    return value if value in NATIVE_INSTRUCTIONS else 'full'


def _rules_marker(root):
    policy='proposal-only' if maintenance_mode(root)=='review-verified' else 'ordinary'
    return FULL_RULES+' Capture policy '+policy+'.'


def _rules_in_context(payload, marker):
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
    latest=tail.rfind(FULL_RULES.encode(),begin)
    # Legacy markers, or a newer full step from another capture policy, must
    # not lend their ordinary/proposal-only restrictions to the current turn.
    return latest>=0 and tail.startswith(marker.encode(),latest)


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
    """The Codex turn behind this Stop: its collaboration mode, sandbox type
    and restricted file-system policy from the rollout's turn_context record
    with the Stop's turn_id, or Nones. Codex's permission_mode follows the
    approval policy only."""
    path, turn = payload.get('transcript_path'), payload.get('turn_id')
    if not isinstance(path, str) or not path or not isinstance(turn, str) or not turn:
        return None, None, None
    try:
        if not stat.S_ISREG(os.stat(path).st_mode):
            return None, None, None
        with open(path, 'rb') as handle:
            handle.seek(0, os.SEEK_END)
            handle.seek(max(0, handle.tell() - ROLLOUT_TAIL_BYTES))
            tail = handle.read(ROLLOUT_TAIL_BYTES)
    except OSError:
        return None, None, None
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
            return None, None, None
        mode = context.get('collaboration_mode')
        sandbox = context.get('sandbox_policy')
        mode = mode.get('mode') if isinstance(mode, dict) else None
        sandbox = sandbox.get('type') if isinstance(sandbox, dict) else None
        files = context.get('file_system_sandbox_policy')
        files = files if isinstance(files, dict) and files.get('kind') == 'restricted' else None
        # Read as not letting the turn write its own directory, the policy is
        # in a form this reading does not know: the Stop asks as before.
        where = context.get('cwd')
        if files is not None and not (isinstance(where, str) and where
                                      and _sandbox_writes(files, os.path.realpath(where))):
            files = None
        return (mode if isinstance(mode, str) else None), (sandbox if isinstance(sandbox, str) else None), files
    return None, None, None


def _sandbox_writes(policy, target):
    """Whether a Codex restricted file-system policy lets a command write
    `target`: the most specific entry over it decides, as it decides Codex's
    own carve-outs (.git, .codex and .agents stay read-only inside a writable
    workspace)."""
    target = os.path.normcase(target)
    best = None
    for entry in policy.get('entries') or ():
        if not isinstance(entry, dict) or entry.get('access') not in ('read', 'write', 'none'):
            continue
        where = entry.get('path')
        if not isinstance(where, dict):
            continue
        if where.get('type') == 'path' and isinstance(where.get('path'), str):
            base = os.path.realpath(where['path'])
        elif where.get('type') == 'special' and isinstance(where.get('value'), dict):
            # The temporary directory: $TMPDIR, or on Windows, where it is
            # rarely set, the user's own (TEMP, TMP) - where _spool_dirs waits.
            temporary = os.environ.get('TMPDIR') or (tempfile.gettempdir() if os.name == 'nt' else None)
            base = {'root': os.sep, 'slash_tmp': '/tmp',
                    'tmpdir': temporary}.get(where['value'].get('kind'))
            if not base:
                continue
            base = os.path.realpath(base)
        else:
            continue
        base = os.path.normcase(base)
        if target == base or target.startswith(base.rstrip(os.sep) + os.sep):
            if best is None or len(base) > best[0]:
                best = (len(base), entry['access'])
    return best is not None and best[1] == 'write'


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


def _read(root, key, slot=None):
    """The session's request state, from its own file or from `slot`, where
    _begin moves a request aside; its identity is the session's either way."""
    path = _file(root, slot or key)
    value = _read_receipt(path)
    if value is None:
        if os.path.lexists(path):
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


def _aside(key):
    """The file a session's pending request moves to when a prompt arrives
    while its save is written (see _begin)."""
    return hashlib.sha256(('aside:'+key).encode()).hexdigest()


def _read_aside(root, key):
    try:
        return _read(root, key, _aside(key))
    except BrainError:
        return None


def _drop_aside(root, key):
    try:
        path = _file(root, _aside(key))
        if os.path.lexists(path): path.unlink()
    except (BrainError, OSError):
        pass


def _prune(root, key):
    directory = _file(root,key).parent
    candidates=[]
    for item in directory.glob('*.json'):
        if HEX.fullmatch(item.stem) and item.stem not in (key, _aside(key)):
            info=item.lstat()
            if stat.S_ISREG(info.st_mode) and info.st_nlink == 1 and not linked_reparse(info):
                candidates.append((info.st_mtime_ns,item))
    for _,item in sorted(candidates)[:max(0,len(candidates)-(MAX_STATES-1))]:
        item.unlink()


def begin(root, task_id, host, payload, *, owner, mode):
    with _Step(root,'prompt',host) as step:
        return _begin(step,root,task_id,host,payload,owner=owner,mode=mode)


def _armed(root, host, payload, task_id, owner):
    """A request armed for this turn, not yet asked for."""
    return {'profile':PROFILE,'owner':owner,'host':host,
            'session':hashlib.sha256(str(payload.get('conversation_id' if host=='cursor' else 'session_id')).encode()).hexdigest(),
            'project':_project(root),'task_id':task_id,'token':uuid4().hex,
            'phase':'armed','task_uuid':None,'revision':None,'scope':None,'digest':None,
            'checkout':None,'adapter':None,'updated':time.time()}


def _begin(step, root, task_id, host, payload, *, owner, mode):
    task_id=check_task_id(task_id)
    if not _enabled(mode): return step({'armed':False},'skipped',_disabled(mode))
    if configured_request(root) == 'off': return step({'armed':False},'skipped','switched-off')
    key=_key(root,host,payload,task_id,owner)
    if key is None: return step({'armed':False},'skipped','no-session-id')
    with mutation_lock(root):
        # A state this runtime cannot read - written by another version of
        # it, or damaged - refused every later prompt of the session, which
        # then saved nothing more. Its request can no longer be settled: a
        # new one replaces it.
        try: previous,unreadable=_read(root,key),False
        except BrainError as error:
            if refusal_code(error) != 'native-result:invalid-state': raise
            previous,unreadable=None,True
        aside=_read_aside(root,key)
        # Codex submits the Stop reason through UserPromptSubmit too. It is
        # bookkeeping for this prompt, never a new generation of user work.
        prompt=payload.get('prompt')
        for pending in (previous,aside):
            if (pending and pending['phase']=='requested' and isinstance(prompt,str)
                    and prompt.lstrip().startswith('INTERNAL_NATIVE_MEMORY_RESULT '+pending['token'])):
                return step({'armed':False,'continuation':True},'continuation')
        # A request moved aside at the last prompt had the rest of its turn to
        # save; what it became is reported now, as for any other request.
        unsettled=None
        if aside is not None:
            _drop_aside(root,key)
            if (aside['phase'] in {'requested','failed'} and host=='claude' and not aside['digest']
                    and _claude_refusal(payload,aside['token'])):
                aside.update(phase='refused',updated=time.time())
            if aside['phase'] == 'refused' and (previous is None or previous['phase'] == 'armed'):
                previous=aside
            elif aside['phase'] in {'requested','failed','held','refused'}:
                unsettled=aside['phase']
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
        # A person may send the next prompt while the agent still writes the
        # save: Claude Code and Codex take it into the running turn and run
        # this hook at once. Replaced, the request refused that save on its
        # way and the agent was warned of a result it was saving; moved
        # aside, the save lands.
        moved=(previous is not None and previous['phase']=='requested'
               and 0 <= time.time()-previous['updated'] < IN_FLIGHT_SECONDS)
        if moved: _write(root,_aside(key),previous)
        warning = bool(unsettled) or not moved and bool(previous and previous['phase'] in {'requested','failed','held','refused'})
        _write(root,key,_armed(root,host,payload,task_id,owner));_prune(root,key)
    # The warning is counted by what the last request became.
    return step({'armed':True,'request':key,**({'warning':'Previous native memory result was not saved completely.'} if warning else {})},
                'armed','previous-'+(unsettled or previous['phase']) if warning
                else 'previous-in-flight' if moved else 'previous-unreadable' if unreadable else None)


def recovery(host, payload):
    return (type(payload.get('loop_count')) is int and payload['loop_count'] > 0
            if host=='cursor' else payload.get('stop_hook_active') is True)


# The thread that owns a worktree Codex made, read from codex-thread.json of
# its Git directory by the patterns context.py's _owner_thread() reads it by.
_CODEX_THREAD_VERSION=re.compile(rb'"version"[ \t\n\v\f\r]*:[ \t\n\v\f\r]*1[ \t\n\v\f\r]*[,}]')
_CODEX_THREAD_OWNER=re.compile(
    rb'"ownerThreadId"[ \t\n\v\f\r]*:[ \t\n\v\f\r]*"([0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12})"')


def _codex_owner(git_directory):
    try:
        record=Path(os.fsdecode(git_directory))/'codex-thread.json'
        if record.is_symlink() or not record.is_file(): return None
        with record.open('rb') as handle: raw=handle.read(4097)
    except (OSError,ValueError):
        return None
    owner=_CODEX_THREAD_OWNER.search(raw)
    if len(raw) > 4096 or _CODEX_THREAD_VERSION.search(raw) is None or owner is None: return None
    return owner.group(1)


def _checkout(root):
    try:
        result=subprocess.run(['git','-C',str(workspace_roots.project_root(root)),
            'rev-parse','--absolute-git-dir','HEAD','--symbolic-full-name','HEAD'],stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,timeout=2,check=False)
    except (OSError,subprocess.TimeoutExpired) as error:
        raise BrainError('native-result:checkout-unavailable') from error
    # A project without Git has no checkout identity to pin. The project
    # root still participates in the challenge's identity and prepared scope.
    if result.returncode != 0: return hashlib.sha256(b'no-git-head').hexdigest()
    # The branch the save was asked on, not its commit: a commit made while
    # the agent wrote the save - the person's, or another session's on the
    # branch - refused it, learnings and all. A worktree Codex made stays
    # detached, and the thread that owns it names the task: its commits move
    # HEAD and leave the task as it was, so the thread is pinned. Detached
    # otherwise, as in the middle of a rebase, the commit is all there is.
    directory,_,head=result.stdout.partition(b'\n')
    _,_,name=head.partition(b'\n')
    if name.startswith(b'refs/heads/'): return hashlib.sha256(name.strip()).hexdigest()
    owner=_codex_owner(directory.strip())
    return hashlib.sha256(b'codex-thread/'+owner if owner else head).hexdigest()


def _adapter():
    return hashlib.sha256(Path(__file__).read_bytes()).hexdigest()


# Encounter hints are disposable, task/actor scoped and never attest a check.
# No manifest scan, bank audit or search runs in the Stop hook.
MAINTENANCE_PROFILE = 'native-memory-encounters-v1'
MAINTENANCE_LIMIT = 3
MAINTENANCE_TTL = 3600


def maintenance_mode(root):
    value = os.environ.get('CONTEXT_NATIVE_MEMORY_MAINTENANCE')
    return value if value is not None else load_config(root).get('native_memory_maintenance', 'off')


def maintenance_enabled(root):
    value=maintenance_mode(root)
    return isinstance(value,str) and value in {'encountered', 'review', 'review-verified'}


def _maintenance_file(root, owner, task_id):
    key = hashlib.sha256(json.dumps([_project(root), owner, check_task_id(task_id)]).encode()).hexdigest()
    directory = brain_root(root) / 'local' / 'native-maintenance'
    for path in (directory.parent.parent, directory.parent, directory):
        if os.path.lexists(path):
            image = path.lstat()
            if not stat.S_ISDIR(image.st_mode) or linked_reparse(image):
                raise BrainError('native-result:unsafe-maintenance-storage')
    path = directory / (key + '.json')
    if os.path.lexists(path):
        image = path.lstat()
        if not stat.S_ISREG(image.st_mode) or image.st_nlink != 1 or linked_reparse(image):
            raise BrainError('native-result:unsafe-maintenance-storage')
    return path


def _maintenance_image(root, name):
    from pathlib import PurePosixPath
    if (not isinstance(name, str) or len(name) > 1024 or '\\' in name
            or str(PurePosixPath(name)) != name or '..' in PurePosixPath(name).parts
            or not re.fullmatch(r'(?:memory-bank/chunks/[^/]+|project-brain/dynamic/(?:findings|decisions)/[^/]+)\.md', name)):
        raise BrainError('native-result:invalid-maintenance-source')
    path = workspace_roots.resolve(root, name)
    for parent in (path, *list(path.parents)[:len(PurePosixPath(name).parts)-1]):
        image = parent.lstat()
        if parent.is_symlink() or linked_reparse(image):
            raise BrainError('native-result:unsafe-maintenance-source')
    descriptor = os.open(path, os.O_RDONLY | getattr(os, 'O_NOFOLLOW', 0) | getattr(os, 'O_NONBLOCK', 0) | getattr(os, 'O_BINARY', 0))
    try:
        image = os.fstat(descriptor)
        if not stat.S_ISREG(image.st_mode) or image.st_nlink != 1 or image.st_size > 65536:
            raise BrainError('native-result:invalid-maintenance-source')
        with os.fdopen(descriptor, 'rb', closefd=False) as handle:
            raw = handle.read(65537)
        after=os.fstat(descriptor);named=path.lstat()
        identity=lambda value:(value.st_dev,value.st_ino,value.st_size,value.st_mtime_ns,workspace_roots.change_time_ns(value),value.st_mode,value.st_nlink)
        if identity(image)!=identity(after) or identity(after)!=identity(named):
            raise BrainError('native-result:maintenance-source-changed')
    finally:
        os.close(descriptor)
    if len(raw) > 65536: raise BrainError('native-result:invalid-maintenance-source')
    return raw.decode('utf-8').replace('\r\n', '\n'), hashlib.sha256(raw.replace(b'\r\n', b'\n')).hexdigest()


def observe_maintenance(root, capsule, task_id, *, owner):
    """Remember at most three final returned references, not proof of reading.

    Same actor/task scope deliberately permits a later session of that task.
    A new retrieval replaces this disposable packet; publication/read failures
    never affect ordinary context or saves. No task is created for a hint.
    """
    try:
        if not maintenance_enabled(root) or not task_id or not isinstance(capsule, dict): return
        if not capsule.get('manifest') or capsule.get('manifest_publication') == 'unconfirmed': return
        _text(owner, 'owner', 200)
        from context_retrieval import _capsule_index_images
        images = _capsule_index_images(capsule)
        if images is None: return
        paths = {row.get('path') for row in capsule.get('semantic', []) if isinstance(row, dict)}
        rows = []
        for name, indexed in sorted(images.items()):
            if name not in paths: continue
            try:
                _, digest = _maintenance_image(root, name)
                if not isinstance(indexed, tuple) or hashlib.sha256(indexed[2].encode('utf-8')).hexdigest() != digest: continue
                rows.append({'path': name, 'sha256': digest})
            except (BrainError, OSError, ValueError, UnicodeError): continue
            if len(rows) == MAINTENANCE_LIMIT: break
        if not rows: return
        with mutation_lock(root):
            path = _maintenance_file(root, owner, task_id)
            atomic_json(path, {'profile': MAINTENANCE_PROFILE, 'at': time.time(),
                              'task_uuid': capsule.get('task_uuid'), 'rows': rows})
            # Storage is bounded separately from native challenge files.
            others = list(path.parent.glob('*.json'))
            if len(others) > MAX_STATES:
                for old in sorted(others, key=lambda item: item.lstat().st_mtime_ns)[:len(others)-MAX_STATES]:
                    if old != path and HEX.fullmatch(old.stem) and not old.is_symlink(): old.unlink()
    except (BrainError, OSError, ValueError, TypeError, KeyError, UnicodeError):
        return


def maintenance_candidates(root, state):
    """Current admission of old encounter images; not an automatic truth check."""
    try:
        if not maintenance_enabled(root): return []
        from brain_runtime import parse_markdown_record, record_attestation, canonical_source_image
        from memory_results import checked_fingerprints, CLOSED_STATE
        from validate import indexed_secret_policy_reason, memory_temporal_reason
        value = _read_receipt(_maintenance_file(root, state['owner'], state['task_id']))
        if (not isinstance(value, dict) or set(value) != {'profile', 'at', 'task_uuid', 'rows'}
                or value['profile'] != MAINTENANCE_PROFILE or type(value['at']) not in (int, float)
                or not math.isfinite(value['at']) or not 0 <= time.time()-value['at'] <= MAINTENANCE_TTL
                or value['task_uuid'] is not None and value['task_uuid'] != state['task_uuid']
                or not isinstance(value['rows'], list) or not 1 <= len(value['rows']) <= MAINTENANCE_LIMIT): return []
        config = load_config(root); hints = []; seen = set()
        for row in value['rows']:
            try:
                if not isinstance(row, dict) or set(row) != {'path', 'sha256'}: continue
                text, digest = _maintenance_image(root, row['path'])
                if digest != row['sha256']: continue
                if indexed_secret_policy_reason(text): continue
                record_path = row['path']
                metadata, _ = parse_markdown_record(Path(row['path']), text=text)
                if row['path'].startswith('memory-bank/'):
                    if metadata.get('status') != 'active' or memory_temporal_reason(metadata): continue
                    links = [pin for pin in metadata.get('source_digests', [])
                             if isinstance(pin, dict) and isinstance(pin.get('path'), str)
                             and pin['path'].startswith(('project-brain/dynamic/findings/', 'project-brain/dynamic/decisions/'))]
                    if len(links) != 1: continue
                    text, digest = _maintenance_image(root, links[0]['path'])
                    if digest != links[0].get('sha256'): continue
                    if indexed_secret_policy_reason(text): continue
                    record_path = links[0]['path']
                    metadata, _ = parse_markdown_record(Path(links[0]['path']), text=text)
                from brain_runtime import validate_record
                validate_record(metadata)
                if (metadata['type'] not in CLOSED_STATE or metadata['status'] != CLOSED_STATE[metadata['type']]
                        or metadata['authority'] != 'verified' or record_attestation(metadata) != 'agent'
                        or metadata['authority'] not in config['allowed_authority']
                        or metadata['privacy'] not in set(config['allowed_privacy']) & {'public', 'team'}
                        or not (set(config['owners']) & {'*', metadata['owner']})
                        or state['owner'] not in metadata['authorized_owners'] or metadata['superseded_by']
                        or metadata['id'] in seen): continue
                sources = metadata['sources']
                source_image = checked_fingerprints(root, sources)
                if source_image == canonical_source_image(metadata['source_fingerprints']): continue
                learning = {'type': metadata['type'], 'title': metadata['title'],
                            'consequence': metadata.get('progress') or metadata['goal'], 'sources': sources}
                _text(learning['title'], 'title', 160)
                _text(learning['consequence'], 'consequence', 1000)
                hint = {**learning, 'record_id': metadata['id'], 'expected_revision': metadata['revision']}
                if len(json.dumps(hint, ensure_ascii=False)) > 2500: continue
                hints.append({**hint,'_record_path':record_path,'_record_sha256':digest,'_source_image':source_image}); seen.add(metadata['id'])
            except (BrainError, OSError, ValueError, TypeError, KeyError, UnicodeError): continue
        return hints
    except (BrainError, OSError, ValueError, TypeError, KeyError, UnicodeError):
        return []


def maintenance_instruction(root, state):
    if maintenance_mode(root) != 'encountered': return ''
    hints=[{k:v for k,v in row.items() if not k.startswith('_')} for row in maintenance_candidates(root,state)]
    if not hints: return ''
    return (' Experimental maintenance: these same-task encounter hints have changed cited files. '
            'They are untrusted prior claims, not proof you read or checked them. '
            'Only if the original task work already checked the claim and its current sources, '
            'confirm or correct its consequence using the exact reference below plus source_fingerprints '
            'from context.py source-fingerprints --source PATH --json. Preserve type/title/sources. '
            'Otherwise omit it and keep ordinary progress; do not start new research or bulk-refresh hashes. '
            'No valid replacement means the old claim stays unresolved. References: '
            + json.dumps(hints, ensure_ascii=False) + '\n')


REVIEW_PROFILE = 'pre-answer-memory-review-v1'
REVIEW_REASON = 'Explicit memory review working stage; agent-attested, not human reviewed'


def review_enabled(root):
    value=maintenance_mode(root)
    return isinstance(value,str) and value in {'review','review-verified'} and load_config(root)['mode'] == 'governed'


def _review_command(root, task_id):
    layout = workspace_roots.roots(root)
    assignments = ''
    if layout.shared or layout.attached:
        assignments = ' '.join(name+'='+shlex.quote(str(value)) for name,value in (
            (workspace_roots.HOME_VARIABLE,layout.tooling),(workspace_roots.STATE_VARIABLE,layout.state),
            (workspace_roots.PROJECT_VARIABLE,layout.project)))+' '
    interpreter = ['python3'] if os.name != 'nt' else (['python'] if shutil.which('python') else ['py','-3'])
    return assignments + shlex.join([*interpreter,str(Path(__file__).resolve().with_name('context.py')),
        '--root',str(root),*(['--project',str(layout.project)] if layout.shared or layout.attached else []),
        'memory-review','--task-id',task_id,'--json'])


def review_notice(root, task_id, *, owner):
    """Workflow instruction, not a host-level gate on an already visible answer."""
    try:
        if not review_enabled(root) or not task_id: return None
        task = writable_task(root, check_task_id(task_id), owner)
        value = _read_receipt(_maintenance_file(root, owner, task_id))
        if not value or not value.get('rows') or task['status'] in {'completed', 'cancelled'}: return None
        command = _review_command(root,task_id)
        if maintenance_mode(root)=='review-verified':
            return ('MEMORY_REVIEW_STAGE: after implementing and testing, BEFORE your final answer, run '
                    +command+'. Inspect each claim and its current sources; explicitly confirm, correct or defer every item. '
                    'memory-review --input - stages a proposal ONLY. Independent verification and publication are performed '
                    'by the external controller after this session. Do not write reusable knowledge through other commands. '
                    'The native closing save must use learnings=[] and ordinary progress. Report pending review, never claim publication.')
        return ('MEMORY_REVIEW_STAGE: after implementing and testing, BEFORE your final answer, run '
                + command + '. Read each offered claim and its current cited sources; explicitly confirm, '
                'correct or defer every item using memory-review --input -. This sanctioned working-stage '
                'command applies checked references; the later native Stop still saves your answer progress. '
                'If unavailable, report deferred review. Hashes alone are not verification.')
    except (BrainError, OSError, ValueError, TypeError):
        return None


def _review_file(root, owner, task_id, identifier, *, receipt=False):
    if not isinstance(identifier, str) or not TOKEN.fullmatch(identifier):
        raise BrainError('memory-review:invalid-plan')
    directory = _maintenance_file(root, owner, task_id).parent / 'reviews'
    if os.path.lexists(directory):
        image = directory.lstat()
        if not stat.S_ISDIR(image.st_mode) or linked_reparse(image):
            raise BrainError('memory-review:unsafe-storage')
    result = directory / (identifier + ('-result' if receipt else '') + '.json')
    if os.path.lexists(result):
        image = result.lstat()
        if not stat.S_ISREG(image.st_mode) or image.st_nlink != 1 or linked_reparse(image):
            raise BrainError('memory-review:unsafe-storage')
    return result


def _review_read(source, *, limit=65536):
    """Bounded local plans are larger than the 4 KiB result-proof envelope."""
    if not os.path.lexists(source): return None
    descriptor = os.open(source,os.O_RDONLY|getattr(os,'O_NOFOLLOW',0)|getattr(os,'O_NONBLOCK',0)|getattr(os,'O_BINARY',0))
    try:
        before=os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode) or before.st_nlink!=1 or before.st_size>limit: return None
        with os.fdopen(descriptor,'rb',closefd=False) as handle:raw=handle.read(limit+1)
        after=os.fstat(descriptor);named=source.lstat()
        identity=lambda image:(image.st_dev,image.st_ino,image.st_size,image.st_mtime_ns,workspace_roots.change_time_ns(image),image.st_mode,image.st_nlink)
        if len(raw)>limit or identity(before)!=identity(after) or identity(after)!=identity(named): return None
        def unique(pairs):
            value={}
            for key,item in pairs:
                if key in value:raise ValueError('duplicate key')
                value[key]=item
            return value
        def constant(_):raise ValueError('nonfinite number')
        try:value=json.loads(raw,object_pairs_hook=unique,parse_constant=constant)
        except (ValueError,UnicodeError,RecursionError):return None
        return value if isinstance(value,dict) else None
    finally:
        os.close(descriptor)


def memory_review_plan(root, task_id, *, owner):
    if not review_enabled(root): raise BrainError('memory-review:disabled')
    task_id = check_task_id(task_id)
    with mutation_lock(root):
        task = writable_task(root, task_id, owner)
        if task['status'] in {'completed','cancelled'}: raise BrainError('memory-review:task-closed')
        candidates = maintenance_candidates(root, {'owner':owner,'task_id':task_id,'task_uuid':task['id']})
        if not candidates:
            return {'profile':REVIEW_PROFILE,'state':'unavailable','plan_id':None,'records':[],
                    'reason':'No safely admitted changed-source encounter; no claim was verified.'}
        identifier = uuid4().hex
        plan = {'profile':REVIEW_PROFILE,'plan_id':identifier,'owner':owner,'project':_project(root),
                'task_id':task_id,'task_uuid':task['id'],'task_revision':task['revision'],'at':time.time(),
                'scope':prepared_scope(root,owner,REVIEW_REASON,'agent'),'adapter':_adapter(),
                'checkout':_checkout(root),'records':candidates}
        if maintenance_mode(root)=='review-verified':plan['verification_required']=True
        destination = _review_file(root,owner,task_id,identifier)
        if len(json.dumps(plan,ensure_ascii=False,sort_keys=True,indent=2).encode('utf-8'))>65535:
            raise BrainError('memory-review:plan-too-large')
        atomic_json(destination, plan)
        # Bounded local replay history; authoritative result proofs are not pruned.
        from itertools import islice
        old = list(islice(destination.parent.glob('*.json'), 2*MAX_STATES+1))
        if len(old) > 2*MAX_STATES:
            for item in sorted(old,key=lambda p:p.lstat().st_mtime_ns)[:len(old)-2*MAX_STATES]:
                if item != destination and not item.is_symlink(): item.unlink()
        return {'profile':REVIEW_PROFILE,'state':'review-required','plan_id':identifier,
                'verification_required':bool(plan.get('verification_required')),
                'task_id':task_id,'task_uuid':task['id'],'records':[
                    {k:v for k,v in row.items() if not k.startswith('_')} for row in candidates],
                'instructions':'Open every claim and necessary current cited source now. Submit one decision per record. '
                    'confirm/correct require top-level verified=true and source_fingerprints from checked sources; '
                    'correct additionally supplies consequence. defer requires a short reason and does not renew knowledge. '
                    'Preserve exact type/title/sources. A title or source-set change requires separate review.',
                'submit_command':_review_command(root,task_id)+' --input -',
                'submit_schema':{'plan_id':identifier,'verified':True,
                    'decisions':'one object for every offered record; use the exact field set below'},
                'decision_fields':{'confirm':['record_id','action','source_fingerprints'],
                    'correct':['record_id','action','source_fingerprints','consequence'],
                    'defer':['record_id','action','reason']}}


def _validate_review_plan(root,plan,task_id,identifier,owner,task_uuid):
    plan_fields={'profile','plan_id','owner','project','task_id','task_uuid','task_revision','at',
                 'scope','adapter','checkout','records'}
    if isinstance(plan,dict) and plan.get('verification_required') is True:plan_fields.add('verification_required')
    if (not isinstance(plan,dict) or set(plan)!=plan_fields
            or type(plan.get('task_revision')) is not int or plan['task_revision']<1
            or type(plan.get('at')) not in (int,float) or not math.isfinite(plan['at']) or plan['at']<0
            or plan.get('profile') != REVIEW_PROFILE or plan.get('plan_id') != identifier
            or plan.get('owner') != owner or plan.get('project') != _project(root) or plan.get('task_id') != task_id
            or not isinstance(plan.get('task_uuid'),str)
            or not re.fullmatch(r'[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}',plan['task_uuid'])
            or plan.get('task_uuid') != task_uuid or plan.get('scope') != prepared_scope(root,owner,REVIEW_REASON,'agent')
            or plan.get('adapter') != _adapter() or plan.get('checkout') != _checkout(root)
            or not isinstance(plan.get('records'),list) or not 1 <= len(plan['records']) <= MAINTENANCE_LIMIT):
        raise BrainError('memory-review:plan-unavailable-or-scope-changed')
    reference_fields={'type','title','consequence','sources','record_id','expected_revision',
                      '_record_path','_record_sha256','_source_image'}
    identifiers=set()
    for row in plan['records']:
        if (not isinstance(row,dict) or set(row)!=reference_fields
                or not isinstance(row['record_id'],str)
                or not re.fullmatch(r'[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}',row['record_id'])
                or row['record_id'] in identifiers or type(row['expected_revision']) is not int or row['expected_revision']<1
                or not isinstance(row['_record_path'],str) or not isinstance(row['_record_sha256'],str)
                or not HEX.fullmatch(row['_record_sha256'])):
            raise BrainError('memory-review:plan-unavailable')
        normalize(root,{'progress':None,'verified':True,'learnings':[
            {**{k:row[k] for k in ('type','title','consequence','sources','record_id','expected_revision')},
             'source_fingerprints':row['_source_image']}]},check_files=False)
        identifiers.add(row['record_id'])
    return {r['record_id']:r for r in plan['records']}


def memory_review_apply(root, task_id, data, *, owner):
    """Explicit semantic work boundary; native save wire/replay is unchanged."""
    if not review_enabled(root): raise BrainError('memory-review:disabled')
    if (not isinstance(data,dict) or set(data)-{'plan_id','verified','decisions'}
            or not {'plan_id','decisions'} <= set(data) or type(data.get('verified',False)) is not bool):
        raise BrainError('memory-review:invalid-decisions')
    task_id = check_task_id(task_id);identifier = data['plan_id']
    from brain_runtime import canonical_source_image
    from memory_results import checked_fingerprints, request_digest
    with mutation_lock(root):
        task = writable_task(root,task_id,owner)
        plan = _review_read(_review_file(root,owner,task_id,identifier))
        offered = _validate_review_plan(root,plan,task_id,identifier,owner,task['id'])
        if maintenance_mode(root)=='review-verified' and plan.get('verification_required') is not True:
            raise BrainError('memory-review:verification-required')
        decisions=data['decisions']
        if not isinstance(decisions,list) or len(decisions)!=len(offered): raise BrainError('memory-review:decide-every-record')
        checked=[];deferred=[];seen=set()
        for decision in decisions:
            if (not isinstance(decision,dict) or not isinstance(decision.get('record_id'),str)
                    or decision['record_id'] not in offered or decision['record_id'] in seen
                    or not isinstance(decision.get('action'),str)):
                raise BrainError('memory-review:invalid-or-duplicate-record')
            seen.add(decision['record_id']);row=offered[decision['record_id']];action=decision.get('action')
            if action=='defer':
                if set(decision)!={'record_id','action','reason'}:raise BrainError('memory-review:invalid-defer')
                deferred.append({'record_id':row['record_id'],'reason':_text(decision['reason'],'defer reason',300)})
                continue
            fields={'record_id','action','source_fingerprints'} | ({'consequence'} if action=='correct' else set())
            if action not in {'confirm','correct'} or set(decision)!=fields or data.get('verified') is not True:
                raise BrainError('memory-review:checked-agent-attestation-required')
            pins=canonical_source_image(decision['source_fingerprints'])
            if pins != row['_source_image']: raise BrainError('memory-review:checked-image-does-not-match-plan')
            checked.append({k:row[k] for k in ('type','title','consequence','sources','record_id','expected_revision')})
            checked[-1]['source_fingerprints']=pins
            if action=='correct':checked[-1]['consequence']=_text(decision['consequence'],'consequence',1000)
        payload={'progress':None,'learnings':checked,'verified':True}
        normalized=normalize(root,payload,check_files=False) if checked else None
        incoming=hashlib.sha256(json.dumps(data,sort_keys=True,ensure_ascii=False,allow_nan=False).encode()).hexdigest()
        destination=_review_file(root,owner,task_id,identifier,receipt=True)
        receipt=_review_read(destination)
        if os.path.lexists(destination) and (not isinstance(receipt,dict) or receipt.get('input_sha256') != incoming
                or set(receipt)!={'profile','plan_id','input_sha256','result_id','state'}
                or not isinstance(receipt.get('state'),str)
                or receipt['state'] not in ({'started','applied','held','deferred','proposed'} if plan.get('verification_required') else {'started','applied','held','deferred'})
                or receipt.get('result_id') != 'review-'+identifier
                or receipt.get('plan_id') != identifier or receipt.get('profile') != REVIEW_PROFILE):
            raise BrainError('memory-review:decisions-already-frozen-or-unavailable')
        if receipt and ((checked and receipt['state']=='deferred')
                        or (not checked and receipt['state'] in {'applied','held','proposed'})):
            raise BrainError('memory-review:receipt-state-does-not-match-decisions')
        if checked and receipt and receipt['state']=='proposed':
            return _semantic_propose(root,plan,payload,incoming,deferred)
        result_id='review-'+identifier
        digest=request_digest(task['id'],normalized) if checked else None
        proof=_read_attempt(_attempt(root,task['id'],result_id,digest)) if checked else None
        known=bool(proof and proof.get('task_id')==task['id'] and proof.get('result_id')==result_id
                   and proof.get('digest')==digest and proof.get('owner')==owner and proof.get('attestation')=='agent'
                   and proof.get('state') in {'started','complete'})
        if not known and not (not checked and receipt and receipt['state']=='deferred'):
            if (type(plan.get('at')) not in (int,float) or not math.isfinite(plan['at'])
                    or not 0 <= time.time()-plan['at'] <= MAINTENANCE_TTL
                    or task['revision'] != plan['task_revision'] or task['status'] in {'completed','cancelled'}):
                raise BrainError('memory-review:plan-expired-or-task-changed')
            for row in plan['records']:
                # Deferral never claims a check and need not read missing sources.
                if row['record_id'] not in {r['record_id'] for r in checked}:continue
                _,current=_maintenance_image(root,row['_record_path'])
                if current!=row['_record_sha256'] or checked_fingerprints(root,row['sources'])!=row['_source_image']:
                    raise BrainError('memory-review:claim-or-source-changed')
        if receipt is None:
            receipt={'profile':REVIEW_PROFILE,'plan_id':identifier,'input_sha256':incoming,
                     'result_id':result_id,'state':'started'}
            atomic_json(destination,receipt)
        if not checked:
            reply={'profile':REVIEW_PROFILE,'state':'deferred','plan_id':identifier,'deferred':deferred,
                   'records':[],'attestation':'not-performed','replayed':receipt['state']=='deferred'}
        elif plan.get('verification_required'):
            reply=_semantic_propose(root,plan,payload,incoming,deferred)
            atomic_json(destination,{**receipt,'state':'proposed'})
            return reply
        else:
            saved=record_result(root,task['id'],result_id,plan['task_revision'],payload,owner=owner,
                                reason=REVIEW_REASON,attestation='agent',prepared=plan['scope'])
            held=any(row['state'] not in {'updated','existing'} for row in saved['records'])
            held=held or bool(saved['promotion'].get('failed') or saved['promotion'].get('blocked'))
            reply={'profile':REVIEW_PROFILE,'state':'held' if held else 'applied','plan_id':identifier,
                   'deferred':deferred,'result':saved,'attestation':'agent','replayed':saved['replayed']}
        atomic_json(destination,{**receipt,'state':reply['state']})
        return reply


# Provider-free workflow gate. Publication is a trusted-controller operation;
# the primary namespace must mount canonical knowledge read-only. Actor names
# and these local JSON digests do not authenticate an independent model.
SEMANTIC_PROFILE = 'memory-semantic-proposal-v1'
SEMANTIC_PACKET = 'memory-semantic-packet-v1'
SEMANTIC_VERDICT = 'memory-semantic-verdict-v1'
SEMANTIC_AUTH = 'memory-semantic-controller-v1'
SEMANTIC_LIMIT = 1024 * 1024


def _semantic_digest(value):
    try:
        return hashlib.sha256(json.dumps(value,sort_keys=True,ensure_ascii=False,allow_nan=False,
                                        separators=(',',':')).encode('utf-8')).hexdigest()
    except (ValueError,TypeError,UnicodeError,RecursionError) as error:
        raise BrainError('semantic-review:invalid-json-image') from error


def _semantic_path(root,owner,task_id,identifier,suffix='proposal'):
    result=_review_file(root,owner,task_id,identifier).with_name(identifier+'-'+suffix+'.json')
    if os.path.lexists(result):
        image=result.lstat()
        if not stat.S_ISREG(image.st_mode) or image.st_nlink!=1 or linked_reparse(image):
            raise BrainError('semantic-review:unsafe-storage')
    return result


def _semantic_write(destination,value):
    if len(json.dumps(value,ensure_ascii=False,sort_keys=True,indent=2).encode('utf-8'))>SEMANTIC_LIMIT-1:
        raise BrainError('semantic-review:packet-too-large')
    atomic_json(destination,value)


def _semantic_propose(root,plan,payload,incoming,deferred):
    normalized=normalize(root,payload,check_files=False)
    # The legacy promotion renderer folds whitespace. Never approve one text
    # then silently publish another, especially literal NBSP/formfeed examples.
    for row in normalized['learnings']:
        if row['consequence']!=' '.join(row['consequence'].split()):
            raise BrainError('semantic-review:rendering-would-change-claim')
    effective={'progress':None,'verified':True,'learnings':normalized['learnings']}
    value={'profile':SEMANTIC_PROFILE,'proposal_id':plan['plan_id'],'plan':plan,
           'input_sha256':incoming,'payload':effective,'deferred':deferred}
    digest=_semantic_digest(value);value['sha256']=digest
    destination=_semantic_path(root,plan['owner'],plan['task_id'],plan['plan_id'])
    if os.path.lexists(destination):
        previous=_review_read(destination,limit=SEMANTIC_LIMIT)
        if previous!=value:raise BrainError('semantic-review:proposal-already-frozen-or-unavailable')
    else:_semantic_write(destination,value)
    return {'profile':SEMANTIC_PROFILE,'state':'proposal-staged','proposal_id':plan['plan_id'],
            'proposal_sha256':digest,'applied':False,'verification_required':True,
            'attestation':'agent-proposal','deferred':deferred,
            'next':'External controller must independently verify before publication; native save uses learnings=[].'}


def _semantic_load(root,task_id,identifier,expected,owner):
    value=_review_read(_semantic_path(root,owner,task_id,identifier),limit=SEMANTIC_LIMIT)
    if (not isinstance(value,dict) or set(value)!={'profile','proposal_id','plan','input_sha256','payload','deferred','sha256'}
            or value['profile']!=SEMANTIC_PROFILE or value['proposal_id']!=identifier
            or not isinstance(expected,str) or not HEX.fullmatch(expected) or value['sha256']!=expected
            or _semantic_digest({k:v for k,v in value.items() if k!='sha256'})!=expected):
        raise BrainError('semantic-review:proposal-unavailable')
    plan=value['plan']
    if not isinstance(plan,dict) or plan.get('verification_required') is not True:
        raise BrainError('semantic-review:proposal-scope-changed')
    offered=_validate_review_plan(root,plan,task_id,identifier,owner,plan.get('task_uuid'))
    if (not isinstance(value['input_sha256'],str) or not HEX.fullmatch(value['input_sha256'])
            or not isinstance(value['payload'],dict) or set(value['payload'])!={'progress','verified','learnings'}
            or value['payload']['progress'] is not None or value['payload']['verified'] is not True
            or not isinstance(value['deferred'],list)):
        raise BrainError('semantic-review:invalid-proposal')
    normalized=normalize(root,value['payload'],check_files=False)
    seen=set()
    for learning in normalized['learnings']:
        identifier=learning.get('record_id')
        if identifier not in offered or identifier in seen:
            raise BrainError('semantic-review:foreign-or-duplicate-reference')
        previous=offered[identifier]
        if any(learning[k]!=previous[k] for k in ('type','title','sources','expected_revision')) or learning['source_fingerprints']!=previous['_source_image']:
            raise BrainError('semantic-review:reference-changed')
        if learning['consequence']!=' '.join(learning['consequence'].split()):
            raise BrainError('semantic-review:rendering-would-change-claim')
        seen.add(identifier)
    for row in value['deferred']:
        if (not isinstance(row,dict) or set(row)!={'record_id','reason'}
                or not isinstance(row['record_id'],str) or row['record_id'] not in offered or row['record_id'] in seen):
            raise BrainError('semantic-review:invalid-deferral')
        _text(row['reason'],'defer reason',300);seen.add(row['record_id'])
    if seen!=set(offered) or not normalized['learnings']:
        raise BrainError('semantic-review:incomplete-proposal')
    return value


def _semantic_clauses(text,prefix):
    # Segmentation is only a complete checklist, never a truth classifier.
    # No Unicode whitespace/case normalization precedes hashing or comparison.
    if not isinstance(text,str) or not text:raise BrainError('semantic-review:empty-claim')
    starts=[0,*[m.start() for m in re.finditer(r'(?<=[.!?;])(?=[ \t\r\n])|(?<=\n)',text)],len(text)]
    starts=sorted(set(starts));rows=[]
    for begin,end in zip(starts,starts[1:]):
        if begin==end:continue
        fragment=text[begin:end]
        rows.append({'id':prefix+'-'+str(len(rows)),'text':fragment,
                     'sha256':hashlib.sha256(fragment.encode('utf-8')).hexdigest()})
    return rows


def _semantic_current(root,proposal,task,*,fresh=True):
    plan=proposal['plan']
    if task['id']!=plan['task_uuid'] or task['status'] in {'completed','cancelled'}:
        raise BrainError('semantic-review:task-changed')
    if fresh and (type(plan.get('at')) not in (int,float) or not math.isfinite(plan['at'])
                  or not 0<=time.time()-plan['at']<=MAINTENANCE_TTL):
        raise BrainError('semantic-review:proposal-expired')
    from memory_results import checked_fingerprints
    indexed={row['record_id']:row for row in plan['records']}
    for learning in proposal['payload']['learnings']:
        row=indexed[learning['record_id']]
        _,digest=_maintenance_image(root,row['_record_path'])
        if digest!=row['_record_sha256'] or checked_fingerprints(root,row['sources'])!=learning['source_fingerprints']:
            raise BrainError('semantic-review:record-or-source-changed')


def _semantic_sources(root,proposal):
    from memory_results import check_source
    from brain_runtime import _line_ending_neutral
    from validate import indexed_secret_policy_reason
    sources={pin['path']:pin['sha256'] for row in proposal['payload']['learnings'] for pin in row['source_fingerprints']}
    rows=[];total=0
    for name,expected in sorted(sources.items()):
        check_source(root,name)
        source=workspace_roots.resolve(root,name)
        descriptor=os.open(source,os.O_RDONLY|getattr(os,'O_NOFOLLOW',0)|getattr(os,'O_NONBLOCK',0)|getattr(os,'O_BINARY',0))
        try:
            before=os.fstat(descriptor)
            if not stat.S_ISREG(before.st_mode) or before.st_nlink!=1 or before.st_size>262144:
                raise BrainError('semantic-review:source-unavailable')
            with os.fdopen(descriptor,'rb',closefd=False) as handle:raw=handle.read(262145)
            after=os.fstat(descriptor);named=source.lstat()
            identity=lambda image:(image.st_dev,image.st_ino,image.st_size,image.st_mtime_ns,workspace_roots.change_time_ns(image),image.st_nlink,image.st_mode)
            if len(raw)>262144 or identity(before)!=identity(after) or identity(after)!=identity(named):
                raise BrainError('semantic-review:source-changed')
        finally:os.close(descriptor)
        try:text=raw.decode('utf-8')
        except UnicodeError as error:raise BrainError('semantic-review:source-unavailable') from error
        if '\0' in text or indexed_secret_policy_reason(text) or hashlib.sha256(_line_ending_neutral(raw)).hexdigest()!=expected:
            raise BrainError('semantic-review:source-unavailable')
        total+=len(raw)
        if total>524288:raise BrainError('semantic-review:source-budget')
        rows.append({'path':name,'sha256':expected,'raw_sha256':hashlib.sha256(raw).hexdigest(),'text':text})
    return rows


def semantic_review_export(root,task_id,identifier,expected,*,owner):
    """Trusted host exports after primary exit. No model invocation or writes."""
    if maintenance_mode(root)!='review-verified':raise BrainError('semantic-review:disabled')
    with mutation_lock(root):
        proposal=_semantic_load(root,task_id,identifier,expected,owner)
        task=writable_task(root,task_id,owner);_semantic_current(root,proposal,task)
        from brain_runtime import find_record,parse_markdown_record
        from validate import indexed_secret_policy_reason,memory_temporal_reason
        task_path,_,_=find_record(root,task['id']);task_hash=hashlib.sha256(task_path.read_bytes()).hexdigest()
        sources=_semantic_sources(root,proposal);records=[]
        old={r['record_id']:r for r in proposal['plan']['records']}
        chunks=sorted(islice(workspace_roots.resolve(root,'memory-bank/chunks').glob('*.md'),2001))
        if len(chunks)>2000:raise BrainError('semantic-review:chunk-budget')
        for learning in proposal['payload']['learnings']:
            previous=old[learning['record_id']];memories=[]
            for chunk in chunks:
                name='memory-bank/chunks/'+chunk.name
                text,digest=_maintenance_image(root,name)
                metadata,body=parse_markdown_record(Path(name),text=text)
                links=metadata.get('sources')
                if (not isinstance(links,list) or len(links)>100 or any(not isinstance(link,str) for link in links)
                        or not isinstance(metadata.get('id'),str) or not metadata['id']
                        or not isinstance(metadata.get('privacy','team'),str)
                        or not isinstance(metadata.get('authority','observed'),str)):
                    raise BrainError('semantic-review:memory-unavailable')
                if metadata.get('status')=='active' and previous['_record_path'] in links:
                    config=load_config(root)
                    if (metadata.get('privacy','team') not in set(config['allowed_privacy']) & {'public','team'}
                            or metadata.get('authority','observed') not in config['allowed_authority']
                            or memory_temporal_reason(metadata)
                            or indexed_secret_policy_reason(text)):
                        raise BrainError('semantic-review:memory-not-admitted')
                    memories.append({'id':metadata['id'],'path':name,'body':body,'sha256':digest})
            records.append({'record_id':learning['record_id'],'title':learning['title'],
                'old_consequence':previous['consequence'],'proposed_consequence':learning['consequence'],
                'old_clauses':_semantic_clauses(previous['consequence'],'old'),
                'new_clauses':_semantic_clauses(learning['consequence'],'new'),
                'sources':learning['sources'],'old_memories':memories})
        packet={'profile':SEMANTIC_PACKET,'proposal_id':identifier,'proposal_sha256':expected,
                'task_id':task_id,'task_uuid':task['id'],'owner':owner,'project':_project(root),
                'task_revision':task['revision'],'task_sha256':task_hash,'sources':sources,
                'source_snapshot_sha256':_semantic_digest(sources),'records':records}
        packet['sha256']=_semantic_digest(packet)
        if len(json.dumps(packet,ensure_ascii=False,indent=2).encode())>SEMANTIC_LIMIT:
            raise BrainError('semantic-review:packet-too-large')
        return packet


def semantic_verdict_check(packet,verdict):
    """Validate evidence coverage/bindings. The external reviewer judges truth."""
    if (not isinstance(verdict,dict) or set(verdict)!={'profile','packet_sha256','proposal_sha256','verdict','records'}
            or verdict.get('profile')!=SEMANTIC_VERDICT or verdict.get('packet_sha256')!=packet['sha256']
            or verdict.get('proposal_sha256')!=packet['proposal_sha256']
            or not isinstance(verdict.get('verdict'),str) or verdict['verdict'] not in {'accept','reject','unresolved'}
            or not isinstance(verdict.get('records'),list) or len(verdict['records'])!=len(packet['records'])):
        raise BrainError('semantic-review:invalid-verdict')
    records={row['record_id']:row for row in packet['records']};sources={s['path']:s for s in packet['sources']};seen=set();accepted=True
    def evidence(rows):
        if not isinstance(rows,list) or not 1<=len(rows)<=5:raise BrainError('semantic-review:missing-evidence')
        for cite in rows:
            if (not isinstance(cite,dict) or set(cite)!={'path','sha256','quote'}
                    or not isinstance(cite.get('path'),str) or cite['path'] not in sources
                    or cite['sha256']!=sources[cite['path']]['sha256']
                    or not isinstance(cite['quote'],str) or not 1<=len(cite['quote'])<=500
                    or cite['quote'] not in sources[cite['path']]['text']):
                raise BrainError('semantic-review:invalid-evidence')
    for review in verdict['records']:
        if (not isinstance(review,dict) or set(review)!={'record_id','old_clauses','new_clauses','boundary_checks'}
                or not isinstance(review.get('record_id'),str) or review['record_id'] not in records or review['record_id'] in seen):
            raise BrainError('semantic-review:invalid-record-review')
        seen.add(review['record_id']);record=records[review['record_id']]
        for key in ('old_clauses','new_clauses'):
            offered={r['id']:r for r in record[key]};rows=review[key];used=set()
            if not isinstance(rows,list) or len(rows)!=len(offered):raise BrainError('semantic-review:incomplete-coverage')
            for row in rows:
                if (not isinstance(row,dict) or set(row)!={'id','sha256','status','reason','evidence'}
                        or not isinstance(row.get('id'),str) or row['id'] not in offered or row['id'] in used
                        or row['sha256']!=offered[row['id']]['sha256'] or not isinstance(row.get('status'),str)):
                    raise BrainError('semantic-review:invalid-clause-review')
                allowed=({'retained','changed-with-evidence','removed-with-evidence','unresolved'} if key=='old_clauses'
                         else {'supported','contradicted','unresolved'})
                if row['status'] not in allowed:raise BrainError('semantic-review:invalid-clause-status')
                _text(row['reason'],'clause reason',500);evidence(row['evidence']);used.add(row['id'])
                if row['status'] in {'contradicted','unresolved'}:accepted=False
        checks=review['boundary_checks']
        if not isinstance(checks,list) or not 1<=len(checks)<=10:raise BrainError('semantic-review:missing-boundary-checks')
        for check in checks:
            if (not isinstance(check,dict) or set(check)!={'case','status','evidence'}
                    or not isinstance(check.get('status'),str) or check['status'] not in {'supported','contradicted','unresolved'}):
                raise BrainError('semantic-review:invalid-boundary-check')
            _text(check['case'],'boundary case',500);evidence(check['evidence'])
            if check['status']!='supported':accepted=False
    return {'state':('accepted' if accepted else 'unresolved') if verdict['verdict']=='accept' else verdict['verdict'],
            'publish':bool(verdict['verdict']=='accept' and accepted)}


def _semantic_packet_check(packet,proposal):
    fields={'profile','proposal_id','proposal_sha256','task_id','task_uuid','owner','project',
            'task_revision','task_sha256','sources','source_snapshot_sha256','records','sha256'}
    plan=proposal['plan']
    if (not isinstance(packet,dict) or set(packet)!=fields or packet['profile']!=SEMANTIC_PACKET
            or any(packet[k]!=plan[k] for k in ('task_id','task_uuid','owner','project'))
            or packet['proposal_id']!=plan['plan_id'] or packet['proposal_sha256']!=proposal['sha256']
            or type(packet['task_revision']) is not int or packet['task_revision']<1
            or not isinstance(packet['task_sha256'],str) or not HEX.fullmatch(packet['task_sha256'])
            or _semantic_digest({k:v for k,v in packet.items() if k!='sha256'})!=packet['sha256']
            or not isinstance(packet['sources'],list) or not isinstance(packet['records'],list)):
        raise BrainError('semantic-review:packet-changed')
    from brain_runtime import _line_ending_neutral
    pins={p['path']:p['sha256'] for row in proposal['payload']['learnings'] for p in row['source_fingerprints']}
    sources={}
    for row in packet['sources']:
        if (not isinstance(row,dict) or set(row)!={'path','sha256','raw_sha256','text'}
                or not isinstance(row['path'],str) or row['path'] not in pins or row['path'] in sources
                or row['sha256']!=pins[row['path']] or not isinstance(row['text'],str)
                or hashlib.sha256(row['text'].encode('utf-8')).hexdigest()!=row['raw_sha256']
                or hashlib.sha256(_line_ending_neutral(row['text'].encode('utf-8'))).hexdigest()!=row['sha256']):
            raise BrainError('semantic-review:invalid-packet-source')
        sources[row['path']]=row
    if set(sources)!=set(pins) or packet['source_snapshot_sha256']!=_semantic_digest(packet['sources']):
        raise BrainError('semantic-review:incomplete-packet-source')
    offered={row['record_id']:row for row in plan['records']};seen=set()
    proposed={row['record_id']:row for row in proposal['payload']['learnings']}
    for row in packet['records']:
        if (not isinstance(row,dict) or set(row)!={'record_id','title','old_consequence','proposed_consequence',
                                                  'old_clauses','new_clauses','sources','old_memories'}
                or not isinstance(row['record_id'],str) or row['record_id'] not in proposed or row['record_id'] in seen):
            raise BrainError('semantic-review:invalid-packet-record')
        previous=offered[row['record_id']];current=proposed[row['record_id']]
        if (row['title']!=current['title'] or row['sources']!=current['sources']
                or row['old_consequence']!=previous['consequence'] or row['proposed_consequence']!=current['consequence']
                or row['old_clauses']!=_semantic_clauses(previous['consequence'],'old')
                or row['new_clauses']!=_semantic_clauses(current['consequence'],'new')
                or not isinstance(row['old_memories'],list) or len(row['old_memories'])>2000):
            raise BrainError('semantic-review:packet-claim-changed')
        for memory in row['old_memories']:
            if (not isinstance(memory,dict) or set(memory)!={'id','path','body','sha256'}
                    or any(not isinstance(memory[k],str) for k in memory)
                    or not HEX.fullmatch(memory['sha256'])):
                raise BrainError('semantic-review:invalid-packet-memory')
        seen.add(row['record_id'])
    if seen!=set(proposed):raise BrainError('semantic-review:incomplete-packet-records')


def semantic_controller_input(root,name):
    """Only host-owned files outside all three accelerator/project roots.

    This is a placement check, not authentication. OS isolation keeps these
    files and this publishing entry point outside the primary's control.
    """
    source=Path(name)
    if not source.is_absolute():raise BrainError('semantic-review:external-input-required')
    resolved=source.resolve(strict=True)
    for anchor in workspace_roots.roots(root)[:3]:
        try:resolved.relative_to(anchor.resolve())
        except ValueError:continue
        raise BrainError('semantic-review:external-input-required')
    value=_review_read(source,limit=SEMANTIC_LIMIT)
    if value is None:raise BrainError('semantic-review:external-input-unavailable')
    return value


def semantic_review_apply(root,task_id,identifier,expected,packet,authorization,*,owner):
    """Trusted controller only, outside the primary's read-only knowledge view.

    No actor-name/JSON field authenticates a model. The host must capture an
    independent isolated process and keep this authorization outside primary
    control. All digests below provide consistency, not signatures.
    """
    if maintenance_mode(root)!='review-verified':raise BrainError('semantic-review:disabled')
    with mutation_lock(root):
        proposal=_semantic_load(root,task_id,identifier,expected,owner)
        _semantic_packet_check(packet,proposal)
        if (not isinstance(authorization,dict) or set(authorization)!={'profile','packet_sha256','primary_request','reviewer_session','verdict'}
                or authorization['profile']!=SEMANTIC_AUTH or authorization['packet_sha256']!=packet['sha256']
                or not isinstance(authorization.get('primary_request'),str) or not HEX.fullmatch(authorization['primary_request'])
                or not isinstance(authorization.get('reviewer_session'),str)
                or not re.fullmatch(r'[A-Za-z0-9._:-]{1,160}',authorization['reviewer_session'])):
            raise BrainError('semantic-review:controller-authorization-unavailable')
        assessment=semantic_verdict_check(packet,authorization['verdict'])
        authorization_hash=_semantic_digest(authorization)
        binding={'profile':SEMANTIC_AUTH,'packet_sha256':packet['sha256'],'authorization_sha256':authorization_hash}
        destination=_semantic_path(root,owner,task_id,identifier,'authorization')
        previous=_review_read(destination)
        if os.path.lexists(destination) and previous!=binding:
            raise BrainError('semantic-review:authorization-already-frozen-or-unavailable')
        normalized=normalize(root,proposal['payload'],check_files=False)
        task_uuid=proposal['plan']['task_uuid'];digest=request_digest(task_uuid,normalized)
        result_id='semantic-'+authorization_hash[:48]
        from memory_results import _receipt,_valid_receipt
        completed_receipt=_valid_receipt(_read_receipt(_receipt(root,task_uuid,result_id)),task_uuid,result_id,digest)
        proof=_read_attempt(_attempt(root,task_uuid,result_id,digest))
        known=completed_receipt or bool(proof and proof.get('state') in {'started','complete'} and proof.get('owner')==owner
                   and proof.get('task_id')==task_uuid and proof.get('result_id')==result_id and proof.get('digest')==digest
                   and proof.get('attestation')=='agent')
        if known and previous!=binding:raise BrainError('semantic-review:authorization-unavailable')
        # A completed canonical result is passive: a newer prompt, expiration,
        # source removal or task archival must not turn replay into new work.
        if not (completed_receipt or known and proof['state']=='complete'):
            state=_read(root,authorization['primary_request'])
            if (state is None or state['phase']!='saved' or state['owner']!=owner or state['task_id']!=task_id
                    or state['task_uuid']!=task_uuid or not _proven(root,state)
                    or state['session']==hashlib.sha256(authorization['reviewer_session'].encode()).hexdigest()):
                raise BrainError('semantic-review:primary-unfinished-or-same-session')
        if not assessment['publish']:
            if previous is None:_semantic_write(destination,binding)
            return {'profile':SEMANTIC_AUTH,'state':assessment['state'],'applied':False,'attestation':'not-performed'}
        if not known:
            from brain_runtime import find_record
            task=writable_task(root,task_id,owner);_semantic_current(root,proposal,task)
            task_path,_,_=find_record(root,task['id'])
            if (packet['task_uuid']!=task['id'] or packet['task_revision']!=task['revision']
                    or packet['task_sha256']!=hashlib.sha256(task_path.read_bytes()).hexdigest()
                    or _semantic_sources(root,proposal)!=packet['sources']):
                raise BrainError('semantic-review:task-or-packet-source-changed')
            # Re-export checks the old active memory body and admission too.
            if semantic_review_export(root,task_id,identifier,expected,owner=owner)!=packet:
                raise BrainError('semantic-review:publication-frontier-changed')
        if previous is None:_semantic_write(destination,binding)
        result=record_result(root,task_uuid,result_id,packet['task_revision'],proposal['payload'],owner=owner,
                             reason=REVIEW_REASON,attestation='agent',prepared=proposal['plan']['scope'])
        held=any(row['state'] not in {'updated','existing'} for row in result['records'])
        held=held or bool(result['promotion'].get('failed') or result['promotion'].get('blocked'))
        return {'profile':SEMANTIC_AUTH,'state':'held' if held else 'applied','applied':not held,
                'attestation':'agent','reviewer_session':authorization['reviewer_session'],'result_id':result_id,
                'proposal_sha256':expected,'packet_sha256':packet['sha256'],'result':result}


def _instruction(root,key,state,*,brief=False,host=None,full=True,entered=False,back=None):
    tooling=Path(__file__).resolve().parents[2]
    interpreter=['python3']
    if os.name == 'nt':
        interpreter=['python'] if shutil.which('python') else ['py','-3']
    # A worktree its main checkout's installation serves: Claude Code gets the
    # roots' variables for its commands from the dispatcher's environment
    # entry, Codex has no file a hook can give its commands variables through
    # - the command names them, so the save serves the worktree as the hooks do.
    layout=workspace_roots.roots(root)
    assign=''
    if layout.shared and host=='codex':
        assign=' '.join(name+'='+shlex.quote(str(value)) for name,value in (
            (workspace_roots.HOME_VARIABLE,layout.tooling),(workspace_roots.STATE_VARIABLE,layout.state),
            (workspace_roots.PROJECT_VARIABLE,layout.project)))+' '
    # Keep the documented CLI spelling covered by the installed Bash
    # permission rules, even when the session's cwd is a subdirectory. In a
    # worktree this installation serves, the command names the worktree: run
    # from the root, nothing else would show it to a session that entered it -
    # nor, attached, from the clone, where the session's own variables name
    # the folder the launcher started it in.
    layout=workspace_roots.roots(Path(root))
    named=layout.shared or entered
    place=['--project',str(layout.project)] if named else []
    def cli(*tail):
        return assign+shlex.join([*interpreter,'memory-bank/scripts/context.py','--root',str(root),*place,'native-result',
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
        # In a worktree the command reads the result from stdin without
        # naming the descriptor: once a session has entered a worktree,
        # Claude Code refuses a program fed a descriptor it names ("-").
        piped=() if named else ('--input','-')
        # Attached, the cd is into the clone, which the launch adds as a
        # working directory: Claude Code's shell stayed there, and every
        # command the agent ran after the save - for good, in a session a
        # person drives - ran in the clone. A save that went through goes
        # back to where the session was; a refused one keeps its exit status
        # (after `;` the cd's 0 hid it). `cd -` an entered worktree refuses
        # ("a program assembled at runtime").
        back=' && cd '+shlex.quote(back) if layout.attached and not layout.shared and back else ''
        ask=('Use your shell tool to run this exact command: '+enter+"echo -E '<JSON>' | "
             +cli(*piped,'--json')+back+'\n'
             "Put one JSON object in place of <JSON>, writing each ' inside it as '\\''. ")
        skip='run this command instead: '+enter+cli('--skip','--json')+back+'\n'
    else:
        ask='Use your shell tool to pipe one JSON object into this exact command: (' \
            +enter+cli('--input','-','--json')+')\n'
        skip='use the same command replacing --input - with --skip. '
    rules=(_rules_marker(root)+' JSON: progress (summary, at most 1000 characters), optional next_steps '
        '(at most 3 strings of 300 characters; omit to preserve the current plan), '
        'learnings (at most 3 objects with type finding|decision, title, consequence, sources). '
        'Exact schema example (replace placeholders, never save the example): '
        '{"progress":"summary","verified":true,"learnings":[{"type":"finding",'
        '"title":"checked rule","consequence":"reusable consequence","sources":["checked-file.php"]}]}. '
        'verified is a TOP-LEVEL boolean, NEVER a field inside a learning. '
        'New consequences use ordinary learnings. To recheck or correct an inspected existing claim, '
        'preserve its type/title/sources and add its exact record_id, expected_revision and checked '
        'source_fingerprints (read with context.py source-fingerprints --source PATH --json). '
        'Do not create a duplicate finding. Recalling alone uses learnings=[] and progress only. '
        'Use only canonical project-relative source files you actually checked. Set verified=true '
        'only when you personally checked every learning against current sources; this is agent '
        'attestation, never human review. Empty learnings is valid. No raw requests, transcripts, '
        'logs, credentials or personal data. ') if full else (
        # The conversation holds the full step from an earlier answer.
        'The JSON follows the rules of the full save step earlier in this conversation: progress, optional '
        'next_steps, learnings with checked sources, and verified as a top-level boolean. ')
    if maintenance_mode(root)=='review-verified':
        rules+=' Independent-review mode: use learnings=[]; reusable knowledge is proposal-only and published by the external controller. '
    body=('This is an internal continuation: do not announce routine memory operations. Save only the result of the work already performed in this session. Do not repeat the task, '
        'edit application files, run tests, delegate, or mark the task completed. '
        +ask+rules+maintenance_instruction(root,state)+'If no project result is safe to save, '+skip+
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


def stop(root, task_id, host, payload, *, owner, mode, entered=False, served=None):
    with _Step(root,'stop',host) as step:
        return _stop(step,root,task_id,host,payload,owner=owner,mode=mode,entered=entered,served=served)


def _stop(step, root, task_id, host, payload, *, owner, mode, entered=False, served=None):
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
    collaboration,sandbox,files=_rollout_turn(payload) if host=='codex' else (None,None,None)
    with mutation_lock(root):
        state=_read(root,key)
        if state is None and entered and not recovery(host,payload):
            # The session entered this worktree during the turn (Claude
            # Code's EnterWorktree): its prompt armed the request where the
            # session was then. The turn's work is this worktree's, so the
            # request is armed here.
            state=_armed(root,host,payload,task_id,owner);_write(root,key,state)
        if state is None: return step({},'skipped','not-armed')
        if recovery(host,payload):
            # The continuation answered the request a prompt sent during its
            # save moved aside.
            slot=key
            if state['phase'] != 'requested':
                aside=_read_aside(root,key)
                if aside is not None: state,slot=aside,_aside(key)
            if state['phase']=='requested':
                # Held: a finished save the state never took. Failed: no
                # save began, or one began and did not finish. Refused:
                # Claude Code refused the save command itself.
                if _proven(root,state): state['phase']='held'
                elif host=='claude' and not state['digest'] and _claude_refusal(payload,state['token']):
                    state['phase']='refused'
                else: state['phase']='failed'
                _write(root,slot,state)
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
        # Codex started below the project writes only there: a save to the
        # project's Brain failed with a read-only file system, every answer.
        # Where the sandbox writes a temporary directory, the save waits
        # there for this session's next hook (_spool); elsewhere none is asked.
        if (files is not None and not _sandbox_writes(files, os.path.realpath(brain_root(root)))
                and not any(_sandbox_writes(files, os.path.realpath(str(folder.parent))) for folder in _spool_dirs())):
            return step({},'skipped','outside-sandbox')
        try: task=_task(root,task_id,owner,served)
        except BrainRecordNotFound as error:
            if error.identifier != task_id: raise
            task=None
        except BrainError as error:
            # A record claiming the task cannot be read - an unresolved merge
            # conflict, say. The capsule asks the person to repair it, and a
            # save could only fail: none is asked until then.
            if 'record invalid' not in str(error): raise
            return step({},'skipped','task-unreadable')
        if task is not None:
            if task['status'] in {'completed','cancelled'}: return step({},'skipped','task-'+task['status'])
            state.update(task_uuid=task['id'],revision=task['revision'])
        state.update(phase='requested',scope=prepared_scope(root,owner,REASON,'agent'),
                     checkout=_checkout(root),adapter=_adapter(),updated=time.time())
        _write(root,key,state)
        # Cursor's transcript, when it keeps one, and how it records a
        # summarized conversation could not be checked here: it gets the
        # full step every time, as before.
        full=configured_instruction(root)=='full' or host=='cursor' or not _rules_in_context(payload,_rules_marker(root))
        where=payload.get('cwd')
        instruction=_instruction(root,key,state,brief=brief,host=host,full=full,entered=entered,
                                 back=where if (isinstance(where,str) and os.path.isabs(where)
                                                and not any(c in where for c in '\n\r\0')) else None)
    # Claude Code labels a block a hook error, notifies one after every
    # answer and hands the model the reason twice. Its Stop additionalContext
    # (2.1.163 and later) continues the turn as hook feedback, once, through
    # the same stop_hook_active and continuation cap.
    if host=='claude': output={'hookSpecificOutput':{'hookEventName':'Stop','additionalContext':instruction}}
    elif host=='cursor': output={'followup_message':instruction}
    else: output={'decision':'block','reason':instruction}
    return step(output,'requested',code,answer='brief' if brief else 'repeat')


def save(root,key,token,data,*,owner,mode,provision,skip=False,spooled=False,served=None):
    with _Step(root,'save',key=key) as step:
        try:
            return _save(step,root,key,token,data,owner=owner,mode=mode,provision=provision,skip=skip,spooled=spooled,
                         served=served)
        except OSError as error:
            # A sandbox that writes only below a subdirectory mounts the
            # project read-only: the save's first write, the runtime lock,
            # fails before anything is written.
            if spooled or error.errno not in UNWRITABLE or not _under(error.filename, brain_root(root)): raise
            return _spool(step,root,key,token,data,owner=owner,skip=skip)


def _under(path, base):
    if not isinstance(path, (str, bytes, os.PathLike)): return False
    path, base = os.path.realpath(os.fsdecode(path)), os.path.realpath(str(base))
    return path == base or path.startswith(base.rstrip(os.sep) + os.sep)


def _spool_dirs():
    """The folders a save the project cannot take waits in: one of this
    user's in each temporary directory a sandbox lets a command write
    (workspace_roots.spool_directories)."""
    return workspace_roots.spool_directories(SPOOL_NAME)


def _own(image):
    """A spool entry this user owns, not a link."""
    return not linked_reparse(image) and (not hasattr(os, 'getuid') or image.st_uid == os.getuid())


def _check_save_scope(root, state, owner):
    if state['owner'] != owner or state['project'] != _project(root):
        raise BrainError('native-result:request-unavailable')
    if (time.time()-state['updated'] > 24*60*60 or state['updated'] > time.time()+60
            or state['checkout'] != _checkout(root) or state['adapter'] != _adapter()
            or state['scope'] != prepared_scope(root,owner,REASON,'agent')):
        raise BrainError('native-result:scope-changed')


def _spool(step, root, key, token, data, *, owner, skip):
    """The project cannot be written from here. The request and the result
    are checked as a save checks them, so the agent can still correct a
    refused one, and the result waits in the spool for the session's next
    hook (drain)."""
    state = _read(root, key)
    if state is None or state['token'] != token:
        aside = _read_aside(root, key)
        if aside is not None and aside['token'] == token: state = aside
    if state is None or state['token'] != token or state['phase'] != 'requested':
        raise BrainError('native-result:request-unavailable')
    step.host = state['host']
    _check_save_scope(root, state, owner)
    if not skip: normalize(root, data, check_files=True)
    record = {'profile': SPOOL_PROFILE, 'token': token, 'skip': bool(skip), 'data': None if skip else data,
              'created': time.time()}
    for directory in _spool_dirs():
        try:
            directory.mkdir(mode=0o700, exist_ok=True)
            image = directory.lstat()
            if not stat.S_ISDIR(image.st_mode) or not _own(image): continue
            atomic_json(directory / (key + '.json'), record)
        except OSError:
            continue
        return step({'state': 'spooled', 'attestation': 'agent',
                     'detail': 'This sandbox cannot write the project; the save is made when this turn ends.'},
                    'spooled')
    raise BrainError('native-result:project-read-only')


def _read_spool(path):
    try:
        image, folder = path.lstat(), path.parent.lstat()
        if (not stat.S_ISREG(image.st_mode) or image.st_nlink != 1 or not _own(image)
                or image.st_size > MAX_INPUT + 4096 or not stat.S_ISDIR(folder.st_mode) or not _own(folder)):
            return None
        value = json.loads(path.read_bytes())
    except (OSError, ValueError, RecursionError):
        return None
    if (not isinstance(value, dict) or value.get('profile') != SPOOL_PROFILE or type(value.get('skip')) is not bool
            or not isinstance(value.get('token'), str) or not TOKEN.fullmatch(value['token'])):
        return None
    return value


def _task(root, task_id, owner, served=None):
    """writable_task, except where more than one record claims the task ID -
    two copies of a branch that each started its task, merged: then the
    record `served` names, the one this clone's capsule serves and its turn
    checkpoint writes (context.served_task_record), so the save goes where
    the capsule says the turn does."""
    try:
        return writable_task(root,task_id,owner)
    except BrainRecordNotFound:
        raise
    except BrainError as error:
        uuid=served(task_id) if served is not None and 'ambiguous' in str(error) else None
        if uuid is None: raise
        task=writable_task(root,uuid,owner)
        if task['external_id'] != task_id: raise
        return task


def _spooled(directory, key):
    """The spool files `key` has in `directory`, as (path, attempts begun):
    those an earlier hook began first (drain), then the one the sandbox left."""
    found=[(directory/f'{key}.attempt{attempt}.json',attempt) for attempt in range(DRAIN_ATTEMPTS,0,-1)]
    return [(path,attempt) for path,attempt in found+[(directory/(key+'.json'),0)] if os.path.lexists(path)]


def drain(root, task_id, host, payload, *, owner, mode, provision, served=None):
    """Save what the agent's sandbox left in the spool (_spool), from a hook,
    which runs outside the sandbox.

    The file goes once the save has answered, whatever it said. Removed
    before, a save that waited on another session's lock was lost when the
    hook's own budget (`timeout`) or its host stopped the hook: the file
    gone, the save never made, nothing journaled. So the file is first
    renamed for the attempt, and an attempt that never answers leaves it for
    the next hook, which begins it again - up to DRAIN_ATTEMPTS, then gives
    it up as refused (`interrupted`). Beginning again a save that had landed
    only repeats it: the request's token keys it."""
    try: task_id=check_task_id(task_id)
    except BrainError: return None
    key=_key(root,host,payload,task_id,owner) if isinstance(payload,dict) else None
    if key is None: return None
    result=None
    for directory in _spool_dirs():
        if not _spooled(directory,key): continue
        # A file under an attempt's name is a stopped drain's only while no
        # other drain - a hook its host left running - still holds it.
        with workspace_roots.draining(directory,'spool') as alone:
            if not alone: continue
            for path,begun in _spooled(directory,key):
                record=_read_spool(path)
                if record is None: continue
                if begun >= DRAIN_ATTEMPTS:
                    try: path.unlink()
                    except OSError: continue
                    _journal(root,host,'save','refused','interrupted')
                    continue
                attempt=directory/f'{key}.attempt{begun+1}.json'
                try: os.replace(path,attempt)
                except OSError: continue
                try:
                    result=save(root,key,record['token'],record['data'],owner=owner,mode=mode,provision=provision,
                                skip=record['skip'],spooled=True,served=served)
                except BrainError:
                    pass  # journaled as the save's refusal; the Stop settles the request
                # Unexpected failures leave this named attempt available for
                # the next bounded drain, just as a killed hook does.
                try: attempt.unlink()
                except OSError: pass
    return result


def _save(step,root,key,token,data,*,owner,mode,provision,skip,spooled=False,served=None):
    if not _enabled(mode): raise BrainError('native-result:governed-session-required')
    if not isinstance(token,str) or not TOKEN.fullmatch(token): raise BrainError('native-result:invalid-token')
    with mutation_lock(root):
        state,slot=_read(root,key),key
        # A prompt sent while this save was written moved its request aside.
        if state is None or state['token'] != token:
            aside=_read_aside(root,key)
            if aside is not None and aside['token'] == token: state,slot=aside,_aside(key)
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
        _check_save_scope(root, state, owner)
        if skip:
            if state['digest'] is not None: raise BrainError('native-result:save-already-started')
            state['phase']='omitted';_write(root,slot,state)
            return step({'state':'omitted','attestation':'agent'},'omitted',
                        'spooled' if spooled else 'in-flight' if slot != key else None)
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
        # Another session of the branch may have created the task or moved
        # its revision since this save was requested: its own save, or the
        # turn checkpoint its Stop hook writes every few turns. That refused
        # this save, learnings included, and had the agent report it to a
        # person who had already read the answer. Nothing else writes under
        # this lock, so a save asked for while it could still be written -
        # IN_FLIGHT_SECONDS, as for a prompt sent meanwhile - is recorded at
        # the task as it now is, as the Harness retries its own: its progress
        # and steps replace the other's, as they would have a moment later.
        # An older one, spooled until a hook long after, yields to what was
        # written since, and a task replaced since - completed and started
        # again - is another task's.
        fresh=0 <= time.time()-state['updated'] < IN_FLIGHT_SECONDS
        rebased=False
        if state['task_uuid'] is None:
            try: task=_task(root,state['task_id'],owner,served)
            except BrainRecordNotFound as error:
                if error.identifier != state['task_id']: raise
                provision(state['task_id'])
                task=_task(root,state['task_id'],owner,served)
            else:
                if not fresh: raise _overtaken('native-result:task-changed','the branch task was created')
                rebased=True
            state.update(task_uuid=task['id'],revision=task['revision'])
            _write(root,slot,state)
        elif not begun:
            task=_task(root,state['task_id'],owner,served)
            if task['id'] != state['task_uuid']:
                raise _overtaken('native-result:task-changed','the branch task was replaced')
            if task['revision'] != state['revision'] and fresh: state['revision'],rebased=task['revision'],True
        digest=request_digest(state['task_uuid'],normalized)
        if state['digest'] is not None and state['digest'] != digest:
            raise BrainError('native-result:content-changed')
        state['digest']=digest;_write(root,slot,state)
        try:
            result=record_result(root,state['task_uuid'],'native-'+token,state['revision'],data,
                                 owner=owner,reason=REASON,attestation='agent',prepared=state['scope'],
                                 replay_only=begun)
        except BrainError as error:
            if str(error) != 'memory-result-v1:prepared_stale_no_write': raise
            raise _overtaken(str(error),'another session or a person saved the task') from error
        held=any(item['state'] in {'differs','archived-differs'} for item in result['records'])
        state['phase']='held' if held else 'saved';_write(root,slot,state)
        # For the journal: a save the agent's sandbox spooled, one that landed
        # after a prompt moved its request aside, or one recorded at a task
        # another session created or moved since the request.
        return step(result,state['phase'],repeat or ('spooled' if spooled else 'in-flight' if slot != key
                                                     else 'rebased' if rebased else None),
                    learnings=sum(item['state'] in {'created','completed','updated'} for item in result['records']),
                    submitted=len({learning_id(state['task_uuid'],row) for row in normalized['learnings']}),
                    existing=sum(item['state'] in {'existing','archived'} for item in result['records']),
                    held=sum(item['state'] in {'differs','archived-differs','missing'} for item in result['records']),
                    promoted=len((result['promotion'] or {}).get('promoted') or ()),
                    **({'capture_decision': normalized['capture_decision']} if 'capture_decision' in normalized else {}))
