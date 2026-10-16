"""Private insert-only bindings for an original worker's safe memory request.

Bodies never enter session/launch columns or events. Retention is the lifetime
of the owner's sessions database: no unresolved binding is evicted or replayed
on startup. History grows with generations like sessions/events; only each
binding's size is capped, never the lifetime row count. This table is local
bookkeeping, not tamper-proof evidence.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re

from .sessions import SessionError

PROFILE = 'harness-memory-request-v1'
BYTE_LIMIT = 128 * 1024
CONFLICT = 'This memory attempt changed. Its original safe request was retained; no new writes were started.'
INCOMPLETE = 'Memory replay is incomplete: the original attempt cannot be safely resumed.'


def encoded(value):
    try:
        text = json.dumps(value, sort_keys=True, ensure_ascii=False, allow_nan=False)
        if len(text.encode('utf-8')) > BYTE_LIMIT:
            raise ValueError()
        return text
    except (TypeError, ValueError, RecursionError, UnicodeError) as error:
        raise SessionError('The private memory attempt exceeds its safe JSON limits.') from error


def fingerprint(data):
    if not isinstance(data, dict):
        raise SessionError('Invalid memory draft.')
    return hashlib.sha256(encoded({k: v for k, v in data.items() if k != 'used_memory'}).encode('utf-8')).hexdigest()


def scope(knowledge, session, options, workspace):
    layout = knowledge.layout(session['project_id'], workspace)
    environment = {**os.environ, **(layout['environment'] or {})}
    return {'project': session['project_id'], 'workspace': str(Path(workspace).resolve()),
            'bank': options['bank'], 'task': options.get('record_id'),
            'folder': str(layout['folder'].resolve()),
            'sources': str(layout['sources'].resolve()),
            'scripts': str(layout['scripts'].resolve()) if layout['scripts'] is not None else None,
            'owner': environment.get('PROJECT_BRAIN_OWNER', 'local')}


def lookup(sessions, sid, generation):
    with sessions.lock:
        row = sessions.db.execute('SELECT binding FROM memory_result_attempts WHERE session_id=? AND generation=?',
                                  (sid, generation)).fetchone()
    if row is None:
        return None
    try:
        if (not isinstance(row[0], str) or len(row[0]) > BYTE_LIMIT
                or len(row[0].encode('utf-8')) > BYTE_LIMIT):
            raise ValueError()
        value = json.loads(row[0])
        encoded(value)
        if (not isinstance(value, dict) or set(value) != {'profile', 'identity', 'prepared', 'skipped_count'}
                or value['profile'] != PROFILE or not isinstance(value['identity'], dict)
                or not isinstance(value['prepared'], dict) or type(value['skipped_count']) is not int
                or not 0 <= value['skipped_count'] <= 3):
            raise ValueError()
        return value
    except (TypeError, ValueError, RecursionError, SessionError) as error:
        raise SessionError(INCOMPLETE) from error


def guard(sessions, sid, generation, observation):
    if (sid in sessions.cancelled or sessions.stopping.is_set()
            or sessions.generations.get(sid) != generation
            or getattr(observation, 'memory_owner', None) != (sid, generation)
            or getattr(observation, 'final', True) is not None):
        raise SessionError(INCOMPLETE)
    row = sessions.db.execute('SELECT kind FROM launches WHERE id=? AND session_id=?',
                              (generation, sid)).fetchone()
    if row is None or row[0] != 'native':
        raise SessionError(INCOMPLETE)


def identity(sessions, sid, generation, event_id, data, current_scope):
    from . import memory_draft
    digest = fingerprint(data)
    if type(event_id) is not int or event_id < 1:
        raise SessionError(INCOMPLETE)
    row = sessions.db.execute('SELECT data FROM events WHERE id=? AND session_id=?', (event_id, sid)).fetchone()
    try:
        event = json.loads(row[0]) if row else None
        if (not isinstance(event, dict) or event.get('launch_id') != generation
                or event.get('kind') not in ('text', 'result')):
            raise ValueError()
        draft = memory_draft.parse(event.get('text'))
        if draft is None or fingerprint(draft) != digest:
            raise ValueError()
    except (TypeError, ValueError, RecursionError) as error:
        raise SessionError(CONFLICT) from error
    return {'session': sid, 'generation': generation, 'event': event_id,
            'input_digest': digest, 'scope': current_scope}


def prepared(value, task_id, result_id, revision, owner, reason):
    """Check the native versioned safe-wire reply before retaining its body."""
    from . import memory_draft
    fields = {'profile', 'task_id', 'result_id', 'revision', 'owner', 'reason',
              'attestation', 'scope', 'digest', 'request'}
    if (not isinstance(value, dict) or set(value) != fields or value['profile'] != PROFILE
            or value['task_id'] != task_id or value['result_id'] != result_id
            or type(value['revision']) is not int or value['revision'] != revision
            or value['owner'] != owner or value['reason'] != reason or value['attestation'] != 'agent'
            or not isinstance(task_id, str) or not re.fullmatch(
                r'[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}', task_id)
            or any(not isinstance(value[k], str) or not re.fullmatch(r'[0-9a-f]{64}', value[k])
                   for k in ('scope', 'digest'))):
        raise SessionError(INCOMPLETE)
    request = value['request']
    if (not isinstance(request, dict) or set(request) - {'progress', 'next_steps', 'learnings', 'verified'}
            or request.get('verified') is not True or 'learnings' not in request):
        raise SessionError(INCOMPLETE)
    draft = memory_draft.submission(request)
    normalized = {'progress': draft['progress'] or None,
                  'next_steps': draft['next_steps'] if 'next_steps' in request else None,
                  'learnings': draft['learnings']}
    wire = {'learnings': draft['learnings'], 'verified': True}
    for field in ('progress', 'next_steps'):
        if normalized[field] is not None:
            wire[field] = normalized[field]
    digest = hashlib.sha256(json.dumps({'task': task_id, **normalized}, sort_keys=True,
                                       ensure_ascii=False).encode('utf-8')).hexdigest()
    if wire != request or digest != value['digest']:
        raise SessionError(INCOMPLETE)
    encoded(value)
    return json.loads(encoded(value))


def insert(sessions, identity, value, skipped_count):
    """The caller holds Sessions.lock; only the insertion owner may first-write."""
    sid, generation = identity['session'], identity['generation']
    binding = {'profile': PROFILE, 'identity': identity, 'prepared': value, 'skipped_count': skipped_count}
    text = encoded(binding)
    old = lookup(sessions, sid, generation)
    if old is not None:
        # The original draft/context identity owns selection too: a contender
        # may have refiltered after files changed. Never replace those omissions
        # or the initial revision; native replay checks the retained scope/proof.
        if old['identity'] != identity:
            raise SessionError(CONFLICT)
        return old, False
    # Durable identities are history, not simultaneous worker reservations.
    # Counting this insert-only table as capacity permanently blocks new runs.
    # Retain every ambiguous/settled binding; per-entry admission and ownership
    # stay enforced without inventing settlement or evicting replay evidence.
    sessions.db.execute('INSERT INTO memory_result_attempts(session_id,generation,binding) VALUES(?,?,?)',
                        (sid, generation, text))
    sessions.db.commit()
    return binding, True
