"""Fresh Kite controls for automatic save regressions; no model calls."""
from __future__ import annotations
import copy
import importlib
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
PUBLIC = json.loads((ROOT / 'tests/fixtures/memory_review_regressions_public.json').read_text(encoding='utf-8'))


class MemoryReviewRegressionTests(unittest.TestCase):
    def setUp(self):
        clean = {k: v for k, v in os.environ.items() if not k.startswith(('GIT_', 'CONTEXT_', 'ACCELERATOR_'))}
        clean['GIT_OPTIONAL_LOCKS'] = '0'
        environment = mock.patch.dict(os.environ, clean, clear=True)
        environment.start(); self.addCleanup(environment.stop)
        sys.path.insert(0, str(ROOT / 'PHP Core/memory-bank/scripts'))
        try:
            self.result = importlib.import_module('memory_results')
            self.brain = importlib.import_module('brain_runtime')
        finally:
            sys.path.pop(0)
        folder = tempfile.TemporaryDirectory(prefix='kite-review-save-')
        self.addCleanup(folder.cleanup); self.root = Path(folder.name)
        source = self.root / PUBLIC['source_path']; source.parent.mkdir()
        source.write_text(PUBLIC['source'], encoding='utf-8')
        config = self.root / 'project-brain/config/runtime.json'; config.parent.mkdir(parents=True)
        config.write_text(json.dumps({'mode': 'governed', 'automatic_promotion': False}), encoding='utf-8')
        (self.root / 'memory-bank/chunks').mkdir(parents=True)
        self.task = self.brain.create_task(self.root, PUBLIC['task_alias'], PUBLIC['task_title'], [], [], owner='local')

    def data(self, *, followup=False):
        return {'progress': PUBLIC['followup_progress' if followup else 'first_progress'],
                'next_steps': PUBLIC['followup_steps'] if followup else [],
                'learnings': [copy.deepcopy(PUBLIC['learning'])], 'verified': True}

    def save(self, result_id, data, *, revision=None, attestation='agent'):
        revision = self.now()['revision'] if revision is None else revision
        return self.result.record_result(self.root, self.task['id'], result_id, revision, data,
                                        owner='local', reason='Checked public Kite policy', attestation=attestation)

    def now(self):
        return self.brain.find_record(self.root, self.task['id'], record_type='task')[1]

    def attempt(self, result_id, data=None):
        result = self.result.normalize(self.root, self.data() if data is None else data, check_files=False)
        return self.result._attempt(self.root, self.task['id'], result_id,
                                    self.result.request_digest(self.task['id'], result))

    def learning(self):
        identity = self.result.learning_id(self.task['id'], PUBLIC['learning'])
        return self.brain.find_record(self.root, identity, include_archive=True)

    def assert_followup(self, reply):
        self.assertIsNotNone(reply['task'])
        self.assertEqual(PUBLIC['followup_progress'], self.now()['progress'])
        self.assertEqual(PUBLIC['followup_steps'], self.now()['next_steps'])
        self.assertEqual('created', reply['records'][1]['state'])

    def followup(self):
        data = self.data(followup=True)
        data['learnings'][0]['consequence'] = PUBLIC['reworded_consequence']
        data['learnings'].append(copy.deepcopy(PUBLIC['new_learning']))
        return data

    def test_auto_revision_accepts_hook_updated_task(self):
        self.brain.update_record(self.root, self.task['id'], expected_revision=1,
            progress='Kite hook checkpoint', next_steps=[], files=[], sources=[], actor='local', reason='Kite hook')
        reply = self.save('kite-hook-auto', self.data(), revision='auto')
        self.assertEqual(PUBLIC['first_progress'], self.now()['progress'])
        self.assertIsNotNone(reply['task'])

    def test_native_agent_reword_updates_only_current_owned_image(self):
        self.save('kite-first', self.data())
        reply = self.save('kite-second', self.followup())
        self.assert_followup(reply)
        self.assertEqual('updated', reply['records'][0]['state'])
        self.assertEqual(PUBLIC['reworded_consequence'], self.learning()[1]['progress'])

    def test_foreign_record_kind_is_retained_without_losing_independent_progress(self):
        learning=PUBLIC['learning'];identity=self.result.learning_id(self.task['id'],learning)
        foreign=self.brain.create_record(self.root,'bug',identity,learning['title'],[],learning['sources'],
            owner='local',authority='observed',goal=learning['consequence'])
        for status in ('triaged','fixing','verifying','resolved'):
            foreign=self.brain.update_record(self.root,foreign['id'],expected_revision=foreign['revision'],
                progress=learning['consequence'],next_steps=[],files=[],sources=[],actor='local',
                reason='Kite kind collision public verification',transition_to=status,
                **({'authority':'verified','attestation':'agent'} if status=='resolved' else {}))
        leaf=self.brain.find_record(self.root,foreign['id'])[0];before=leaf.read_bytes()
        request=self.followup();request['learnings'][0]['expected_revision']=foreign['revision']
        reply=self.save('kite-kind-collision',request)
        self.assert_followup(reply);self.assertEqual('kind-conflict',reply['records'][0]['reason'])
        self.assertEqual(before,leaf.read_bytes())

    def test_person_claim_is_retained_without_losing_independent_parts(self):
        first = self.data(); first['learnings'][0]['consequence'] = PUBLIC['person_consequence']
        self.save('kite-person', first, attestation='person')
        path, _, _ = self.learning(); before = path.read_bytes()
        reply = self.save('kite-agent-followup', self.followup())
        self.assert_followup(reply)
        self.assertEqual('differs', reply['records'][0]['state'])
        self.assertEqual(before, path.read_bytes())
        self.assertIn('revision', reply['records'][0])

    def test_plain_manual_edit_cannot_be_overwritten_by_old_agent_attestation(self):
        self.save('kite-first', self.data()); path, record, _ = self.learning()
        self.brain.update_record(self.root, record['id'], expected_revision=record['revision'],
            progress=PUBLIC['person_consequence'], next_steps=[], files=[], sources=[], actor='local', reason='Kite manual correction')
        before = path.read_bytes()
        reply = self.save('kite-after-manual', self.followup())
        self.assert_followup(reply)
        self.assertEqual('differs', reply['records'][0]['state'])
        self.assertEqual(before, path.read_bytes())

    def assert_unproven_manual_edit_is_held(self):
        path, record, _ = self.learning()
        self.brain.update_record(self.root, record['id'], expected_revision=record['revision'],
            progress=PUBLIC['person_consequence'], next_steps=[], files=[], sources=[], actor='local',
            reason='Manual correction after unavailable learning proof')
        before = path.read_bytes()
        reply = self.save('unproven-followup', self.followup())
        self.assert_followup(reply)
        self.assertEqual('differs', reply['records'][0]['state'])
        self.assertEqual('original-image-unproven', reply['records'][0]['reason'])
        self.assertEqual(before, path.read_bytes())

    def test_unavailable_learning_image_preserves_later_manual_edit(self):
        identity = self.result.learning_id(self.task['id'], PUBLIC['learning'])
        image = self.result._learning_image_path(self.root, identity)
        image.parent.parent.mkdir(parents=True, exist_ok=True)
        image.parent.write_text('Unavailable proof directory\n', encoding='utf-8')
        first = self.save('unavailable-proof', self.data())
        self.assertEqual('created', first['records'][0]['state'])
        self.assertFalse(image.exists())
        self.assert_unproven_manual_edit_is_held()

    def test_removed_learning_image_preserves_later_manual_edit(self):
        self.save('removed-proof', self.data())
        identity = self.learning()[1]['external_id']
        self.result._learning_image_path(self.root, identity).unlink()
        self.assert_unproven_manual_edit_is_held()

    def test_corrupt_learning_image_preserves_later_manual_edit(self):
        self.save('corrupt-proof', self.data())
        identity = self.learning()[1]['external_id']
        self.result._learning_image_path(self.root, identity).write_text('{', encoding='utf-8')
        self.assert_unproven_manual_edit_is_held()

    def test_explicit_learning_revision_allows_correction_without_image(self):
        self.save('explicit-proof', self.data())
        record = self.learning()[1]
        self.result._learning_image_path(self.root, record['external_id']).unlink()
        self.brain.update_record(self.root, record['id'], expected_revision=record['revision'],
            progress=PUBLIC['person_consequence'], next_steps=[], files=[], sources=[], actor='local',
            reason='Manual correction before deliberate revision-checked update')
        data = self.followup()
        data['learnings'][0]['expected_revision'] = self.learning()[1]['revision']
        reply = self.save('explicit-correction', data)
        self.assert_followup(reply)
        self.assertEqual('updated', reply['records'][0]['state'])
        self.assertEqual(PUBLIC['reworded_consequence'], self.learning()[1]['progress'])

    def test_unrelated_cyrillic_findings_from_one_source_both_promote(self):
        config = self.root / 'project-brain/config/runtime.json'
        config.write_text(json.dumps({'mode': 'governed', 'automatic_promotion': True}), encoding='utf-8')
        source = self.root / PUBLIC['source_path']
        claims = (('Регистрация события', 'Система пишет аудит при изменении ролей.'),
                  ('Проверка срока', 'Сессия завершается после истечения срока.'))
        source.write_text('# Правила\n\n' + '\n\n'.join(rule for _, rule in claims), encoding='utf-8')
        for index, (title, rule) in enumerate(claims):
            reply = self.save('cyrillic-' + str(index), {'verified': True, 'learnings': [
                {'type': 'finding', 'title': title, 'consequence': rule, 'sources': [PUBLIC['source_path']]}]})
            self.assertEqual(1, len(reply['promotion']['promoted']), reply['promotion'])
            self.assertEqual([], reply['promotion']['blocked'])
        self.assertEqual(2, len(list((self.root / 'memory-bank/chunks').glob('MEM-*.md'))))

    def test_repeated_cyrillic_finding_is_still_deduplicated(self):
        config = self.root / 'project-brain/config/runtime.json'
        config.write_text(json.dumps({'mode': 'governed', 'automatic_promotion': True}), encoding='utf-8')
        consequence = 'Система пишет аудит при изменении ролей.'
        (self.root / PUBLIC['source_path']).write_text('# Правило\n\n' + consequence, encoding='utf-8')
        for index in range(2):
            task = self.brain.create_task(self.root, 'cyrillic-task-' + str(index),
                                          'Проверка аудита', [], [], owner='local')
            data = {'verified': True, 'learnings': [{'type': 'finding', 'title': 'Регистрация события',
                'consequence': consequence, 'sources': [PUBLIC['source_path']]}]}
            reply = self.result.record_result(self.root, task['id'], 'cyrillic-duplicate-' + str(index),
                task['revision'], data, owner='local', reason='Checked Cyrillic source', attestation='agent')
            self.assertEqual(1 if index == 0 else 0, len(reply['promotion']['promoted']))
            if index:
                self.assertEqual(1, len(reply['promotion']['blocked']))
                self.assertIn('near-duplicate', reply['promotion']['blocked'][0]['reason'])
        self.assertEqual(1, len(list((self.root / 'memory-bank/chunks').glob('MEM-*.md'))))

    def test_wrong_learning_revision_retains_only_that_learning(self):
        self.save('kite-first', self.data()); path, record, _ = self.learning(); before = path.read_bytes()
        data = self.followup(); data['learnings'][0]['expected_revision'] = record['revision'] + 7
        reply = self.save('kite-wrong-learning-cas', data)
        self.assert_followup(reply)
        self.assertEqual('differs', reply['records'][0]['state'])
        self.assertEqual('revision-conflict', reply['records'][0]['reason'])
        self.assertEqual(before, path.read_bytes())

    def test_archived_learning_does_not_block_new_task_progress(self):
        self.save('kite-first', self.data()); path, _, _ = self.learning(); before = path.read_bytes()
        destination = self.root / 'project-brain/archive/findings' / path.name
        destination.parent.mkdir(parents=True); path.rename(destination)
        reply = self.save('kite-archive-followup', self.followup())
        self.assert_followup(reply)
        self.assertEqual('archived-differs', reply['records'][0]['state'])
        self.assertEqual(before, destination.read_bytes())

    def test_stale_task_revision_and_changed_result_payload_remain_hard_refusals(self):
        self.save('kite-first', self.data()); before = self.now()
        with self.assertRaises(self.brain.BrainError):
            self.save('kite-stale-task', self.followup(), revision=1)
        with self.assertRaisesRegex(self.brain.BrainError, 'different content'):
            self.save('kite-first', self.followup())
        self.assertEqual(before, self.now())

    def test_failure_before_effect_retries_same_result_at_exact_frontier(self):
        with mock.patch.object(self.result, 'create_record', side_effect=OSError('Kite creation did not start')):
            with self.assertRaises(OSError):
                self.save('kite-no-effect', self.data())
        reply = self.save('kite-no-effect', self.data(), revision='auto')
        self.assertEqual('created', reply['records'][0]['state'])
        self.assertEqual(PUBLIC['first_progress'], self.now()['progress'])

    def test_observed_created_stage_resumes_without_duplicate_learning(self):
        with mock.patch.object(self.result, 'update_record', side_effect=OSError('Kite close not started')):
            with self.assertRaises(OSError):
                self.save('kite-created-frontier', self.data())
        identity = self.learning()[1]['id']
        reply = self.save('kite-created-frontier', self.data(), revision='auto')
        self.assertEqual(identity, reply['records'][0]['id'])
        self.assertEqual('resolved', self.learning()[1]['status'])
        self.assertEqual(PUBLIC['first_progress'], self.now()['progress'])

    def test_changed_owned_frontier_is_preserved_and_the_same_result_saves_progress(self):
        with mock.patch.object(self.result, 'update_record', side_effect=OSError('Kite close not started')):
            with self.assertRaises(OSError):
                self.save('kite-created-frontier', self.data())
        path, record, _ = self.learning()
        self.brain.update_record(self.root, record['id'], expected_revision=record['revision'],
            progress=PUBLIC['person_consequence'], next_steps=[], files=[], sources=[], actor='local', reason='Kite manual recovery edit')
        before = path.read_bytes()
        # The same result ID runs again: the person's edit is held, not
        # overwritten, and the result's own progress is saved.
        reply = self.save('kite-created-frontier', self.data(), revision='auto')
        self.assertEqual('differs', reply['records'][0]['state'])
        self.assertEqual(PUBLIC['first_progress'], self.now()['progress'])
        self.assertEqual(before, path.read_bytes())
        reply = self.save('kite-independent-followup', self.followup(), revision='auto')
        self.assert_followup(reply)
        self.assertEqual(before, path.read_bytes())

    def test_effect_without_checkpoint_is_finished_by_the_same_result(self):
        atomic = self.result.atomic_json
        def fail_checkpoint(path, value):
            if path.parent.parent.name == 'result-attempts' and value.get('stages'):
                raise OSError('Kite checkpoint persistence failed')
            return atomic(path, value)
        with mock.patch.object(self.result, 'atomic_json', side_effect=fail_checkpoint):
            with self.assertRaises(OSError):
                self.save('kite-ambiguous-effect', self.data())
        identity = self.learning()[1]['id']
        # The open learning is exactly this save's: the retry finishes it
        # instead of refusing the result ID for good, and creates no twin.
        reply = self.save('kite-ambiguous-effect', self.data(), revision='auto')
        self.assertEqual(identity, reply['records'][0]['id'])
        self.assertEqual('resolved', self.learning()[1]['status'])
        self.assertEqual(PUBLIC['first_progress'], self.now()['progress'])

    def test_completed_task_frontier_does_not_increment_revision_again(self):
        atomic = self.result.atomic_json
        def fail_complete(path, value):
            if path.parent.parent.name == 'result-attempts' and value.get('state') == 'complete':
                raise OSError('Kite complete persistence failed')
            return atomic(path, value)
        with mock.patch.object(self.result, 'atomic_json', side_effect=fail_complete):
            with self.assertRaises(OSError):
                self.save('kite-task-frontier', self.data())
        before = self.now()
        self.save('kite-task-frontier', self.data(), revision='auto')
        self.assertEqual(before, self.now())

    def test_owned_harness_replay_only_frontier_can_continue(self):
        prepared = self.result.prepared_scope(self.root, 'local', 'Checked public Kite policy', 'agent')
        with mock.patch.object(self.result, 'update_record', side_effect=OSError('Kite close not started')):
            with self.assertRaises(OSError):
                self.result.record_result(self.root, self.task['id'], 'kite-bound-frontier', 1, self.data(),
                    owner='local', reason='Checked public Kite policy', prepared=prepared)
        reply = self.result.record_result(self.root, self.task['id'], 'kite-bound-frontier', 1, self.data(),
            owner='local', reason='Checked public Kite policy', prepared=prepared, replay_only=True)
        self.assertEqual(PUBLIC['first_progress'], self.now()['progress'])
        self.assertIsNotNone(reply['task'])

    def test_owned_harness_replay_finishes_an_effect_without_checkpoint(self):
        prepared = self.result.prepared_scope(self.root, 'local', 'Checked public Kite policy', 'agent')
        def bound(result_id, data, revision, **extra):
            return self.result.record_result(self.root, self.task['id'], result_id, revision, data,
                owner='local', reason='Checked public Kite policy', prepared=prepared, **extra)
        atomic = self.result.atomic_json
        def fail_checkpoint(path, value):
            if path.parent.parent.name == 'result-attempts' and value.get('stages'):
                raise OSError('Kite checkpoint persistence failed')
            return atomic(path, value)
        with mock.patch.object(self.result, 'atomic_json', side_effect=fail_checkpoint):
            with self.assertRaises(OSError):
                bound('kite-bound-effect', self.data(), 1)
        identity = self.learning()[1]['id']
        # The started attempt proves this save was begun: the replay finishes
        # it rather than leaving the run's memory incomplete for good.
        reply = bound('kite-bound-effect', self.data(), 1, replay_only=True)
        self.assertEqual(identity, reply['records'][0]['id'])
        self.assertEqual('resolved', self.learning()[1]['status'])
        self.assertEqual(PUBLIC['first_progress'], self.now()['progress'])
        # A replay with no started attempt still never becomes a first write.
        before = self.now()
        with self.assertRaisesRegex(self.brain.BrainError, 'receipt_unavailable'):
            bound('kite-never-started', self.data(followup=True), before['revision'], replay_only=True)
        self.assertEqual(before, self.now())

    def test_malformed_task_stage_cannot_mark_unsaved_progress_as_completed(self):
        with mock.patch.object(self.result, 'create_record', side_effect=OSError('Kite create not started')):
            with self.assertRaises(OSError):
                self.save('kite-malformed-frontier', self.data())
        marker = self.attempt('kite-malformed-frontier')
        note = json.loads(marker.read_bytes())
        note['stages']['task'] = {'record_id': self.task['id'], 'revision': 1,
            'sha256': self.result._learning_image(self.root, self.task['id']), 'state': 'created', 'done': False}
        marker.write_text(json.dumps(note), encoding='utf-8')
        before = self.now()
        with self.assertRaises(self.brain.BrainError):
            self.save('kite-malformed-frontier', self.data(), revision='auto')
        self.assertEqual(before, self.now())

    def test_frontier_directory_nodes_consume_the_admission_bound(self):
        parent = self.root / 'project-brain/dynamic/empty-nodes'; parent.mkdir()
        for i in range(20):
            (parent / str(i)).mkdir()
        with mock.patch.object(self.result, 'DOMAIN_NODE_LIMIT', 10, create=True):
            self.assertIsNone(self.result._domain_image(self.root))

    def test_malformed_learning_stage_cannot_skip_unverified_claim(self):
        with mock.patch.object(self.result, 'update_record', side_effect=OSError('Kite close not started')):
            with self.assertRaises(OSError):
                self.save('kite-malformed-learning', self.data())
        marker = self.attempt('kite-malformed-learning')
        note = json.loads(marker.read_bytes())
        for stage in note['stages'].values():
            stage['done'] = True
        marker.write_text(json.dumps(note), encoding='utf-8')
        before = self.now(); path, _, _ = self.learning(); raw = path.read_bytes()
        with self.assertRaisesRegex(self.brain.BrainError, 'attempt_incomplete'):
            self.save('kite-malformed-learning', self.data(), revision='auto')
        self.assertEqual(before, self.now()); self.assertEqual(raw, path.read_bytes())

    def test_unavailable_frontier_does_not_block_fresh_save_and_restarts_resume(self):
        with mock.patch.object(self.result, '_domain_image', return_value=None):
            self.save('kite-unavailable-complete', self.data())
            self.assertEqual(PUBLIC['first_progress'], self.now()['progress'])
            self.assertTrue(self.save('kite-unavailable-complete', self.data())['replayed'])
            data = self.followup()
            with mock.patch.object(self.result, 'update_record', side_effect=OSError('Kite update not started')):
                with self.assertRaises(OSError):
                    self.save('kite-unavailable-started', data)
            # No observable frontier: the save starts again instead of
            # leaving the result ID refused for good.
            reply = self.save('kite-unavailable-started', data, revision='auto')
            self.assert_followup(reply)


class MemoryNoticeReviewTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        sys.path.insert(0, str(ROOT / 'harness/src'))
        try:
            from harness import memory_draft
            from harness.task_context import TaskContext
            cls.draft = memory_draft; cls.context = TaskContext
        finally:
            sys.path.pop(0)

    def test_failed_save_does_not_claim_nothing_was_available(self):
        text = self.draft.summary('ready', {'ok': False, 'saved': {}, 'error': 'Kite save incomplete'})
        self.assertNotIn('Nothing new to save', text)
        self.assertIn('Kite save incomplete', text)

    def test_retained_native_learning_keeps_revision_and_fixed_reason_in_notice(self):
        reply = self.context._memory_outcome({'task': {'id': 'kite', 'revision': 3},
            'records': [{'id': 'kite-finding', 'type': 'finding', 'title': PUBLIC['learning']['title'],
                         'status': 'resolved', 'state': 'differs', 'revision': 4, 'reason': 'revision-conflict'}]}, [])
        self.assertTrue(reply['ok'])
        self.assertEqual(4, reply['saved']['records'][0]['revision'])
        self.assertEqual('revision-conflict', reply['saved']['records'][0]['reason'])
        text = self.draft.summary('ready', reply)
        self.assertIn('revision 4', text)
        self.assertIn('progress and next steps', text)

    def test_automatic_parser_retains_learning_when_optional_revision_is_invalid(self):
        for value in ('2', 0):
            payload = {'progress': PUBLIC['first_progress'], 'next_steps': [],
                       'learnings': [{**PUBLIC['learning'], 'expected_revision': value}], 'used_memory': []}
            parsed = self.draft.parse('```memory-draft\n' + json.dumps(payload) + '\n```')
            self.assertEqual(1, len(parsed['learnings']))
            self.assertNotIn('expected_revision', parsed['learnings'][0])


if __name__ == '__main__':
    unittest.main()
