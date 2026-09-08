"""Bind a browser session to native Brain state and an approved context snapshot."""
from __future__ import annotations

import json
from pathlib import Path
import re
import uuid

from .knowledge import _path, _text
from .sessions import SessionError, read_context

UUID4 = re.compile(r'[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}')
TASK_ID = re.compile(r'[A-Za-z0-9][A-Za-z0-9._/-]{0,199}')
FIELDS = ('bank', 'task_id', 'query', 'create', 'goal', 'record_id')
SCOPED_ACTIONS = {'brain-update', 'rebind', 'complete', 'brain-create',
                  'promote-propose', 'promote-review', 'promote-apply'}


class TaskContext:
    def __init__(self, knowledge):
        self.knowledge = knowledge
        self.sessions = knowledge.sessions

    @staticmethod
    def validate_options(data):
        if not isinstance(data, dict) or set(data) - set(FIELDS):
            raise SessionError('Invalid task context options.')
        bank = _path(data.get('bank'))
        if bank != 'memory-bank' and not bank.endswith('/memory-bank'):
            raise SessionError('Select a project Memory Bank.')
        task_id = _text(data.get('task_id'), 'task ID', 200).strip()
        if not TASK_ID.fullmatch(task_id) or '..' in task_id.split('/'):
            raise SessionError('Enter a valid Brain task ID.')
        query = _text(data.get('query'), 'context query', 4000)
        create = data.get('create', False)
        if type(create) is not bool:
            raise SessionError('The create-task option must be a boolean.')
        result = {'bank': bank, 'task_id': task_id, 'query': query, 'create': create}
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
        options = self.validate_options({key: brain[key] for key in FIELDS if key in brain})
        workspace = self.sessions._workspace(session)
        info = self.knowledge.info(session['project_id'], options['bank'], _root=workspace)
        if not info['runtime_available'] or info['mode'] != 'governed':
            raise SessionError('The session workspace requires an installed governed Brain runtime.')
        return brain, options, workspace, info

    def _call(self, session, options, workspace, action, **fields):
        result = self.knowledge.run(session['project_id'], {'bank': options['bank'], 'action': action, **fields},
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
        if len(encoded) > 8000:
            raise SessionError('The task capsule exceeds the native 8000-character limit.')

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
    def _source_hashes(root, capsule):
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
                    visible.add(_path(item['path']))
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
        # The user explicitly requests a preview. Bypass only the repeat-skip
        # heuristic; native privacy, lifecycle and source-freshness filters stay.
        response = self.knowledge.run(session['project_id'], {
            'action': 'retrieve', 'bank': options['bank'],
            'task_id': options['task_id'], 'query': options['query'],
        }, _root=workspace, _ephemeral=ephemeral, _gate='off')
        if not response.get('ok'):
            raise SessionError(response.get('error') or 'Task context could not be retrieved.')
        capsule = response.get('result')
        self._validate_capsule(capsule, task)
        hashes = self._source_hashes(Path(workspace) / info['root'], capsule)
        return capsule, hashes

    def prepare(self, session):
        _, options, workspace, info = self._bound(session)
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
        pending = {**options, 'approved': False, 'context_id': None, 'capsule': None}
        if hasattr(self.sessions, '_save_brain'):
            with self.sessions.lock:
                # Cancellation can queue a new query while native start is
                # finishing. Preserve that request, but retain the task UUID
                # so its retry cannot create a second task.
                current = self.sessions.get(session['id'])['brain']
                pending['query'] = current['query']
                self.sessions._save_brain(session['id'], pending)
        task = self.knowledge.inspect(session['project_id'], options['bank'], _root=workspace,
                                      task_id=record_id, task_only=True)['task']
        capsule, hashes = self._retrieve(session, options, workspace, task, info, False)
        return {**options, 'context_id': str(uuid.uuid4()), 'capsule': capsule,
                'task': task, 'source_hashes': hashes, 'approved': False}

    def ensure_fresh(self, session):
        brain, options, workspace, info = self._bound(session)
        saved_task, saved_capsule = brain.get('task'), brain.get('capsule')
        if (not isinstance(saved_task, dict) or not isinstance(saved_capsule, dict)
                or not isinstance(brain.get('context_id'), str) or not UUID4.fullmatch(brain['context_id'])):
            raise SessionError('Prepare the linked task context before running this session.')
        task = self.knowledge.inspect(session['project_id'], options['bank'], _root=workspace,
                                      task_id=options.get('record_id') or options['task_id'], task_only=True)['task']
        if (task.get('id') != saved_task.get('id') or task.get('revision') != saved_task.get('revision')
                or task.get('external_id') != options['task_id'] or task.get('status') in ('completed', 'cancelled')):
            raise SessionError('The linked task changed. Prepare and approve its context again.')
        capsule, hashes = self._retrieve(session, options, workspace, task, info, True)
        if hashes != brain.get('source_hashes') or self._material(capsule) != self._material(saved_capsule):
            raise SessionError('Task context sources changed. Prepare and approve the updated context before running.')
        return True

    def info(self, session):
        brain, options, workspace, _ = self._bound(session)
        result = self.knowledge.inspect(session['project_id'], options['bank'], _root=workspace,
                                       task_id=options.get('record_id') or options['task_id'])
        return {**result, 'context_id': brain.get('context_id'), 'approved': brain.get('approved') is True}

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
        response = self.knowledge.run(session['project_id'], {'bank': options['bank'], **fields}, _root=workspace)
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
