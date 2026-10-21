"""Bind a browser session to native Brain state, and run its memory.

By default memory works unattended: each message is the retrieval query, the
context goes straight into the launch, and what the run established is saved
when it finishes. `review` restores the approved-snapshot flow, where a person
reviews each capsule before its turn; the run's draft is saved the same way.
"""
from __future__ import annotations

import json
import hashlib
import math
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath
import re
import time
import uuid

from . import cost_observation, memory_draft, memory_attempts
from .automatic_query import AutomaticQueryError, sanitize_automatic_query
from .knowledge import OLDER_RUNTIME, STALE_RECORD, KnowledgeBusy, KnowledgeRefusal, _path, _text
from .sessions import CAPSULE_LIMIT, SessionError, read_context, normalize_funnel_diagnosis, normalize_ordinary_receipt

UUID4 = re.compile(r'[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}')
TASK_ID = re.compile(r'[^\W_][\w\u0300-\u036f./#+-]{0,199}')  # letters and digits of any script
FIELDS = ('bank', 'task_id', 'query', 'create', 'goal', 'record_id', 'review', 'auto')
SCOPED_ACTIONS = {'brain-update', 'rebind', 'complete', 'brain-create',
                  'promote-propose', 'promote-review', 'promote-apply'}
# The Harness's own bookkeeping beside the options, which preparing a turn keeps.
KEPT = ('remembered',)
SOURCE_PROVENANCE_PROFILE = 'local-source-provenance-v1'
QUERY_BYTES = 4000
GOAL_CHARACTERS = 200


def _plain(text):
    """Text without the control characters every Brain field refuses."""
    return ''.join(char if char in '\n\t' or 31 < ord(char) != 127 else ' ' for char in text)


def message_query(prompt):
    """A message as a retrieval query. The runtime distills it; this only bounds it."""
    try:
        safe = sanitize_automatic_query(_plain(prompt))
    except AutomaticQueryError as error:
        raise SessionError(str(error)) from error
    return safe.encode('utf-8')[:QUERY_BYTES].decode('utf-8', 'ignore').strip()


def automatic_goal(prompt):
    """The first line of the first message: what the session was started to do."""
    line = next((line for line in message_query(prompt).splitlines() if line.strip()), 'Work on the project.')
    line = ' '.join(line.split())
    return line if len(line) <= GOAL_CHARACTERS else line[:GOAL_CHARACTERS - 1].rstrip() + '…'


def automatic_task_id(goal):
    """A readable task ID for a session that named none, unique per session."""
    slug = '-'.join(re.findall(r'[a-z0-9]+', goal.lower())[:5])[:40].strip('-')
    return f'harness/{slug or "task"}-{uuid.uuid4().hex[:6]}'


