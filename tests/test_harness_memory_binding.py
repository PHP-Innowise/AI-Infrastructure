"""Fresh Cedar request binding controls; installed stdlib helpers, no model calls."""
from __future__ import annotations

import copy
import importlib
import json
import os
from pathlib import Path
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'harness/src'))
from harness import cost_observation, memory_attempts, memory_draft, sessions
from harness.knowledge import KnowledgeManager, KnowledgeRefusal, OLDER_RUNTIME, STALE_RECORD
from harness.task_context import TaskContext
from tests.test_harness_knowledge import install_knowledge_fixture

SCRIPTS = Path(__file__).resolve().parents[1] / 'PHP Core/memory-bank/scripts'
SOURCE = 'specs/cedar.md'
DRAFT = {'progress': 'The cedar allocation was checked.', 'next_steps': [], 'learnings': [
    {'type': 'finding', 'title': 'Cedar allocation has one owner',
     'consequence': 'Each cedar allocation records one owner.', 'sources': [SOURCE + '#owner']}],
    'verified': True}
REASON = memory_draft.AUTOMATIC_REASON


class NativePreparedTests(unittest.TestCase):
    def setUp(self):
        sys.path.insert(0, str(SCRIPTS))
        try:
            self.brain = importlib.import_module('brain_runtime')
            self.results = importlib.import_module('memory_results')
        finally:
            sys.path.pop(0)
        temp = tempfile.TemporaryDirectory(prefix='cedar-native-')
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        (self.root / 'specs').mkdir()
        (self.root / SOURCE).write_text('# Cedar allocation\n\nEach cedar allocation records one owner.\n')
        self.config = self.root / 'project-brain/config/runtime.json'
        self.config.parent.mkdir(parents=True)
        self.config.write_text(json.dumps({'mode': 'governed', 'automatic_promotion': False}))
        self.task = self.brain.create_task(self.root, 'CEDAR-PREPARE', 'Cedar allocation', [], [], owner='local')
        self.value = self.prepare()

    def prepare(self, data=None, reason=REASON):
        return self.results.prepare_result(self.root, self.task['id'], 'cedar-prepared', 1,
            DRAFT if data is None else data, owner='local', reason=reason)

    def record(self, *, replay=False, revision=1, task_id=None, scope=None):
        return self.results.record_result(self.root, task_id or self.task['id'], 'cedar-prepared', revision,
            self.value['request'], owner='local', reason=REASON,
            prepared=scope or self.value['scope'], replay_only=replay)

    def images(self):
        return {str(p.relative_to(self.root)): p.read_bytes() for p in self.root.rglob('*') if p.is_file()}

    def test_preparation_writes_no_result_and_preserves_empty_vs_absent(self):
        before = self.images()
        self.assertIn('next_steps', self.value['request'])
        omitted = self.prepare({'progress': 'Cedar checked.'})
        self.assertNotIn('next_steps', omitted['request'])
        self.assertEqual([], self.value['request']['next_steps'])
        self.assertEqual(before, self.images())

    def test_transcript_body_and_reason_fail_before_retention(self):
        before = self.images()
        for data, reason in [({'progress': 'user: pasted transcript'}, REASON), (DRAFT, 'stderr: pasted log')]:
            with self.subTest(reason=reason), self.assertRaises(self.brain.BrainError):
                self.prepare(data, reason)
        self.assertEqual(before, self.images())

    def test_missing_verification_is_refused(self):
        draft = {k: v for k, v in DRAFT.items() if k != 'verified'}
        with self.assertRaises(self.brain.BrainError):
            self.prepare(draft)

    def test_missing_receipt_never_creates_records_even_when_task_holds_progress(self):
        before = self.images()
        with self.assertRaisesRegex(self.brain.BrainError, 'receipt_unavailable'):
            self.record(replay=True)
        self.assertEqual(before, self.images())

    def test_receipt_survives_intervening_alias_replay_and_missing_citation(self):
        self.record()
        self.record(task_id=self.task['external_id'])
        (self.root / SOURCE).unlink()
        before = self.images()
        result = self.record(replay=True)
        self.assertTrue(result['replayed'])
        self.assertEqual(before, self.images())

    def test_archived_task_is_available_only_for_exact_replay(self):
        self.record()
        task = self.brain.find_record(self.root, self.task['id'])[1]
        self.brain.update_record(self.root, self.task['id'], expected_revision=task['revision'],
            progress=None, next_steps=[], files=[], sources=[], actor='local', reason='Cedar task finished',
            transition_to='completed')
        self.brain.compact(self.root)
        (self.root / SOURCE).unlink()
        self.assertTrue(self.record(replay=True)['replayed'])

    def test_corrupt_local_receipt_without_shared_proof_is_incomplete_without_mutation(self):
        self.record()
        self.results._attempt(self.root, self.task['id'], 'cedar-prepared', self.value['digest']).unlink()
        receipt = self.results._receipt(self.root, self.task['id'], 'cedar-prepared')
        for value in ['not json', json.dumps({'digest': self.value['digest']}),
                      json.dumps({'result_id': 'other', 'task_id': self.task['id'],
                                  'digest': self.value['digest'], 'task_revision': 2})]:
            with self.subTest(value=value):
                receipt.write_text(value)
                before = self.images()
                with self.assertRaisesRegex(self.brain.BrainError, 'receipt_unavailable'):
                    self.record(replay=True)
                self.assertEqual(before, self.images())

    def test_unique_stale_refusal_is_before_writes(self):
        self.brain.update_record(self.root, self.task['id'], expected_revision=1, progress='Independent update',
            next_steps=[], files=[], sources=[], actor='local', reason='Cedar revision changed')
        before = self.images()
        with self.assertRaisesRegex(self.brain.BrainError, '^memory-result-v1:prepared_stale_no_write$'):
            self.record()
        self.assertEqual(before, self.images())

    def test_policy_and_actor_changes_refuse_frozen_scope(self):
        self.config.write_text(json.dumps({'mode': 'governed', 'automatic_promotion': True}))
        before = self.images()
        with self.assertRaisesRegex(self.brain.BrainError, 'scope_changed'):
            self.record()
        self.assertEqual(before, self.images())


class HarnessBindingTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix='cedar-harness-')
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.project = self.root / 'project'
        self.project.mkdir()
        install_knowledge_fixture(self.project)
        (self.project / SOURCE).write_text('# Cedar allocation\n\nEach cedar allocation records one owner.\n')
        (self.project / 'project-brain/config/runtime.json').write_text(json.dumps(
            {'mode': 'governed', 'automatic_promotion': False}))
        for target, values in [(sessions.providers, {'discover_providers': [{'id': 'codex', 'available': True,
                'name': 'No models', 'executable': 'unused'}]}),
                (sessions.Sessions, {'_worker': None, '_keep_accelerators_current': None})]:
            for name, result in values.items():
                mocked = patch.object(target, name, return_value=result)
                mocked.start(); self.addCleanup(mocked.stop)
        env = patch.dict(os.environ, {'PROJECT_BRAIN_OWNER': 'local'})
        env.start(); self.addCleanup(env.stop)
        self.store = sessions.Sessions(self.root / 'state', [self.project])
        self.addCleanup(self.store.close)
        self.manager = KnowledgeManager(self.store)
        self.addCleanup(self.manager.close)
        self.store.knowledge = self.manager
        self.project_id = next(iter(self.store.projects))
        self.session = self.store.create({'project_id': self.project_id, 'provider': 'codex', 'mode': 'plan',
            'prompt': 'Check cedar allocation', 'brain': {'bank': 'memory-bank', 'task_id': 'CEDAR-HARNESS',
                'query': 'cedar allocation', 'goal': 'Check cedar allocation', 'create': True, 'review': True}})
        self.sid = self.session['id']
        context = TaskContext(self.manager)
        value = context.prepare(self.session)
        self.store._save_brain(self.sid, value)
        self.session = self.store.get(self.sid)
        self.generation = self.store.generations[self.sid]
        self.store.results.start_launch(self.sid, self.generation)
        self.store.launch_ids[self.sid] = self.generation
        self.store._event(self.sid, {'kind': 'result', 'text': '```memory-draft\n' + json.dumps(DRAFT) + '\n```'})
        self.event_id = self.store.events(self.sid)[-1]['id']
        self.data = memory_draft.latest(self.store, self.sid)['draft']
        self.observation = cost_observation.LaunchObservation('codex')
        self.observation.memory_owner = (self.sid, self.generation)
        self.context = TaskContext(self.manager, observation=self.observation)

    def save(self, **changes):
        fields = {'automatic': True, 'generation': self.generation, 'origin_event_id': self.event_id}
        fields.update(changes)
        return self.context.save_memory(self.store.get(self.sid), self.data, **fields)

    def binding(self):
        return memory_attempts.lookup(self.store, self.sid, self.generation)

    def test_first_save_freezes_before_mutation_then_replays_without_source_filter(self):
        first = self.save()
        self.assertTrue(first['ok'], first)
        binding = self.binding()
        self.assertEqual(self.session['brain']['record_id'], binding['prepared']['task_id'])
        (self.project / SOURCE).unlink()
        with patch.object(memory_draft, 'usable', side_effect=AssertionError('Replay refiltered')):
            second = self.save()
        self.assertTrue(second['ok'], second)
        self.assertEqual(binding, self.binding())
        self.assertEqual(['prepare-result', 'record-result', 'replay-result'],
            [x['operation'] for x in self.observation.operations if x['operation'] != 'inspect'])

    def test_source_absent_original_omission_stays_absent_after_source_arrival(self):
        (self.project / SOURCE).unlink()
        self.assertTrue(self.save()['ok'])
        self.assertEqual([], self.binding()['prepared']['request']['learnings'])
        (self.project / SOURCE).write_text('# Cedar newly available\n')
        reply = self.save()
        self.assertTrue(reply['ok'], reply)
        self.assertEqual([], reply['saved']['records'])
        self.assertEqual(1, len(reply['skipped']))

    def test_changed_original_draft_conflicts_even_if_live_filter_would_omit_it(self):
        self.save()
        self.data['learnings'][0]['consequence'] = 'Changed cedar claim'
        before = len(self.observation.operations)
        with self.assertRaises(sessions.SessionError):
            self.save()
        self.assertEqual(before, len(self.observation.operations))

    def test_missing_receipt_never_falls_back_to_normal_or_legacy_write(self):
        self.save()
        for folder in ('project-brain/local/results', 'project-brain/control/result-attempts'):
            for path in (self.project / folder).rglob('*.json'):
                path.unlink()
        with patch.object(self.context, '_save_memory_chain', side_effect=AssertionError('Legacy fallback')):
            reply = self.save()
        self.assertFalse(reply['ok'])
        self.assertEqual('replay-result', self.observation.operations[-1]['operation'])

    def test_existing_binding_refuses_absent_or_finalized_observer(self):
        self.save()
        for observer in [None, self.observation]:
            if observer is not None: observer.finish('completed')
            self.context.observation = observer
            with self.assertRaises(sessions.SessionError):
                self.save()

    def test_wrong_observer_owner_or_origin_never_binds(self):
        self.observation.memory_owner = ('other', self.generation)
        # Missing observer ownership is a compatibility first call, but an exact
        # claimed origin with an owned observer must match the event's launch.
        self.observation.memory_owner = (self.sid, self.generation)
        self.store.db.execute('UPDATE events SET data=? WHERE id=?',
            (json.dumps({'kind': 'result', 'launch_id': 'other', 'text': '```memory-draft\n'+json.dumps(DRAFT)+'\n```'}), self.event_id))
        self.store.db.commit()
        with self.assertRaises(sessions.SessionError):
            self.save()
        self.assertIsNone(self.binding())

    def test_cancel_after_prepare_prevents_binding_and_first_write(self):
        original = self.context._call
        def call(*args, **kwargs):
            value = original(*args, **kwargs)
            if args[3] == 'prepare-result': self.store.cancelled.add(self.sid)
            return value
        with patch.object(self.context, '_call', side_effect=call), self.assertRaises(sessions.SessionError):
            self.save()
        self.assertIsNone(self.binding())

    def test_cancel_after_binding_keeps_incomplete_intent_without_launching_write(self):
        original = memory_attempts.insert
        def insert(*args):
            result = original(*args)
            self.store.cancelled.add(self.sid)
            return result
        with patch.object(memory_attempts, 'insert', side_effect=insert):
            result = self.save()
        self.assertFalse(result['ok'])
        self.assertIsNotNone(self.binding())
        record = self.observation.operations[-1]
        self.assertEqual('record-result', record['operation'])
        self.assertFalse(record['runtime_io'][0]['started'])
        self.assertEqual([], list((self.project / 'project-brain/local/results').glob('*.json')))

    def test_context_dispatcher_change_refuses_frozen_scope(self):
        self.save()
        with (self.project / 'memory-bank/scripts/context.py').open('a') as stream:
            stream.write('\n# Changed Cedar CLI dispatch\n')
        self.assertFalse(self.save()['ok'])

    def test_requeue_restart_actor_root_and_runtime_mismatch_do_not_rewrite(self):
        self.save()
        old = self.binding()
        with patch.dict(os.environ, {'PROJECT_BRAIN_OWNER': 'other'}), self.assertRaises(sessions.SessionError):
            self.save()
        self.store.generations.clear()
        with self.assertRaises(sessions.SessionError): self.save()
        self.store.generations[self.sid] = self.generation
        with (self.project / 'memory-bank/scripts/memory_results.py').open('a') as stream:
            stream.write('\n# Changed installed implementation\n')
        self.assertFalse(self.save()['ok'])
        self.assertEqual(old, self.binding())

    def test_private_request_never_enters_history_session_or_cost_snapshot(self):
        self.save()
        rendered = json.dumps({'session': self.store.get(self.sid),
            'history': self.store.results.history(self.sid), 'events': self.store.events(self.sid),
            'cost': self.observation.finish('completed')})
        # Original provider draft remains an event, as before; immutable body is
        # confined to the private table and never expands a public row.
        self.assertNotIn('prepared_scope', rendered)
        self.assertNotIn('input_digest', rendered)
        self.assertNotIn('memory_result_attempts', rendered)
        self.assertNotIn(self.binding()['prepared']['scope'], rendered)

    def no_retry(self, message):
        original = self.context._call
        calls = []
        def call(*args, **kwargs):
            calls.append(args[3])
            if args[3] == 'record-result': raise KnowledgeRefusal(message)
            return original(*args, **kwargs)
        with patch.object(self.context, '_call', side_effect=call), \
                patch.object(self.context, '_save_memory_chain', side_effect=AssertionError('Legacy fallback')):
            reply = self.save()
        self.assertFalse(reply['ok'])
        self.assertEqual(1, calls.count('record-result'))

    def test_unknown_outcome_never_retries_first_write(self):
        self.no_retry('Unknown write outcome')

    def test_generic_stale_never_retries_first_write(self):
        self.no_retry(STALE_RECORD)

    def test_frozen_original_observer_without_binding_cannot_downgrade(self):
        self.observation.finish('completed')
        with patch.object(self.context, '_save_memory_unbound', side_effect=AssertionError('Downgrade')), \
                self.assertRaises(sessions.SessionError):
            self.save()
        self.assertIsNone(self.binding())

    def test_alias_only_fresh_link_freezes_original_canonical_binding(self):
        value = copy.deepcopy(self.session['brain'])
        value.pop('record_id')
        self.store._save_brain(self.sid, value)
        self.assertTrue(self.save()['ok'])
        binding = self.binding()
        self.assertIsNotNone(binding)
        self.assertEqual(self.session['brain']['record_id'], binding['identity']['scope']['task'])
        self.assertEqual(value['task_id'], binding['identity']['scope']['task_alias'])

    def test_existing_binding_cannot_downgrade_by_removing_linked_uuid(self):
        self.save()
        value = copy.deepcopy(self.session['brain']); value.pop('record_id')
        self.store._save_brain(self.sid, value)
        with self.assertRaises(sessions.SessionError): self.save()

    def test_corrupt_retained_payload_never_launches(self):
        self.save()
        old = self.binding()
        for value in [None, [], {'progress': '\ud800'}]:
            corrupt = {**old, 'prepared': value}
            self.store.db.execute('UPDATE memory_result_attempts SET binding=? WHERE session_id=? AND generation=?',
                (json.dumps(corrupt), self.sid, self.generation)); self.store.db.commit()
            before = len(self.observation.operations)
            with self.assertRaises(sessions.SessionError): self.save()
            self.assertEqual(before, len(self.observation.operations))

    def test_unique_native_stale_retry_keeps_original_body_and_initial_revision(self):
        original = self.context._call
        calls = []
        def call(*args, **kwargs):
            calls.append(args[3])
            if args[3] == 'record-result' and calls.count('record-result') == 1:
                response = self.manager.run(self.project_id, {'action': 'brain-update', 'bank': 'memory-bank',
                    'record_id': self.session['brain']['record_id'], 'revision': kwargs['revision'],
                    'progress': 'Independent cedar checkpoint', 'reason': 'Cedar checkpoint'})
                self.assertTrue(response['ok'], response)
            return original(*args, **kwargs)
        with patch.object(self.context, '_call', side_effect=call): reply = self.save()
        self.assertTrue(reply['ok'], reply)
        self.assertEqual(2, calls.count('record-result'))
        self.assertEqual(1, self.binding()['prepared']['revision'])

    def test_unsupported_fresh_prepare_keeps_old_path_but_frozen_replay_never_falls_back(self):
        original = self.context._call
        def old(*args, **kwargs):
            if args[3] == 'prepare-result': raise KnowledgeRefusal(OLDER_RUNTIME)
            return original(*args, **kwargs)
        with patch.object(self.context, '_call', side_effect=old): self.assertTrue(self.save()['ok'])
        self.assertIsNone(self.binding())
        self.assertTrue(self.save()['ok'])
        with patch.object(self.context, '_call', side_effect=KnowledgeRefusal(OLDER_RUNTIME)), \
                patch.object(self.context, '_save_memory_chain', side_effect=AssertionError('Legacy replay')):
            self.assertFalse(self.save()['ok'])

    def test_durable_history_does_not_exhaust_new_admission_or_evict_rows(self):
        public = json.loads((Path(__file__).resolve().parents[1] /
            'tests/fixtures/memory_regression_remediation_public.json').read_text())
        count = public['history_rows']
        for i in range(count):
            self.store.db.execute('INSERT INTO memory_result_attempts VALUES(?,?,?)', ('other', str(i), '{}'))
        self.store.db.commit()
        before = [tuple(row) for row in self.store.db.execute(
            'SELECT generation,binding FROM memory_result_attempts WHERE session_id=? ORDER BY generation', ('other',))]
        result = self.save()
        self.assertTrue(result['ok'], result)
        binding = self.binding()
        self.assertIsNotNone(binding)
        self.assertTrue(self.save()['ok'])
        self.assertEqual(binding, self.binding())
        self.assertEqual(count + 1, self.store.db.execute('SELECT COUNT(*) FROM memory_result_attempts').fetchone()[0])
        self.assertEqual(before, [tuple(row) for row in self.store.db.execute(
            'SELECT generation,binding FROM memory_result_attempts WHERE session_id=? ORDER BY generation', ('other',))])
        # Reopen the persistent database, not a recreated empty in-memory store.
        import sqlite3
        from contextlib import closing
        with closing(sqlite3.connect(self.store.db.execute('PRAGMA database_list').fetchone()[2])) as reopened:
            self.assertEqual(count + 1, reopened.execute('SELECT COUNT(*) FROM memory_result_attempts').fetchone()[0])
            self.assertEqual(before, list(reopened.execute(
                'SELECT generation,binding FROM memory_result_attempts WHERE session_id=? ORDER BY generation', ('other',))))

    def test_same_binding_insert_returns_only_one_first_writer(self):
        self.save()
        old = self.binding()
        newer = {**old['prepared'], 'revision': old['prepared']['revision'] + 1}
        with self.store.lock:
            value, first = memory_attempts.insert(self.store, old['identity'], newer, old['skipped_count'])
        self.assertFalse(first)
        self.assertEqual(old, value)

    def test_contender_keeps_first_selection_when_source_availability_changes(self):
        self.save()
        old = self.binding()
        contender = {**old['prepared'], 'request': {'progress': 'The cedar allocation was checked.',
                                                    'next_steps': [], 'learnings': [], 'verified': True}}
        with self.store.lock:
            value, first = memory_attempts.insert(self.store, old['identity'], contender, 1)
        self.assertFalse(first)
        self.assertEqual(old, value)

    def test_two_save_callers_one_mutator_loser_receipt_only(self):
        barrier = threading.Barrier(2)
        contender_done = threading.Event()
        original = self.context._call
        calls, replies, errors = [], [], []
        def call(*args, **kwargs):
            action = args[3]
            calls.append(action)
            if action == 'prepare-result':
                value = original(*args, **kwargs)
                barrier.wait(timeout=10)
                return value
            if action == 'record-result':
                if not contender_done.wait(timeout=10): raise AssertionError('Contender never replayed')
            try:
                return original(*args, **kwargs)
            finally:
                if action == 'replay-result': contender_done.set()
        def worker():
            try: replies.append(self.save())
            except Exception as error: errors.append(error)
        self.context.wait = 5
        with patch.object(self.context, '_call', side_effect=call):
            threads = [threading.Thread(target=worker) for _ in range(2)]
            for thread in threads: thread.start()
            for thread in threads: thread.join(timeout=15)
        self.assertTrue(all(not t.is_alive() for t in threads))
        self.assertEqual([], errors)
        self.assertEqual(1, calls.count('record-result'))
        self.assertEqual(1, calls.count('replay-result'))
        self.assertEqual([False, True], sorted(x['ok'] for x in replies))

    def test_unsupported_prepare_after_concurrent_bind_cannot_fallback(self):
        original = self.context._call
        intercepted = False
        def call(*args, **kwargs):
            nonlocal intercepted
            if args[3] == 'prepare-result' and not intercepted:
                intercepted = True
                # A second caller binds/writes after the outer initial lookup.
                self.assertTrue(self.save()['ok'])
                raise KnowledgeRefusal(OLDER_RUNTIME)
            return original(*args, **kwargs)
        with patch.object(self.context, '_call', side_effect=call), \
                patch.object(self.context, '_save_memory_chain', side_effect=AssertionError('Legacy fallback')), \
                self.assertRaises(sessions.SessionError):
            self.save()
        self.assertIsNotNone(self.binding())

    def test_tagged_wrong_owner_or_missing_origin_never_downgrades(self):
        self.observation.memory_owner = ('other', self.generation)
        with self.assertRaises(sessions.SessionError): self.save()
        self.observation.memory_owner = (self.sid, self.generation)
        with self.assertRaises(sessions.SessionError): self.save(origin_event_id=None)
        self.assertEqual([], self.observation.operations)

    def test_tagged_observer_missing_generation_never_downgrades(self):
        for frozen in [False, True]:
            if frozen: self.observation.finish('completed')
            with self.subTest(frozen=frozen), self.assertRaises(sessions.SessionError):
                self.save(generation=None)
        self.assertEqual([], self.observation.operations)
        self.assertIsNone(self.binding())

    def test_oversized_stored_binding_is_refused_before_json_decode(self):
        self.store.db.execute('INSERT INTO memory_result_attempts VALUES(?,?,?)',
            (self.sid, self.generation, ' ' * (memory_attempts.BYTE_LIMIT + 1)))
        self.store.db.commit()
        with patch.object(memory_attempts.json, 'loads', side_effect=AssertionError('Decoded oversized row')), \
                self.assertRaises(sessions.SessionError):
            memory_attempts.lookup(self.store, self.sid, self.generation)

    def test_rejected_transcript_has_no_immutable_body(self):
        self.data['progress'] = 'user: pasted transcript'
        text = '```memory-draft\n' + json.dumps(self.data) + '\n```'
        self.store.db.execute('UPDATE events SET data=? WHERE id=?', (json.dumps(
            {'kind': 'result', 'launch_id': self.generation, 'text': text}), self.event_id))
        self.store.db.commit()
        with self.assertRaises(sessions.SessionError): self.save()
        self.assertIsNone(self.binding())
        self.assertEqual([], list(Path(self.manager.temporary.name).glob('result-*.json')))

    def test_manual_saves_do_not_adopt_automatic_generation(self):
        self.assertTrue(self.save()['ok'])
        before = self.binding()
        data = {**self.data, 'verified': True, 'progress': 'A person checked the cedar release path.'}
        ids = []
        original = self.context._call
        def call(*args, **kwargs):
            if args[3] == 'record-result': ids.append(kwargs['result_id'])
            return original(*args, **kwargs)
        with patch.object(self.context, '_call', side_effect=call):
            for _ in range(2):
                self.assertTrue(self.context.save_memory(self.store.get(self.sid), data)['ok'])
        self.assertEqual(2, len(set(ids)))
        self.assertEqual(before, self.binding())

    def test_partial_candidate_staging_write_is_cleaned(self):
        original = Path.write_text
        def write(path, data, *args, **kwargs):
            if path.parent == Path(self.manager.temporary.name) and path.name.startswith('result-'):
                original(path, data[:5], *args, **kwargs)
                raise OSError('Cedar staging failure')
            return original(path, data, *args, **kwargs)
        with patch.object(Path, 'write_text', new=write):
            result = self.manager.run(self.project_id, {'action': 'prepare-result', 'task_id': self.session['brain']['record_id'],
                'result_id': 'cedar-staging', 'revision': 1, 'request': DRAFT, 'attestation': 'agent', 'reason': REASON})
        self.assertFalse(result['ok'])
        self.assertEqual([], list(Path(self.manager.temporary.name).glob('result-*.json')))


if __name__ == '__main__':
    unittest.main()
