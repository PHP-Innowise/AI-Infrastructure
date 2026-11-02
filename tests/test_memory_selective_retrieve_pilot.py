"""One original prompt hook, exact transport, experimental profile routing only."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / 'scripts/memory_selective_retrieve_pilot.py'
sys.path.insert(0, str(SCRIPT.parent))


class SelectiveWrapperTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.hook = self.root / 'hook.py'
        self.hook.write_text('''import os,sys,json
from pathlib import Path
p=Path('count'); p.write_text(str(int(p.read_text())+1) if p.exists() else '1')
Path('input.bin').write_bytes(sys.stdin.buffer.read())
Path('environment.json').write_text(json.dumps({k:os.environ.get(k) for k in ('CONTEXT_CAPSULE_PROFILE','CONTEXT_TASK_ID','SENTINEL')}))
sys.stdout.buffer.write(b'capsule\\r\\n\\xff'); sys.stderr.buffer.write(b'diagnostic\\r\\n')
raise SystemExit(7)
''')
        self.rules = self.root / 'rules.json'
        self.rules.write_text(json.dumps({'version': 1, 'historical': ['раньше', 'why'],
                                        'decisions': ['отказались', 'reject']}))
        self.audit = self.root / 'audit.jsonl'
        self.env = {**os.environ, 'CONTEXT_CAPSULE_PROFILE': 'navigation',
                    'CONTEXT_TASK_ID': 'unchanged-task', 'SENTINEL': 'keep'}

    def run_wrapper(self, prompt, *extra):
        import shlex
        raw = json.dumps({'prompt': prompt, 'session_id': 'private-session',
                          'transcript_path': '/private/transcript'}, ensure_ascii=False).encode()
        result = subprocess.run([sys.executable, '-B', str(SCRIPT), '--hook',
            shlex.join([sys.executable, str(self.hook)]), '--rules', str(self.rules),
            '--audit', str(self.audit), '--runtime-dir', str(ROOT / 'PHP Core/memory-bank/scripts'), *extra],
            input=raw, cwd=self.root, env=self.env, capture_output=True)
        return raw, result

    def test_disabled_path_does_not_read_rules_and_keeps_transport(self):
        self.rules.unlink()
        raw, result = self.run_wrapper('why reject Redis')
        self.assertEqual(7, result.returncode, result.stderr)
        self.assertEqual(raw, (self.root / 'input.bin').read_bytes())
        self.assertEqual(b'capsule\r\n\xff', result.stdout)
        self.assertEqual('navigation', json.loads((self.root / 'environment.json').read_text())['CONTEXT_CAPSULE_PROFILE'])
        self.assertFalse(self.audit.exists())

    def test_enabled_historical_and_ordinary_payload_run_once_without_changes(self):
        for prompt, profile in [('Почему раньше отказались от Redis?', 'full'), ('Добавь Redis в новый модуль', 'recall')]:
            raw, result = self.run_wrapper(prompt, '--enabled')
            self.assertEqual(7, result.returncode, result.stderr)
            self.assertEqual(raw, (self.root / 'input.bin').read_bytes())
            self.assertEqual(b'capsule\r\n\xff', result.stdout)
            env = json.loads((self.root / 'environment.json').read_text())
            self.assertEqual(profile, env['CONTEXT_CAPSULE_PROFILE'])
            self.assertEqual('unchanged-task', env['CONTEXT_TASK_ID']); self.assertEqual('keep', env['SENTINEL'])
        self.assertEqual('2', (self.root / 'count').read_text())
        records = [json.loads(line) for line in self.audit.read_text().splitlines()]
        self.assertEqual(['full', 'recall'], [r['profile'] for r in records])
        self.assertNotIn('private-session', str(records)); self.assertNotIn('transcript', str(records))

    def test_rules_failure_falls_back_before_hook_and_secret_is_not_audited(self):
        self.rules.write_text('{broken')
        prompt = 'why reject Redis password="not-a-real-example-credential" $(touch injected)'
        raw, result = self.run_wrapper(prompt, '--enabled')
        self.assertEqual(7, result.returncode)
        self.assertEqual('1', (self.root / 'count').read_text())
        self.assertEqual(raw, (self.root / 'input.bin').read_bytes())
        self.assertFalse((self.root / 'injected').exists())
        report = self.audit.read_text()
        self.assertNotIn('credential', report); self.assertNotIn(prompt, report)
        self.assertEqual('recall', json.loads(report)['profile'])
        self.assertEqual('routing-unavailable', json.loads(report)['reason'])

    def test_choose_profile_rejects_unbounded_or_regex_rules(self):
        from memory_selective_retrieve_pilot import choose_profile
        for rules in ({'version': 1, 'regex': '.*'}, {'version': 1, 'historical': ['x' * 200], 'decisions': ['a']}):
            with self.assertRaises(ValueError): choose_profile('why', rules)

    def test_observations_validate_digests_and_do_not_authenticate_producer(self):
        from memory_selective_retrieve_pilot import parse_observations
        _, result = self.run_wrapper('why reject Redis', '--enabled')
        record = json.loads(self.audit.read_text())
        expected = {key: record[key] for key in ('rules_digest', 'router_digest', 'sanitizer_digest')}
        observed = parse_observations(self.audit.read_bytes(), expected)
        self.assertEqual('observed', observed['status'])
        self.assertFalse(observed['producer_authenticated'])
        record['router_digest'] = '0' * 64
        self.assertEqual('unknown', parse_observations(json.dumps(record).encode(), expected)['status'])

    def test_default_routes_past_states_without_decision_words(self):
        from memory_selective_retrieve_pilot import choose_profile, DEFAULT_RULES
        for query in ('Какой порог был раньше?', 'Какой лимит действовал в прошлом месяце?',
                      'Which timeout was configured before the migration?',
                      'Recover the former cleanup retention.',
                      'What was the old limit and what is configured currently?'):
            with self.subTest(query=query):
                self.assertEqual('full', choose_profile(query, DEFAULT_RULES))

    def test_current_procedural_and_continuation_stay_compact(self):
        from memory_selective_retrieve_pilot import choose_profile, DEFAULT_RULES
        for query in ('Почему тест падает?', 'What is the current timeout?',
                      'Добавь previous_item в новый DTO', 'Implement a previousStatus property',
                      'Как выполнять проверку миграций?', 'Дальше', 'Continue'):
            with self.subTest(query=query):
                self.assertEqual('recall', choose_profile(query, DEFAULT_RULES))

    def test_route_reports_purpose_without_exposing_query(self):
        from memory_selective_retrieve_pilot import DEFAULT_RULES
        self.rules.write_text(json.dumps(DEFAULT_RULES), encoding='utf-8')
        for query, reason, profile in [
            ('Какой порог был раньше?', 'historical-state', 'full'),
            ('Почему раньше отказались от Redis?', 'historical-decision', 'full'),
            ('What is the current timeout?', 'current-state', 'recall'),
            ('How to run the review workflow?', 'procedural', 'recall'),
            ('Дальше', 'continuation', 'recall')]:
            with self.subTest(query=query):
                raw, result = self.run_wrapper(query, '--enabled')
                self.assertEqual(7, result.returncode)
                self.assertEqual(raw, (self.root/'input.bin').read_bytes())
                record=json.loads(self.audit.read_text().splitlines()[-1])
                self.assertEqual(reason, record['reason'])
                self.assertEqual(profile, record['profile'])
                self.assertNotIn(query, json.dumps(record))

    def test_legacy_rules_keep_their_original_routing(self):
        from memory_selective_retrieve_pilot import choose_profile
        legacy={'version':1,'historical':['раньше','why'],'decisions':['отказались','reject']}
        self.assertEqual('recall',choose_profile('Какой порог был раньше?',legacy))
        self.assertEqual('full',choose_profile('why reject Redis',legacy))

    def test_long_prompt_routing_keeps_clipped_query_within_bound_and_original_transport(self):
        from memory_selective_retrieve_pilot import DEFAULT_RULES
        self.rules.write_text(json.dumps(DEFAULT_RULES),encoding='utf-8')
        prefix='What was the previous timeout? '
        for length in (24000,24001,30000):
            with self.subTest(length=length):
                prompt=prefix+'x'*(length-len(prefix))
                raw,result=self.run_wrapper(prompt,'--enabled')
                self.assertEqual(7,result.returncode)
                self.assertEqual(raw,(self.root/'input.bin').read_bytes())
                record=json.loads(self.audit.read_text().splitlines()[-1])
                self.assertEqual(('full','historical-state'),(record['profile'],record['reason']))


if __name__ == '__main__': unittest.main()
