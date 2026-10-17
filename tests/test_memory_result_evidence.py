"""What a memory save leaves in the shared Brain under project-brain/control.

One small proof per finished save, in a folder per task; a bounded number per
task; gone with the records compaction archives; and never one file for two
different saves, so two clones' evidence merges without a conflict.
"""
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
REASON = 'Checked public Kite policy'


class ResultEvidenceTests(unittest.TestCase):
    def setUp(self):
        clean = {k: v for k, v in os.environ.items() if not k.startswith(('GIT_', 'CONTEXT_', 'ACCELERATOR_'))}
        environment = mock.patch.dict(os.environ, clean, clear=True)
        environment.start(); self.addCleanup(environment.stop)
        sys.path.insert(0, str(ROOT / 'PHP Core/memory-bank/scripts'))
        try:
            self.result = importlib.import_module('memory_results')
            self.brain = importlib.import_module('brain_runtime')
        finally:
            sys.path.pop(0)
        folder = tempfile.TemporaryDirectory(prefix='kite-evidence-')
        self.addCleanup(folder.cleanup)
        self.base = Path(folder.name)
        self.root = self.base / 'one'
        source = self.root / PUBLIC['source_path']; source.parent.mkdir(parents=True)
        source.write_text(PUBLIC['source'], encoding='utf-8')
        config = self.root / 'project-brain/config/runtime.json'; config.parent.mkdir(parents=True)
        config.write_text(json.dumps({'mode': 'governed', 'automatic_promotion': False}), encoding='utf-8')
        (self.root / 'memory-bank/chunks').mkdir(parents=True)
        self.task = self.brain.create_task(self.root, PUBLIC['task_alias'], PUBLIC['task_title'], [], [], owner='local')
        self.attempts = self.root / 'project-brain/control/result-attempts'

    def data(self, progress=None, *, learnings=True):
        return {'progress': progress or PUBLIC['first_progress'], 'next_steps': [],
                'learnings': [copy.deepcopy(PUBLIC['learning'])] if learnings else [], 'verified': True}

    def save(self, result_id, data=None, *, root=None, task=None):
        return self.result.record_result(root or self.root, (task or self.task)['id'], result_id, 'auto',
                                         data or self.data(), owner='local', reason=REASON)

    def evidence(self, root=None):
        folder = (root or self.root) / 'project-brain/control/result-attempts'
        return sorted(path for path in folder.rglob('*') if path.is_file())

    def test_a_finished_save_leaves_only_what_a_replay_checks(self):
        self.save('kite-proof')
        [proof] = self.evidence()
        self.assertEqual(self.task['id'], proof.parent.name)
        value = json.loads(proof.read_text(encoding='utf-8'))
        self.assertEqual({'profile', 'task_id', 'result_id', 'digest', 'owner', 'attestation', 'state'}, set(value))
        self.assertEqual(('memory-result-proof-v1', 'complete'), (value['profile'], value['state']))
        self.assertLess(proof.stat().st_size, 400)
        # The proof alone still makes the replay passive.
        self.result._receipt(self.root, self.task['id'], 'kite-proof').unlink()
        self.assertTrue(self.save('kite-proof')['replayed'])

    def test_two_clones_never_write_one_file_for_two_saves(self):
        clones = [self.root]
        for name in ('two', 'three'):
            shutil.copytree(self.root, self.base / name)
            clones.append(self.base / name)
        self.save('kite-shared', root=clones[0])
        self.save('kite-shared', root=clones[1])
        # Another developer reuses the result ID for other content: a save of
        # its own, not a refusal.
        reply = self.save('kite-shared', self.data(PUBLIC['followup_progress']), root=clones[2])
        self.assertFalse(reply['replayed'])
        same, twin, other = ([path.relative_to(root) for path in self.evidence(root)] for root in clones)
        # The same save finished in two clones is the same file, byte for
        # byte, which merges cleanly...
        self.assertEqual(same, twin)
        self.assertEqual([(clones[0] / path).read_bytes() for path in same],
                         [(clones[1] / path).read_bytes() for path in twin])
        # ...and a different save under the same result ID is another file.
        self.assertEqual(1, len(other))
        self.assertNotEqual(same, other)

    def test_finished_proofs_per_task_are_bounded_and_started_ones_stay(self):
        with mock.patch.object(self.result, 'create_record', side_effect=OSError('Kite creation did not start')):
            with self.assertRaises(OSError):
                self.save('kite-interrupted')
        [started] = self.evidence()
        with mock.patch.object(self.result, 'PROOFS_KEPT', 3):
            for n in range(6):
                self.save(f'kite-{n}', self.data(f'Kite progress {n}', learnings=False))
        kept = {path: json.loads(path.read_text(encoding='utf-8')) for path in self.evidence()}
        self.assertEqual(3, sum(value['state'] == 'complete' for value in kept.values()))
        self.assertEqual('started', kept[started]['state'])
        self.assertIn('kite-5', {value['result_id'] for value in kept.values()})
        # The interrupted save still finishes from its own attempt.
        self.save('kite-interrupted')
        self.assertEqual('resolved', self.brain.find_record(
            self.root, self.result.learning_id(self.task['id'], PUBLIC['learning']), include_archive=True)[1]['status'])

    def test_a_receipt_from_before_uuid_keys_still_proves_the_save(self):
        # The runtime before named the task in a receipt as the caller did,
        # usually by its branch alias, and kept no shared proof: after an
        # upgrade the exact replay was refused for good (receipt_unavailable).
        self.save('kite-legacy')
        receipt = self.result._receipt(self.root, self.task['id'], 'kite-legacy')
        value = json.loads(receipt.read_text(encoding='utf-8'))
        value['task_id'] = self.task['external_id']
        receipt.write_text(json.dumps(value), encoding='utf-8')
        for proof in self.evidence():
            proof.unlink()
        self.assertTrue(self.save('kite-legacy')['replayed'])
        self.assertEqual(self.task['id'], json.loads(receipt.read_text(encoding='utf-8'))['task_id'])

    def test_compaction_takes_the_evidence_of_what_it_archives(self):
        self.save('kite-archived')
        images = self.root / 'project-brain/control/result-learning-images'
        self.assertEqual(1, len(list(images.glob('*.json'))))
        # A file of the layout before folders per task: nothing reads it.
        stray = self.attempts / ('0' * 64 + '.json')
        stray.write_text('{}', encoding='utf-8')
        keeper = self.brain.create_task(self.root, 'feature/kite-keeper', 'Kite keeper', [], [], owner='local')
        self.save('kite-kept', self.data(PUBLIC['followup_progress'], learnings=False), task=keeper)
        task = self.brain.find_record(self.root, self.task['id'], record_type='task')[1]
        self.brain.update_record(self.root, task['id'], expected_revision=task['revision'], progress=None,
                                 next_steps=[], files=[], sources=[], actor='local',
                                 transition_to='completed', reason='Kite fixture finished')
        self.assertEqual({'moved': 2}, self.brain.compact(self.root))
        self.assertFalse((self.attempts / self.task['id']).exists())
        self.assertFalse(stray.exists())
        self.assertEqual([], list(images.glob('*.json')))
        self.assertEqual(1, len(list((self.attempts / keeper['id']).glob('*.json'))))


if __name__ == '__main__':
    unittest.main()
