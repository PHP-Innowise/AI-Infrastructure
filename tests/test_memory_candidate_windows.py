"""Layered discovery must deliver project knowledge despite crowded skills."""
from collections import Counter
import importlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
ANSWER = 'Input validation rejects missing account ownership before dispatch.'


class CandidateWindowTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='layer-candidates-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.env = dict(os.environ, GIT_OPTIONAL_LOCKS='0', PYTHONDONTWRITEBYTECODE='1')
        for key in ('ACCELERATOR_HOME', 'ACCELERATOR_PROJECT_DIR', 'ACCELERATOR_STATE_DIR',
                    'PROJECT_BRAIN_MODE', 'CONTEXT_CAPSULE_DELIVERED', 'CONTEXT_MEMORY_RECOVERY'):
            self.env.pop(key, None)
        subprocess.run(['git', 'init', '-q', str(self.root)], env=self.env,
                       check=True, capture_output=True)
        for number in range(32):
            candidate = self.root / ('.agents/skills/check-%02d/SKILL.md' % number)
            candidate.parent.mkdir(parents=True)
            candidate.write_text('---\nname: checker\ndescription: Input validation checks.\n---\n'
                                 '# Input validation\n\nApply input validation before processing '
                                 'external parameters.\n', encoding='utf-8')
        (self.root / 'docs').mkdir()
        (self.root / 'docs/endpoint.md').write_text(
            '# Endpoint contract\n\n' +
            'The existing endpoint has documented compatibility guarantees. ' * 200 +
            '\n' + ANSWER, encoding='utf-8')

    def refresh(self, query='input validation', limit=3):
        result = subprocess.run([sys.executable, '-B',
            str(ROOT / 'PHP Core/memory-bank/scripts/context.py'), '--root', str(self.root),
            'refresh', '--query', query, '--task-id', 'TASK-CANDIDATE', '--limit', str(limit),
            '--ephemeral', '--json'], env=self.env, capture_output=True, timeout=25)
        self.assertEqual(0, result.returncode, result.stderr.decode('utf-8'))
        return json.loads(result.stdout)

    def test_default_window_delivers_exact_project_answer_despite_32_skills(self):
        result = self.refresh()
        self.assertIn(ANSWER, result['capsule_text'])
        semantic = result['capsule']['semantic']
        self.assertEqual(['docs/endpoint.md'], [item['path'] for item in semantic])
        # The compact public item omits its default "covered" marker.
        self.assertEqual('covered', semantic[0].get('match', 'covered'))
        self.assertLessEqual(len(result['capsule_text']), 3600)

    def test_wider_output_limit_keeps_the_same_project_answer(self):
        result = self.refresh(limit=4)
        self.assertIn(ANSWER, result['capsule_text'])
        self.assertEqual(['docs/endpoint.md'],
                         [item['path'] for item in result['capsule']['semantic']])

    def test_unmatched_subject_does_not_receive_project_answer(self):
        result = self.refresh('meteorite canyon')
        self.assertEqual([], result['capsule']['semantic'])
        self.assertNotIn(ANSWER, result['capsule_text'])

    def test_warm_source_becoming_ignored_is_not_delivered(self):
        self.refresh(limit=4)
        (self.root / '.gitignore').write_text('docs/endpoint.md\n', encoding='utf-8')
        result = self.refresh()
        self.assertEqual([], result['capsule']['semantic'])
        self.assertNotIn(ANSWER, result['capsule_text'])

    def observed_target(self, privacy='public'):
        answer = 'Lease generation admission checks the persisted revision before reusing the ownership token.'
        config = self.root/'project-brain/config/runtime.json'
        config.parent.mkdir(parents=True, exist_ok=True)
        config.write_text(json.dumps({'mode': 'governed', 'allowed_authority': ['observed'],
                                     'allowed_privacy': ['public', 'team'] if privacy=='public' else ['team'],
                                     'automatic_promotion': False, 'automatic_compaction': False}))
        source = self.root/'src/public-contract.txt'
        source.parent.mkdir()
        source.write_text('Public canonical source declares:\n'+answer+'\n')
        created = subprocess.run([sys.executable, '-B', str(ROOT/'PHP Core/memory-bank/scripts/context.py'),
            '--root', str(self.root), 'brain-create', 'finding', '--external-id', 'OBS-WINDOW',
            '--title', 'Field observation', '--goal', 'Public project context identifies the active workflow. '+answer,
            '--authority', 'observed', '--privacy', privacy, '--source', 'src/public-contract.txt', '--json'],
            env=self.env, capture_output=True, timeout=20)
        self.assertEqual(0, created.returncode, created.stderr.decode())
        return answer, config

    def denied_cohort(self):
        for number in range(32):
            (self.root/('docs/cohort-%02d.md'%number)).write_text(
                '# Lease generation\n\nLease generation reference discusses routine inventory checks.\n')

    def test_configured_authority_precedes_bounded_governed_window(self):
        answer, _ = self.observed_target()
        self.assertIn(answer, self.refresh('lease generation')['capsule_text'])
        self.denied_cohort()
        result = self.refresh('lease generation')
        self.assertIn(answer, result['capsule_text'])
        self.assertTrue(result['capsule']['semantic'])
        self.assertFalse(any(item['path'].startswith('docs/cohort-') for item in result['capsule']['semantic']))
        self.assertLessEqual(len(result['capsule']['semantic']), 3)

    def test_only_denied_authority_has_no_eligible_lexical_match(self):
        _, _ = self.observed_target()
        (self.root/'src/public-contract.txt').unlink()
        self.denied_cohort()
        result = self.refresh('lease generation')
        self.assertEqual([], result['capsule']['semantic'])
        self.assertIn('semantic', result['capsule']['no_match'])

    def test_default_authorities_keep_allowed_cohort_competition(self):
        _, config = self.observed_target()
        self.denied_cohort()
        config.write_text(json.dumps({'mode': 'governed', 'allowed_authority': ['observed', 'verified'],
                                     'automatic_promotion': False, 'automatic_compaction': False}))
        result = self.refresh('lease generation')
        self.assertTrue(any(item['path'].startswith('docs/cohort-') for item in result['capsule']['semantic']))
        self.assertLessEqual(len(result['capsule']['semantic']), 3)

    def test_team_only_privacy_precedes_bounded_governed_window(self):
        answer, config = self.observed_target(privacy='team')
        settings = json.loads(config.read_text())
        settings['allowed_authority'] = ['observed', 'verified']
        config.write_text(json.dumps(settings))
        self.assertIn(answer, self.refresh('lease generation')['capsule_text'])
        self.denied_cohort()
        result = self.refresh('lease generation')
        self.assertIn(answer, result['capsule_text'])
        self.assertFalse(any(item['path'].startswith('docs/cohort-') for item in result['capsule']['semantic']))

    def test_only_denied_privacy_has_no_eligible_lexical_match(self):
        _, config = self.observed_target(privacy='team')
        settings = json.loads(config.read_text())
        settings['allowed_authority'] = ['observed', 'verified']
        config.write_text(json.dumps(settings))
        (self.root/'src/public-contract.txt').unlink()
        self.denied_cohort()
        result = self.refresh('lease generation')
        self.assertEqual([], result['capsule']['semantic'])
        self.assertIn('semantic', result['capsule']['no_match'])

    def runtime(self):
        directory = ROOT / 'PHP Core/memory-bank/scripts'
        names = [candidate.stem for candidate in directory.glob('*.py')]
        previous = {name: sys.modules.pop(name) for name in names if name in sys.modules}
        def restore():
            for name in names:
                sys.modules.pop(name, None)
            sys.modules.update(previous)
        self.addCleanup(restore)
        sys.path.insert(0, str(directory))
        try:
            context = importlib.import_module('context')
            retrieval = importlib.import_module('context_retrieval')
        finally:
            sys.path.remove(str(directory))
        connection = context.connect(self.root / 'rank.db')
        self.addCleanup(connection.close)
        retrieval.ensure_metadata_tables(connection)
        return connection, retrieval

    def test_each_layer_is_bounded_and_ties_are_deterministic(self):
        connection, retrieval = self.runtime()
        for layer in ('procedural', 'semantic', 'episodic'):
            for number in range(15):
                name = 'docs/%s-%02d.md' % (layer, number)
                connection.execute('INSERT INTO documents VALUES(?,?,?,?,?,?)',
                    (name, layer, 'spec', 'Zirconium gateway', '', 'Zirconium gateway rule.'))
                connection.execute('INSERT INTO document_metadata VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)',
                    (name, 'evidence', 'public', '*', 'verified', 'active', 'a' * 64,
                     None, '[]', '[]', '', 1.0, ''))
        connection.commit()
        trace = retrieval.RetrievalTrace({'owners': ['*'], 'allowed_privacy': ['public']})
        items, _ = retrieval._candidates(connection, 'zirconium gateway', 4,
                                          layer_windows=True, trace=trace)
        self.assertEqual(Counter(procedural=4, semantic=4, episodic=4),
                         Counter(item['layer'] for item in items))
        self.assertTrue(all(item['match'] == 'covered' for item in items))
        paths = [item['path'] for item in items]
        self.assertEqual(12, len(set(paths)))
        repeated, _ = retrieval._candidates(connection, 'zirconium gateway', 4,
                                             layer_windows=True)
        self.assertEqual(paths, [item['path'] for item in repeated])
        self.assertEqual(12, trace.search_limit)
        legacy, _ = retrieval._candidates(connection, 'zirconium gateway', 4)
        self.assertEqual(4, len(legacy))

    def test_authority_window_preserves_corpus_scores_bounds_and_raw_callers(self):
        connection, retrieval = self.runtime()
        for layer in ('procedural', 'semantic', 'episodic'):
            for authority, prefix in (('verified', 'a-denied'), ('observed', 'z-allowed')):
                for number in range(8):
                    name = 'docs/%s-%s-%02d.md'%(prefix, layer, number)
                    connection.execute('INSERT INTO documents VALUES(?,?,?,?,?,?)',
                        (name, layer, 'spec', 'Zirconium gateway', '', 'Zirconium gateway rule.'))
                    connection.execute('INSERT INTO document_metadata VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)',
                        (name, 'evidence', 'team' if authority=='observed' else 'public', '*', authority,
                         'active', 'a'*64, None, '[]', '[]', '', 1.0, ''))
        connection.commit()
        raw, _ = retrieval._candidates(connection, 'zirconium gateway', 100, layer_windows=True)
        scores = {item['path']: item['score'] for item in raw}
        trace = retrieval.RetrievalTrace({'owners': ['*'], 'allowed_privacy': ['public', 'team']})
        allowed, _ = retrieval._candidates(connection, 'zirconium gateway', 4, layer_windows=True,
                                            allowed_authority=['observed'], trace=trace)
        self.assertEqual(Counter(procedural=4, semantic=4, episodic=4), Counter(item['layer'] for item in allowed))
        self.assertTrue(all(item['authority']=='observed' and item['score']==scores[item['path']] for item in allowed))
        self.assertEqual(12, trace.search_limit)
        private_window = retrieval._candidates(connection, 'zirconium gateway', 4, layer_windows=True,
                                                allowed_privacy=['team'])[0]
        self.assertEqual([item['path'] for item in allowed], [item['path'] for item in private_window])
        self.assertEqual([], retrieval._candidates(connection, 'zirconium gateway', 4, layer_windows=True,
                                                   allowed_privacy=[])[0])
        self.assertEqual([], retrieval._candidates(connection, 'zirconium gateway', 4, layer_windows=True,
                                                   allowed_authority=[])[0])
        first = retrieval._candidates(connection, 'zirconium gateway', 4, layer_windows=True,
                                      allowed_authority=['observed', 'verified'])[0]
        second = retrieval._candidates(connection, 'zirconium gateway', 4, layer_windows=True,
                                       allowed_authority=['verified', 'observed', 'observed'])[0]
        self.assertEqual([item['path'] for item in first], [item['path'] for item in second])
        legacy, _ = retrieval._candidates(connection, 'zirconium gateway', 4)
        self.assertEqual(4, len(legacy))
        self.assertTrue(all(item['authority']=='verified' for item in legacy))


if __name__ == '__main__':
    unittest.main()