class TaskContext:
    def __init__(self, knowledge, *, wait=0, observation=None, guard=None):
        """`wait` is how long, in seconds, an operation outlasts another one holding the
        knowledge lock. A person's request is refused at once and can be repeated; the run
        worker's memory has nobody to repeat it, so the worker waits instead."""
        self.knowledge = knowledge
        self.sessions = knowledge.sessions
        self.wait = wait
        self.observation, self.guard = observation, guard

    def _knowledge(self, operation, *arguments, **options):
        data = arguments[1] if operation == 'run' and len(arguments) > 1 and isinstance(arguments[1], dict) else {}
        measured = cost_observation.safe(self.observation, 'operation', operation, data.get('action'))
        if measured is not None and operation in ('run', 'inspect'):
            options['_io_observer'] = measured
        if self.guard is not None and operation == 'run':
            options.setdefault('_guard', self.guard)
        deadline = time.monotonic() + self.wait
        result = None
        try:
            while True:
                try:
                    result = getattr(self.knowledge, operation)(*arguments, **options)
                    return result
                except KnowledgeBusy:
                    cost_observation.safe(measured, 'retry')
                    if time.monotonic() >= deadline:
                        raise
                    time.sleep(.2)
        finally:
            cost_observation.safe(measured, 'finish', result)

    def retire_cursor_rule(self, session):
        """A host capsule supersedes only a prospectively proven local rule."""
        try:
            response = self._knowledge('run', session['project_id'],
                {'action': 'cursor-rule-retire', 'bank': session['brain']['bank']},
                _root=self.sessions._workspace(session))
            value = response.get('result') if isinstance(response, dict) and response.get('ok') is True else None
            if not isinstance(value, dict) or value.get('profile') != 'cursor-rule-custody-v1':
                return {'state': 'unavailable', 'bytes_removed': None}
            state = value.get('state')
            removed = value.get('bytes_removed')
            if state not in ('removed', 'preserved', 'absent', 'attached', 'incomplete') or type(removed) is not int or not 0 <= removed <= 16384:
                return {'state': 'unavailable', 'bytes_removed': None}
            if (state in ('preserved', 'absent', 'attached') and removed != 0) or (state == 'removed' and removed == 0):
                return {'state': 'unavailable', 'bytes_removed': None}
            return {'state': state, 'bytes_removed': removed}
        except (SessionError, OSError, ValueError, TypeError, KeyError):
            # No raw error/rule body enters the session or native prompt.
            return {'state': 'unavailable', 'bytes_removed': None}

    @staticmethod
    def validate_options(data, prompt=None):
        """Validate a session's memory options; `prompt` is the message that opens the session.

        `auto` names no task: one is created from the first message. Without `review`
        each message is the retrieval query, so a query is only required for review.
        """
        if not isinstance(data, dict) or set(data) - set(FIELDS):
            raise SessionError('Invalid task context options.')
        review, auto = data.get('review', False), data.get('auto', False)
        if type(review) is not bool or type(auto) is not bool:
            raise SessionError('Memory options must be booleans.')
        if auto:
            if set(data) & {'task_id', 'goal', 'record_id', 'create'}:
                raise SessionError('An automatic task takes its ID and goal from the first message.')
            if not isinstance(prompt, str) or not prompt.strip():
                raise SessionError('An automatic task needs the first message.')
            goal = automatic_goal(prompt)
            data = {**data, 'task_id': automatic_task_id(goal), 'goal': goal, 'create': True}
        if 'query' not in data and not review and isinstance(prompt, str):
            data = {**data, 'query': message_query(prompt)}
        bank = _path(data.get('bank'))
        if bank != 'memory-bank' and not bank.endswith('/memory-bank'):
            raise SessionError('Select a project Memory Bank.')
        task_id = _text(data.get('task_id'), 'task ID', 200).strip()
        if not TASK_ID.fullmatch(task_id) or '..' in task_id.split('/'):
            raise SessionError('Enter a valid Brain task ID.')
        if review:
            query = _text(data.get('query'), 'context query', QUERY_BYTES)
        else:
            value = data.get('query')
            if not isinstance(value, str) or len(value.encode('utf-8')) > QUERY_BYTES:
                raise SessionError('Enter a bounded context query.')
            query = message_query(value)
        create = data.get('create', False)
        if type(create) is not bool:
            raise SessionError('The create-task option must be a boolean.')
        result = {'bank': bank, 'task_id': task_id, 'query': query, 'create': create, 'review': review}
        if create or 'goal' in data:
            result['goal'] = _text(data.get('goal'), 'task goal', 16000)
        if 'record_id' in data:
            record_id = _text(data['record_id'], 'task record ID', 36).strip()
            if not UUID4.fullmatch(record_id) or create:
                raise SessionError('Choose an existing task UUID or create a new task.')
            result['record_id'] = record_id
        return result

    def _bound(self, session):
        brain = session.get('brain')
        if not isinstance(brain, dict):
            raise SessionError('This session has no linked Brain task.')
        # Every session linked before memory ran unattended was linked for review.
        options = self.validate_options({'review': True, **{key: brain[key] for key in FIELDS if key in brain}})
        workspace = self.sessions._workspace(session)
        info = self.knowledge.info(session['project_id'], options['bank'], _root=workspace)
        if not info['runtime_available'] or info['mode'] != 'governed':
            raise SessionError('The session workspace requires an installed governed Brain runtime.')
        return brain, options, workspace, info

    def _call(self, session, options, workspace, action, *, _guard=None, **fields):
        result = self._knowledge('run', session['project_id'], {'bank': options['bank'], 'action': action, **fields},
                                 _root=workspace, **({'_guard': _guard} if _guard is not None else {}))
        if not result.get('ok') or not isinstance(result.get('result'), dict):
            raise KnowledgeRefusal(result.get('error') or 'The Brain runtime did not complete this operation.',
                                   result.get('error_code'))
        return result['result']

    @staticmethod
    def _validate_capsule(capsule, task):
        if (not isinstance(capsule, dict) or capsule.get('task_id') != task.get('external_id')
                or capsule.get('task_uuid') != task.get('id')
                or type(capsule.get('task_revision')) is not int
                or capsule['task_revision'] != task.get('revision')
                or not isinstance(capsule.get('working'), dict)
                or capsule['working'].get('task_id') != task.get('external_id')):
            # A capsule served without the task's state says why in its
            # warnings - the task's record gone or unreadable, say.
            warnings = capsule.get('warnings') if isinstance(capsule, dict) else None
            reasons = ' '.join(item for item in warnings if isinstance(item, str)) if isinstance(warnings, list) else ''
            raise SessionError('Retrieved context does not match the linked task and revision. '
                               + (reasons[:400] + ' ' if reasons else '') + 'Prepare it again.')
        for layer, limit in (('procedural', 2), ('semantic', 3), ('episodic', 3)):
            values = capsule.get(layer)
            if not isinstance(values, list) or len(values) > limit or any(not isinstance(item, dict) for item in values):
                raise SessionError('The runtime returned an invalid task capsule.')
        if capsule.get('warnings'):
            raise SessionError('Context refresh reported warnings. Resolve the runtime warnings before preparing this session.')
        try:
            encoded = json.dumps(capsule, ensure_ascii=False, separators=(',', ':'), allow_nan=False)
            encoded.encode('utf-8')
        except (ValueError, TypeError, UnicodeError, RecursionError) as error:
            raise SessionError('The runtime returned an invalid task capsule.') from error
        if len(encoded) > CAPSULE_LIMIT:
            raise SessionError(f'The task capsule exceeds the native {CAPSULE_LIMIT}-character limit.')

    @staticmethod
    def _material(capsule):
        # Manifest UUIDs, timings, token estimates, and repeat-gate telemetry
        # change between identical reads and are not approved source content.
        def content(value):
            if isinstance(value, dict):
                return {key: content(item) for key, item in value.items()
                        if key not in ('estimated_tokens', 'score', 'rank')}
            return [content(item) for item in value] if isinstance(value, list) else value
        return content({key: capsule.get(key) for key in (
            'query', 'task_id', 'task_uuid', 'task_revision', 'working',
            'procedural', 'semantic', 'episodic', 'selected',
        )})

    @staticmethod
    def _visible_path(value, home=None):
        """A capsule path: relative to the project, or - for an attached
        accelerator - an absolute path into its edition in the clone."""
        if home is not None and isinstance(value, str) and value.startswith('/'):
            path = PurePosixPath(value)
            if '..' not in path.parts and path.is_relative_to(PurePosixPath(Path(home).as_posix())):
                return value
        return _path(value)

    @staticmethod
    def _source_hashes(root, capsule, home=None, *, provenance=None, allow_local=False):
        manifest = capsule.get('manifest')
        if manifest is None and allow_local:
            return TaskContext._inline_source_hashes(capsule, provenance, home)
        if not isinstance(manifest, str) or not re.fullmatch(
            r'(?:project-brain/control|memory-bank/local)/retrieval-manifests/'
            r'[0-9a-f-]{36}\.json', manifest,
        ):
            raise SessionError('The task capsule has no valid source manifest.')
        document = read_context(root, manifest, 512 * 1024)
        if document is None or document[0] > 512 * 1024:
            raise SessionError('The task context manifest is unavailable.')
        try:
            metadata = json.loads(document[1])
        except (ValueError, RecursionError) as error:
            raise SessionError('The task context manifest is invalid.') from error
        if (not isinstance(metadata, dict) or metadata.get('task_id') != capsule['task_uuid']
                or metadata.get('task_revision') != capsule['task_revision']
                or not isinstance(metadata.get('selected'), list)):
            raise SessionError('The task context manifest has an invalid task binding.')
        # Full-source hashes catch edits outside the bounded snippets displayed
        # in the capsule; omitted candidates must not invalidate the approval.
        visible = set()
        for layer in ('procedural', 'semantic', 'episodic', 'selected'):
            for item in capsule.get(layer, []):
                if isinstance(item, dict) and isinstance(item.get('path'), str):
                    visible.add(TaskContext._visible_path(item['path'], home))
        hashes = {}
        for item in metadata['selected']:
            if not isinstance(item, dict) or item.get('path') not in visible:
                continue
            digest = item.get('source_hash')
            if not isinstance(digest, str) or not re.fullmatch(r'[0-9a-f]{64}', digest):
                raise SessionError('A selected context source has no valid fingerprint.')
            hashes[item['path']] = digest
        if set(hashes) != visible:
            raise SessionError('The source manifest does not cover the approved context.')
        return hashes

    @staticmethod
    def _inline_source_hashes(capsule, provenance, home=None):
        """Explicit local transport consistency; never adopt/rehash later files."""
        required = {'schema_version', 'profile', 'scope', 'publication', 'task_uuid',
                    'task_revision', 'capsule_sha256', 'selected'}
        if (type(provenance) is not dict or set(provenance) != required
                or type(provenance['schema_version']) is not int or provenance['schema_version'] != 1
                or provenance['profile'] != SOURCE_PROVENANCE_PROFILE
                or provenance['scope'] != 'local' or provenance['publication'] != 'unconfirmed'
                or capsule.get('manifest_scope') != 'local'
                or not isinstance(capsule.get('task_uuid'), str) or not UUID4.fullmatch(capsule['task_uuid'])
                or provenance['task_uuid'] != capsule['task_uuid']
                or type(capsule.get('task_revision')) is not int or capsule['task_revision'] < 1
                or type(provenance['task_revision']) is not int or provenance['task_revision'] != capsule['task_revision']
                or not isinstance(provenance['capsule_sha256'], str)
                or not re.fullmatch(r'[0-9a-f]{64}', provenance['capsule_sha256'])
                or type(provenance['selected']) is not list or len(provenance['selected']) > 32):
            raise SessionError('The local source provenance is unavailable or invalid.')
        try:
            encoded = json.dumps(provenance, ensure_ascii=False, separators=(',', ':'), allow_nan=False).encode('utf-8')
            frame = json.dumps(capsule, ensure_ascii=False, separators=(',', ':'), sort_keys=True, allow_nan=False).encode('utf-8')
        except (ValueError, TypeError, UnicodeError, RecursionError) as error:
            raise SessionError('The local source provenance is invalid.') from error
        if len(encoded) > 65536 or hashlib.sha256(frame).hexdigest() != provenance['capsule_sha256']:
            raise SessionError('The local source provenance does not match the capsule.')
        visible = set()
        for layer in ('procedural', 'semantic', 'episodic', 'selected'):
            values = capsule.get(layer, [])
            if type(values) is not list:
                raise SessionError('The local source provenance has invalid capsule coverage.')
            for item in values:
                if isinstance(item, dict) and isinstance(item.get('path'), str):
                    visible.add(TaskContext._visible_path(item['path'], home))
        hashes = {}
        for item in provenance['selected']:
            if type(item) is not dict or set(item) != {'path', 'source_hash'}:
                raise SessionError('The local source provenance has invalid selected sources.')
            leaf = TaskContext._visible_path(item['path'], home)
            digest = item['source_hash']
            if (leaf in hashes or not isinstance(digest, str)
                    or not re.fullmatch(r'[0-9a-f]{64}', digest)):
                raise SessionError('The local source provenance has invalid selected sources.')
            hashes[leaf] = digest
        if set(hashes) != visible:
            raise SessionError('The local source provenance does not cover the approved context.')
        return hashes

    def _provider_hints(self, session, options, workspace, info):
        """Once before KnowledgeBusy retries; trusted operator callback only."""
        provider = getattr(self.knowledge, 'source_hint_provider', None)
        if provider is None:
            return None
        from .hindsight_recall import ReferenceOutcome
        started, calls, outcome, references, transport = time.monotonic(), 0, 'unavailable', 0, None
        def active():
            if getattr(self.observation, 'final', None) is not None:
                return False
            if self.guard is not None:
                try:
                    self.guard()
                except SessionError:
                    return False
            return True
        try:
            if not active():
                outcome = 'cancelled'
                raise SessionError('The memory generation is no longer active.')
            if not self.knowledge._supports_source_hints(session['project_id'], info, workspace):
                outcome = 'unsupported'
                return None
            if not active():
                outcome = 'cancelled'
                raise SessionError('The memory generation is no longer active.')
            calls = 1
            try:
                value = provider(options['query'], active=active, project_id=session['project_id'],
                                 bank=options['bank'], workspace=str(workspace))
            except Exception:
                value = None
            if type(value) is ReferenceOutcome and type(value.diagnostics) is dict:
                transport = value.diagnostics
            # A raising generation guard cannot be converted to local fallback.
            if not active():
                outcome = 'cancelled'
                raise SessionError('The memory generation is no longer active.')
            if type(value) is not ReferenceOutcome:
                outcome = 'invalid'
                return None
            try:
                payload = value.native_payload()
                if self.knowledge._source_hints_json(payload) is None:
                    outcome = 'invalid'
                    return None
            except (ValueError, TypeError, UnicodeError):
                outcome = 'invalid'
                return None
            references = len(payload['references'])
            outcome = 'resolved' if references else 'empty'
            return payload if references else None
        finally:
            cost_observation.safe(self.observation, 'source_hints', provider_calls=calls, outcome=outcome,
                                  seconds=time.monotonic() - started, references=references, transport=transport)

    def _retrieve(self, session, options, workspace, task, info, ephemeral, diagnosis=None):
        # Every retrieval here is one a person or the session asked for. Bypass only
        # the repeat-skip heuristic; native privacy, lifecycle and freshness filters stay.
        # Unattended memory retrieves through `refresh`, which distills the whole
        # message rather than its first words and leaves out the instruction files
        # the provider loads by itself.
        automatic = not options.get('review')
        hints = self._provider_hints(session, options, workspace, info) if automatic else None
        capability = getattr(self.knowledge, 'supports_provenance_response', None)
        transport = bool(ephemeral and callable(capability)
                         and capability(session['project_id'], info, _root=workspace))
        response = self._knowledge('run', session['project_id'], {
            'action': 'refresh' if automatic else 'retrieve', 'bank': options['bank'],
            'task_id': options['task_id'], 'query': options['query'],
        }, _root=workspace, _ephemeral=ephemeral, _gate='off',
            _host=session.get('provider') if automatic else None,
            **({'_provenance_response': True} if transport else {}),
            **({'_source_hints': hints} if hints is not None else {}))
        if not response.get('ok'):
            raise SessionError(response.get('error') or 'Task context could not be retrieved.')
        capsule = response.get('result')
        provenance = None
        if transport:
            if not isinstance(capsule, dict) or capsule.get('response_profile') != SOURCE_PROVENANCE_PROFILE:
                raise SessionError('The runtime returned an unsupported source provenance response.')
            provenance = capsule.get('source_provenance')
            if not automatic:
                capsule = capsule.get('capsule')
        text = None
        if automatic:
            result = capsule if isinstance(capsule, dict) else {}
            capsule = result.get('capsule')
            # The runtime's own render: what the prompt carries in place of the JSON.
            text = result.get('capsule_text') if isinstance(result.get('capsule_text'), str) else None
            if not isinstance(capsule, dict):
                warnings = [item for item in result.get('warnings') or [] if isinstance(item, str)]
                raise SessionError('; '.join(warnings)[:500] or 'Task context could not be retrieved.')
        self._validate_capsule(capsule, task)
        # Attached, the manifest is in the accelerator's state and tooling sources
        # are absolute paths into the clone.
        layout = self.knowledge.layout(session['project_id'], workspace)
        home = layout['attached']['home'] if layout['attached'] else None
        hashes = self._source_hashes(layout['folder'] / info['root'], capsule, home,
                                     provenance=provenance, allow_local=transport and ephemeral)
        if automatic and type(diagnosis) is dict:
            for key, normalizer in (('funnel_diagnosis', normalize_funnel_diagnosis),
                                    ('ordinary_receipt', normalize_ordinary_receipt)):
                try:
                    child = normalizer(result.get(key))
                except Exception:
                    child = None
                if child is not None:
                    diagnosis[key] = child
        return capsule, hashes, text

    def prepare(self, session):
        brain, options, workspace, info = self._bound(session)
        kept = {key: brain[key] for key in KEPT if key in brain}
        if options['create']:
            task = self._call(session, options, workspace, 'start', task_id=options['task_id'], goal=options['goal'])
        else:
            fields = {'task_id': options['task_id']}
            if options.get('record_id'):
                fields['record_id'] = options['record_id']
            task = self._call(session, options, workspace, 'rebind', **fields)
        record_id = task.get('task_uuid')
        if not isinstance(record_id, str) or not UUID4.fullmatch(record_id):
            raise SessionError('The Brain runtime did not return the task identity.')
        options.update(create=False, record_id=record_id)
        # Persist the native identity before retrieval can fail. Retrying must
        # rebind this task rather than attempt to create another task.
        pending = {**options, **kept, 'approved': False, 'context_id': None, 'capsule': None}
        if hasattr(self.sessions, '_save_brain'):
            with self.sessions.lock:
                # Cancellation can queue a new query while native start is
                # finishing. Preserve that request, but retain the task UUID
                # so its retry cannot create a second task.
                current = self.sessions.get(session['id'])['brain']
                pending['query'] = current['query']
                self.sessions._save_brain(session['id'], pending)
        task = self._knowledge('inspect', session['project_id'], options['bank'], _root=workspace,
                               task_id=record_id, task_only=True)['task']
        # An approved snapshot is shared provenance; a turn's own retrieval is not, and
        # one Git-tracked manifest per message would flood the project's history.
        diagnosis = {}
        capsule, hashes, text = self._retrieve(session, options, workspace, task, info, not options.get('review'), diagnosis)
        return {**options, **kept, 'context_id': str(uuid.uuid4()), 'capsule': capsule, 'capsule_text': text,
                'task': task, 'source_hashes': hashes, 'approved': False, **diagnosis}

    def ensure_fresh(self, session):
        brain, options, workspace, info = self._bound(session)
        saved_task, saved_capsule = brain.get('task'), brain.get('capsule')
        if (not isinstance(saved_task, dict) or not isinstance(saved_capsule, dict)
                or not isinstance(brain.get('context_id'), str) or not UUID4.fullmatch(brain['context_id'])):
            raise SessionError('Prepare the linked task context before running this session.')
        task = self._knowledge('inspect', session['project_id'], options['bank'], _root=workspace,
                               task_id=options.get('record_id') or options['task_id'], task_only=True)['task']
        if (task.get('id') != saved_task.get('id') or task.get('revision') != saved_task.get('revision')
                or task.get('external_id') != options['task_id'] or task.get('status') in ('completed', 'cancelled')):
            raise SessionError('The linked task changed. Prepare and approve its context again.')
        capsule, hashes, _ = self._retrieve(session, options, workspace, task, info, True)
        if hashes != brain.get('source_hashes') or self._material(capsule) != self._material(saved_capsule):
            raise SessionError('Task context sources changed. Prepare and approve the updated context before running.')
        return True

    def info(self, session):
        brain, options, workspace, _ = self._bound(session)
        result = self._knowledge('inspect', session['project_id'], options['bank'], _root=workspace,
                                 task_id=options.get('record_id') or options['task_id'])
        return {**result, 'context_id': brain.get('context_id'), 'approved': brain.get('approved') is True,
                'memory_draft': memory_draft.latest(self.sessions, session['id'])}

    def save_memory(self, session, data, *, automatic=False, known=(), generation=None, origin_event_id=None):
        """Freeze safe automatic requests; an existing attempt needs exact receipt proof."""
        old = memory_attempts.lookup(self.sessions, session['id'], generation) if generation is not None else None
        owner = getattr(self.observation, 'memory_owner', None)
        if automatic and owner is not None and (
                generation is None or owner != (session['id'], generation)
                or getattr(self.observation, 'final', True) is not None
                or origin_event_id is None):
            raise SessionError(memory_attempts.INCOMPLETE)
        owned = (automatic and generation is not None and origin_event_id is not None
                 and getattr(self.observation, 'memory_owner', None) == (session['id'], generation)
                 and getattr(self.observation, 'final', True) is None)
        if old is not None and not owned:
            raise SessionError(memory_attempts.INCOMPLETE)
        if not owned:
            return self._save_memory_unbound(session, data, automatic=automatic, known=known, generation=generation)
        _, options, workspace, _ = self._bound(session)
        alias_task = None
        if not isinstance(options.get('record_id'), str) or not UUID4.fullmatch(options['record_id']):
            if old is not None:
                stored_scope = old['identity'].get('scope')
                alias_task = stored_scope.get('task') if isinstance(stored_scope, dict) else None
            else:
                task = self._knowledge('inspect', session['project_id'], options['bank'],
                    _root=workspace, task_id=options['task_id'], task_only=True)['task']
                alias_task = task.get('id')
            if not isinstance(alias_task, str) or not UUID4.fullmatch(alias_task):
                raise SessionError(memory_attempts.INCOMPLETE)
        def scope(current, current_options, current_workspace):
            value = memory_attempts.scope(self.knowledge, current, current_options, current_workspace)
            if not isinstance(value['task'], str) or not UUID4.fullmatch(value['task']):
                # Freeze the canonical lookup before guarded operations. A
                # nested inspect would contend with that same knowledge lock.
                # The original UUID and alias selector own every later repeat.
                if alias_task is None:
                    raise SessionError(memory_attempts.INCOMPLETE)
                value['task'] = alias_task
                value['task_alias'] = current_options['task_id']
            return value
        with self.sessions.lock:
            memory_attempts.guard(self.sessions, session['id'], generation, self.observation)
            identity = memory_attempts.identity(self.sessions, session['id'], generation, origin_event_id, data,
                scope(session, options, workspace))
        def guard():
            with self.sessions.lock:
                memory_attempts.guard(self.sessions, session['id'], generation, self.observation)
                current = self.sessions.get(session['id'])
                _, current_options, current_workspace, _ = self._bound(current)
                checked = memory_attempts.identity(self.sessions, session['id'], generation, origin_event_id, data,
                    scope(current, current_options, current_workspace))
                if checked != identity:
                    raise SessionError(memory_attempts.CONFLICT)
        if old is not None:
            if old['identity'] != identity:
                raise SessionError(memory_attempts.CONFLICT)
            return self._replay_memory(session, options, workspace, old, guard)
        return self._save_memory_unbound(session, data, automatic=automatic, known=known,
                                          generation=generation, _binding=(identity, guard))

    def _replay_memory(self, session, options, workspace, binding, guard):
        value = binding['prepared']
        # Validate even retained local data before any runtime call; corruption is incomplete.
        value = memory_attempts.prepared(value, binding['identity']['scope']['task'], value.get('result_id'),
                                         value.get('revision'), binding['identity']['scope']['owner'],
                                         memory_draft.AUTOMATIC_REASON)
        skipped = [{'reason': 'An omission from the original automatic draft was retained.'}
                   for _ in range(binding['skipped_count'])]
        try:
            outcome = self._call(session, options, workspace, 'replay-result', _guard=guard,
                task_id=value['task_id'], result_id=value['result_id'], revision=value['revision'],
                request=value['request'], reason=value['reason'], attestation=value['attestation'],
                prepared_scope=value['scope'])
        except SessionError:
            # Unsupported/absent/corrupt proof can never become a fresh write or legacy chain.
            return {'ok': False, 'saved': {'task': None, 'records': [], 'promotion': None},
                    'skipped': skipped, 'error': memory_attempts.INCOMPLETE}
        return self._memory_outcome(outcome, skipped)

    def _save_memory_unbound(self, session, data, *, automatic=False, known=(), generation=None, _binding=None,
                             _compat_guard=None):
        """Record a memory draft through the runtime's own `record-result`.

        One call carries the task's progress and next steps and each kept
        learning, which becomes a finding or decision checked against its
        sources - resolved or accepted, with its consequence as the content
        promotion carries - and then runs promotion under the runtime's own
        rules. The call is replayable: a learning is named by its content, so
        a retry after an interruption finds the record instead of writing
        another, and the run's result ID marks the task update as done. A
        runtime older than the command gets the previous chain.

        `automatic` is the unattended save at the end of a run: the agent's draft
        as written, with any learning whose sources are not in the workspace
        left out rather than failing the rest. A prior title cache (`known`,
        retained as a compatibility argument) cannot reject corrections;
        record-result decides equality and promotion retries. Its learnings
        are attested by the agent and no person
        reviewed them, which the records say (`attestation: agent`) and the
        capsule repeats next to them. Promotion still follows the project's
        `automatic_promotion` setting.
        """
        _, options, workspace, info = self._bound(session)
        if _binding is not None:
            identity, bound_guard = _binding
            with self.sessions.lock:
                bound_guard()
                concurrent = memory_attempts.lookup(self.sessions, session['id'], generation)
            if concurrent is not None:
                if concurrent['identity'] != identity:
                    raise SessionError(memory_attempts.CONFLICT)
                return self._replay_memory(session, options, workspace, concurrent, bound_guard)
        root = Path(workspace) / info['root']
        original = data
        skipped = []
        if automatic:
            data, skipped = memory_draft.usable(root, data, known)
            if not (data['progress'] or data['next_steps'] or data['learnings']):
                if _binding is not None:
                    with self.sessions.lock:
                        bound_guard()
                        concurrent = memory_attempts.lookup(self.sessions, session['id'], generation)
                    if concurrent is not None:
                        if concurrent['identity'] != identity:
                            raise SessionError(memory_attempts.CONFLICT)
                        return self._replay_memory(session, options, workspace, concurrent, bound_guard)
                return {'ok': True, 'saved': {'task': None, 'records': [], 'promotion': None},
                        'skipped': skipped, 'error': None}
        draft = memory_draft.submission(data)
        memory_draft.check_sources(root, draft)
        task = self._knowledge('inspect', session['project_id'], options['bank'], _root=workspace,
                               task_id=options.get('record_id') or options['task_id'], task_only=True)['task']
        if task.get('status') in ('completed', 'cancelled'):
            raise SessionError('The linked task is finished. Link an active task to save more.')
        reason = (memory_draft.AUTOMATIC_REASON if automatic else 'Saved from a reviewed Harness session')
        def guard():
            sid = session['id']
            if (sid in self.sessions.cancelled or self.sessions.stopping.is_set()
                    or self.sessions.generations.get(sid) != generation):
                raise SessionError('Memory save was cancelled; no further writes were started.')
        checked = {'_guard': guard} if generation is not None else {}
        if _compat_guard is not None:
            checked = {'_guard': _compat_guard}
        request = {'learnings': [{'type': item['type'], 'title': item['title'], 'consequence': item['consequence'],
                                  'sources': item['sources'],
                                  **({'expected_revision': item['expected_revision']}
                                     if 'expected_revision' in item else {})} for item in draft['learnings']],
                   'verified': True}
        if draft['progress'] or draft['next_steps']:
            # The draft is the current plan, including an empty completed
            # plan; remove steps the run may have finished.
            request['next_steps'] = draft['next_steps']
            if draft['progress']:
                request['progress'] = draft['progress']
        # One run, one result: a replay of this save is recognised as such.
        result_id = f"harness-{session['id']}-{generation if generation is not None else uuid.uuid4().hex[:12]}"[:64]
        task_id = options['task_id']
        extra = {}
        bound_retries = 0
        if _binding is not None:
            identity, bound_guard = _binding
            for preparation in range(2):
                try:
                    value = self._call(session, options, workspace, 'prepare-result', _guard=bound_guard,
                        task_id=task['id'], result_id=result_id, revision=task['revision'], request=request,
                        attestation='agent', reason=reason)
                    value = memory_attempts.prepared(value, identity['scope']['task'], result_id,
                                                     task['revision'], identity['scope']['owner'], reason)
                    with self.sessions.lock:
                        bound_guard()
                        binding, first_owner = memory_attempts.insert(self.sessions, identity, value, len(skipped))
                    if not first_owner:
                        return self._replay_memory(session, options, workspace, binding, bound_guard)
                    request, task_id = value['request'], value['task_id']
                    checked = {'_guard': bound_guard}
                    extra = {'prepared_scope': value['scope']}
                    break
                except KnowledgeRefusal as error:
                    if str(error) == OLDER_RUNTIME:
                        # A fresh old-runtime attempt retains the original guarded save path.
                        def compatibility_guard():
                            bound_guard()
                            if memory_attempts.lookup(self.sessions, session['id'], generation) is not None:
                                raise SessionError(memory_attempts.INCOMPLETE)
                        with self.sessions.lock:
                            compatibility_guard()
                        _compat_guard = compatibility_guard
                        checked = {'_guard': compatibility_guard}
                        _binding = None
                        break
                    if error.code == 'prepared_stale_no_write' and preparation == 0:
                        bound_retries += 1
                        cost_observation.safe(self.observation, 'note', 'stale_revision_retries')
                        task = self._knowledge('inspect', session['project_id'], options['bank'], _root=workspace,
                            task_id=task['id'], task_only=True)['task']
                        continue
                    raise
        outcome = None
        for attempt in range(2):
            try:
                outcome = self._call(session, options, workspace, 'record-result', **checked,
                                     task_id=task_id, result_id=result_id, revision=task['revision'],
                                     request=request, attestation='agent' if automatic else 'person', reason=reason, **extra)
                break
            except SessionError as error:
                if _binding is None and str(error) == OLDER_RUNTIME:
                    return self._save_memory_chain(session, original, automatic=automatic, known=known,
                                                   generation=generation, _guard=_compat_guard)
                no_write_stale = (isinstance(error, KnowledgeRefusal) and error.code == 'prepared_stale_no_write')
                if attempt == 0 and no_write_stale and bound_retries == 0:
                    bound_retries += 1
                    cost_observation.safe(self.observation, 'note', 'stale_revision_retries')
                    # The bound lane's unique refusal proves no result writes started.
                    task = self._knowledge('inspect', session['project_id'], options['bank'], _root=workspace,
                                           task_id=options.get('record_id') or options['task_id'],
                                           task_only=True)['task']
                    continue
                return {'ok': False, 'saved': {'task': None, 'records': [], 'promotion': None},
                        'skipped': skipped, 'error': str(error)}
            except (KeyError, TypeError):
                return {'ok': False, 'saved': {'task': None, 'records': [], 'promotion': None},
                        'skipped': skipped, 'error': 'The runtime returned an unexpected record.'}
        return self._memory_outcome(outcome, skipped)

    @staticmethod
    def _memory_outcome(outcome, skipped):
        records = [{'id': item.get('id'), 'type': item.get('type'), 'title': item.get('title'),
                    'status': item.get('status'), 'state': item.get('state'),
                    **({'revision': item['revision']} if type(item.get('revision')) is int and item['revision'] > 0 else {}),
                    **({'reason': item['reason']} if item.get('reason') in
                       ('archived', 'kind-conflict', 'revision-conflict', 'not-authorized', 'person-or-unknown-attestation',
                        'original-image-unproven') else {})}
                   for item in outcome.get('records') or [] if isinstance(item, dict)]
        promotion = outcome.get('promotion') if isinstance(outcome.get('promotion'), dict) else None
        saved = {'task': outcome.get('task') if isinstance(outcome.get('task'), dict) else None,
                 'records': records, 'promotion': promotion}
        return {'ok': True, 'saved': saved, 'skipped': skipped, 'error': None}

    def _save_memory_chain(self, session, data, *, automatic=False, known=(), generation=None, _guard=None):
        """The command-by-command save, for a runtime older than `record-result`.

        Each command is atomic, the chain is not: a failure reports what was
        already saved, and a retry can write a learning twice.
        """
        _, options, workspace, info = self._bound(session)
        root = Path(workspace) / info['root']
        skipped = []
        if automatic:
            data, skipped = memory_draft.usable(root, data, known)
            if not (data['progress'] or data['next_steps'] or data['learnings']):
                return {'ok': True, 'saved': {'task': None, 'records': [], 'promotion': None},
                        'skipped': skipped, 'error': None}
        draft = memory_draft.submission(data)
        memory_draft.check_sources(root, draft)
        task = self._knowledge('inspect', session['project_id'], options['bank'], _root=workspace,
                               task_id=options.get('record_id') or options['task_id'], task_only=True)['task']
        if task.get('status') in ('completed', 'cancelled'):
            raise SessionError('The linked task is finished. Link an active task to save more.')
        saved = {'task': None, 'records': [], 'promotion': None}
        immediate = []
        reason = (memory_draft.AUTOMATIC_REASON if automatic else 'Saved from a reviewed Harness session')
        def guard():
            sid = session['id']
            if (sid in self.sessions.cancelled or self.sessions.stopping.is_set()
                    or self.sessions.generations.get(sid) != generation):
                raise SessionError('Memory save was cancelled; no further writes were started.')
        checked = {'_guard': guard} if generation is not None else {}
        if _guard is not None:
            checked = {'_guard': _guard}
        try:
            if draft['progress'] or draft['next_steps']:
                fields = {'record_id': task['id'], 'revision': task['revision'], 'reason': reason}
                if draft['progress']:
                    fields['progress'] = draft['progress']
                # The draft is the current plan, including an empty completed
                # plan; remove steps the run may have finished.
                fields.update(next_steps=draft['next_steps'], replace_next_steps=True)
                updated = self._call(session, options, workspace, 'brain-update', **checked, **fields)
                saved['task'] = {'id': updated.get('id'), 'revision': updated.get('revision')}
            for learning in draft['learnings']:
                created = self._call(session, options, workspace, 'brain-create', **checked,
                                     record_type=learning['type'],
                                     external_id=memory_draft.external_id(options['task_id'], learning['type']),
                                     title=learning['title'], goal=learning['consequence'],
                                     sources=learning['sources'], authority='observed' if automatic else 'verified')
                closed = self._call(session, options, workspace, 'brain-update', **checked, record_id=created['id'],
                                    revision=created['revision'], progress=learning['consequence'],
                                    transition='resolved' if learning['type'] == 'finding' else 'accepted',
                                    reason=reason, **({'authority': 'verified'} if automatic else {}))
                saved['records'].append({'id': closed.get('id'), 'type': closed.get('type'),
                                         'title': closed.get('title'), 'status': closed.get('status')})
                # The runtime promotes a record on the update that resolves it.
                at_once = closed.get('promotion') if isinstance(closed.get('promotion'), dict) else {}
                immediate.extend(item for item in at_once.get('promoted') or [] if isinstance(item, dict))
            if saved['records']:
                promotion = self._knowledge('run', session['project_id'], {'bank': options['bank'], 'action': 'promote-auto'},
                                            _root=workspace, **checked)
                saved['promotion'] = (promotion['result'] if promotion.get('ok') and isinstance(promotion.get('result'), dict)
                                      else {'error': promotion.get('error') or 'Automatic promotion did not run.'})
                if immediate:
                    later = saved['promotion'].get('promoted') if isinstance(saved['promotion'].get('promoted'), list) else []
                    saved['promotion'] = {**saved['promotion'], 'promoted': immediate + later}
                    saved['promotion'].pop('error', None)
        except (SessionError, KeyError, TypeError) as error:
            message = str(error) if isinstance(error, SessionError) else 'The runtime returned an unexpected record.'
            return {'ok': False, 'saved': saved, 'skipped': skipped, 'error': message}
        return {'ok': True, 'saved': saved, 'skipped': skipped, 'error': None}

    def checkpoint(self, session, since=None):
        """Run the turn checkpoint for a provider whose headless mode fires no Stop hook.

        A current report for this task means a Stop hook checkpointed it.
        Filesystem mtime alone cannot bind a shared report to this task.
        """
        _, options, workspace, info = self._bound(session)
        if since is not None:
            layout = self.knowledge.layout(session['project_id'], workspace)
            try:
                report = read_context(layout['folder'],
                    str(PurePosixPath(info['root']) / 'memory-bank/local/last-turn-report.json'), 65537)
                if report is not None and report[0] <= 65536 and '\ufffd' not in report[1]:
                    value = json.loads(report[1])
                    if (isinstance(value, dict) and type(value.get('schema_version')) is int
                            and value['schema_version'] == 1 and value.get('task_id') == options['task_id']
                            and isinstance(value.get('timestamp'), str)):
                        written = datetime.fromisoformat(value['timestamp'])
                        if written.tzinfo is None:
                            written = written.replace(tzinfo=timezone.utc)
                        stamp = written.timestamp()
                        if math.isfinite(since) and since <= stamp <= time.time():
                            return None
            except (OSError, ValueError, TypeError, OverflowError, RecursionError):
                pass
        return self._call(session, options, workspace, 'turn', task_id=options['task_id'])

    def run(self, session, data):
        _, options, workspace, _ = self._bound(session)
        if not isinstance(data, dict) or data.get('action') not in SCOPED_ACTIONS:
            raise SessionError('Select a supported linked-task operation.')
        if 'bank' in data:
            raise SessionError('The session Memory Bank cannot change.')
        action = data['action']
        fields = dict(data)
        if action == 'brain-update' and 'record_id' not in fields:
            if not options.get('record_id'):
                raise SessionError('Prepare the linked task before updating it.')
            fields['record_id'] = options['record_id']
        if action in ('complete', 'rebind'):
            if 'task_id' in fields and fields['task_id'] != options['task_id']:
                raise SessionError('This operation must use the linked task identity.')
            fields['task_id'] = options['task_id']
        if action == 'rebind':
            if options.get('record_id') and 'record_id' in fields and fields['record_id'] != options['record_id']:
                raise SessionError('The linked task record cannot change.')
            if options.get('record_id'):
                fields['record_id'] = options['record_id']
        response = self._knowledge('run', session['project_id'], {'bank': options['bank'], **fields}, _root=workspace)
        if action == 'rebind' and response.get('ok'):
            result = response.get('result')
            record_id = result.get('task_uuid') if isinstance(result, dict) else None
            if (not isinstance(record_id, str) or not UUID4.fullmatch(record_id)
                    or result.get('task_id') != options['task_id']
                    or fields.get('record_id', record_id) != record_id):
                raise SessionError('The Brain runtime did not return the linked task identity.')
            # Explicit rebind can recover an interrupted native start whose
            # commit succeeded before its UUID reached the session database.
            with self.sessions.lock:
                current = self.sessions.get(session['id'])['brain']
                self.sessions._save_brain(session['id'], {**current, 'create': False, 'record_id': record_id,
                    'approved': False, 'context_id': None, 'capsule': None, 'source_hashes': {}})
        return response
