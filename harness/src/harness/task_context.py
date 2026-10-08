"""Bind a browser session to native Brain state, and run its memory.

By default memory works unattended: each message is the retrieval query, the
context goes straight into the launch, and what the run established is saved
when it finishes. `review` restores the approved-snapshot flow, where a person
reviews each capsule before its turn; the run's draft is saved the same way.
"""
from __future__ import annotations

import json
from pathlib import Path, PurePosixPath
import re
import time
import uuid

from . import memory_draft
from .knowledge import KnowledgeBusy, _path, _text
from .sessions import CAPSULE_LIMIT, SessionError, read_context

UUID4 = re.compile(r'[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}')
TASK_ID = re.compile(r'[A-Za-z0-9][A-Za-z0-9._/-]{0,199}')
FIELDS = ('bank', 'task_id', 'query', 'create', 'goal', 'record_id', 'review', 'auto')
SCOPED_ACTIONS = {'brain-update', 'rebind', 'complete', 'brain-create',
                  'promote-propose', 'promote-review', 'promote-apply'}
# The Harness's own bookkeeping beside the options, which preparing a turn keeps.
KEPT = ('remembered',)
QUERY_BYTES = 4000
GOAL_CHARACTERS = 200


def _plain(text):
    """Text without the control characters every Brain field refuses."""
    return ''.join(char if char in '\n\t' or 31 < ord(char) != 127 else ' ' for char in text)


def message_query(prompt):
    """A message as a retrieval query. The runtime distills it; this only bounds it."""
    return _plain(prompt).strip().encode('utf-8')[:QUERY_BYTES].decode('utf-8', 'ignore').strip()


def automatic_goal(prompt):
    """The first line of the first message: what the session was started to do."""
    line = next((line for line in _plain(prompt).splitlines() if line.strip()), '')
    line = ' '.join(line.split())
    return line if len(line) <= GOAL_CHARACTERS else line[:GOAL_CHARACTERS - 1].rstrip() + '…'


def automatic_task_id(goal):
    """A readable task ID for a session that named none, unique per session."""
    slug = '-'.join(re.findall(r'[a-z0-9]+', goal.lower())[:5])[:40].strip('-')
    return f'harness/{slug or "task"}-{uuid.uuid4().hex[:6]}'


class TaskContext:
    def __init__(self, knowledge, *, wait=0):
        """`wait` is how long, in seconds, an operation outlasts another one holding the
        knowledge lock. A person's request is refused at once and can be repeated; the run
        worker's memory has nobody to repeat it, so the worker waits instead."""
        self.knowledge = knowledge
        self.sessions = knowledge.sessions
        self.wait = wait

    def _knowledge(self, operation, *arguments, **options):
        deadline = time.monotonic() + self.wait
        while True:
            try:
                return getattr(self.knowledge, operation)(*arguments, **options)
            except KnowledgeBusy:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(.2)

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
        query = _text(data.get('query'), 'context query', QUERY_BYTES)
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

    def _call(self, session, options, workspace, action, **fields):
        result = self._knowledge('run', session['project_id'], {'bank': options['bank'], 'action': action, **fields},
                                 _root=workspace)
        if not result.get('ok') or not isinstance(result.get('result'), dict):
            raise SessionError(result.get('error') or 'The Brain runtime did not complete this operation.')
        return result['result']

    @staticmethod
    def _validate_capsule(capsule, task):
        if (not isinstance(capsule, dict) or capsule.get('task_id') != task.get('external_id')
                or capsule.get('task_uuid') != task.get('id')
                or type(capsule.get('task_revision')) is not int
                or capsule['task_revision'] != task.get('revision')
                or not isinstance(capsule.get('working'), dict)
                or capsule['working'].get('task_id') != task.get('external_id')):
            raise SessionError('Retrieved context does not match the linked task and revision. Prepare it again.')
        for layer, limit in (('procedural', 2), ('semantic', 3), ('episodic', 1)):
            values = capsule.get(layer)
            if not isinstance(values, list) or len(values) > limit or any(not isinstance(item, dict) for item in values):
                raise SessionError('The runtime returned an invalid task capsule.')
        if capsule.get('warnings'):
            raise SessionError('Context refresh reported warnings. Resolve the runtime warnings before preparing this session.')
        try:
            encoded = json.dumps(capsule, ensure_ascii=False, separators=(',', ':'), allow_nan=False)
            encoded.encode('utf-8')
        except (ValueError, RecursionError) as error:
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
    def _source_hashes(root, capsule, home=None):
        manifest = capsule.get('manifest')
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

    def _retrieve(self, session, options, workspace, task, info, ephemeral):
        # Every retrieval here is one a person or the session asked for. Bypass only
        # the repeat-skip heuristic; native privacy, lifecycle and freshness filters stay.
        # Unattended memory retrieves through `refresh`, which distills the whole
        # message rather than its first words and leaves out the instruction files
        # the provider loads by itself.
        automatic = not options.get('review')
        response = self._knowledge('run', session['project_id'], {
            'action': 'refresh' if automatic else 'retrieve', 'bank': options['bank'],
            'task_id': options['task_id'], 'query': options['query'],
        }, _root=workspace, _ephemeral=ephemeral, _gate='off',
            _host=session.get('provider') if automatic else None)
        if not response.get('ok'):
            raise SessionError(response.get('error') or 'Task context could not be retrieved.')
        capsule = response.get('result')
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
        hashes = self._source_hashes(layout['folder'] / info['root'], capsule, home)
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
        capsule, hashes, text = self._retrieve(session, options, workspace, task, info, not options.get('review'))
        return {**options, **kept, 'context_id': str(uuid.uuid4()), 'capsule': capsule, 'capsule_text': text,
                'task': task, 'source_hashes': hashes, 'approved': False}

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

    def save_memory(self, session, data, *, automatic=False, known=()):
        """Record a memory draft through the runtime's own commands.

        The task's progress and next steps are updated first; each kept learning
        becomes a verified finding or decision, resolved or accepted with its
        consequence as the content promotion carries; then automatic promotion
        runs once, under the runtime's own rules. Each command is atomic, the
        chain is not: a failure reports what was already saved.

        `automatic` is the unattended save at the end of a run: the agent's draft
        as written, with any learning whose sources are not in the workspace, or
        that this session already saved (`known`), left out rather than failing
        the rest. Its learnings are written as observed and raised to verified
        with a reason saying the agent attested them and no person reviewed them,
        so the record's own ledger tells the two apart. Promotion still follows
        the project's `automatic_promotion` setting.
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
        try:
            if draft['progress'] or draft['next_steps']:
                fields = {'record_id': task['id'], 'revision': task['revision'], 'reason': reason}
                if draft['progress']:
                    fields['progress'] = draft['progress']
                if draft['next_steps']:
                    # The draft is the current plan, so it replaces the list rather
                    # than appending to steps the run may have finished.
                    fields.update(next_steps=draft['next_steps'], replace_next_steps=True)
                updated = self._call(session, options, workspace, 'brain-update', **fields)
                saved['task'] = {'id': updated.get('id'), 'revision': updated.get('revision')}
            for learning in draft['learnings']:
                created = self._call(session, options, workspace, 'brain-create',
                                     record_type=learning['type'],
                                     external_id=memory_draft.external_id(options['task_id'], learning['type']),
                                     title=learning['title'], goal=learning['consequence'],
                                     sources=learning['sources'], authority='observed' if automatic else 'verified')
                closed = self._call(session, options, workspace, 'brain-update', record_id=created['id'],
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
                                            _root=workspace)
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

        `since` is when the run started: a turn report written after it means a
        Stop hook did fire and ran the checkpoint, and a second one would count
        the turn twice. Returns None then.
        """
        _, options, workspace, info = self._bound(session)
        if since is not None:
            layout = self.knowledge.layout(session['project_id'], workspace)
            report = layout['folder'] / info['root'] / 'memory-bank' / 'local' / 'last-turn-report.json'
            try:
                if report.stat().st_mtime >= since:
                    return None
            except OSError:
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
