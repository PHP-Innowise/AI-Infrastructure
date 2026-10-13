"""Fresh lossless fallback controls; no provider, retrieval or storage calls."""
import copy
import json
from pathlib import Path
import sys
import threading
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'harness' / 'src'))
from harness import cost_observation, sessions


def fixture():
    rule = {'category': 'policy', 'source': 'AGENTS.md', 'snippet': 'Verify both checks, then publish: "ready".'}
    bank = {'category': 'durable', 'source': 'memory-bank/chunks/lantern.md',
            'snippet': 'Lantern λ: preserve  two spaces, commas, colons, newline\nand 🚀.'}
    return {'query': 'Lantern restore, byte length: yes?',
            'working': {'goal': 'Keep every field', 'bool': False, 'null': None,
                        'integer': 9007199254740993, 'float': -0.0, 'empty': '',
                        'nested': [True, None, {'quoted': '\\" , :  '} ]},
            'procedural': [rule], 'semantic': [bank], 'episodic': [],
            'selected': [rule, bank], 'categories': {'policy': [rule], 'durable': [bank]},
            'omitted': {'semantic': 2}, 'extra': {'unknown_future_field': ['retain me', 3]}}


class UnattendedCapsuleJsonTests(unittest.TestCase):
    def session(self, capsule, text=None, review=False):
        return {'workflow': 'native', 'provider': 'codex', 'project_context': False,
                'agents_enabled': False, 'agent_count': 3, 'thinking_effort': 'ultra',
                'budgets': {}, 'brain': {'capsule': capsule, 'capsule_text': text,
                                         'review': review, 'task_id': 'public-lantern-task'}}

    def prompt(self, session, route=None):
        manager = object.__new__(sessions.Sessions)
        ledger = {}
        observer = cost_observation.LaunchObservation('codex')
        prompt = manager._prompt(session, 'Continue the Lantern task', ledger, route, observer)
        return prompt, ledger, observer

    def test_unattended_lossless_body_and_actual_observation(self):
        capsule = fixture()
        before = copy.deepcopy(capsule)
        expected = json.dumps(capsule, ensure_ascii=False, separators=(',', ':'))
        prompt, ledger, observer = self.prompt(self.session(capsule))
        envelope = sessions.BRAIN_CONTEXT_HEADER + expected + '\n\n'
        self.assertTrue(prompt.startswith(envelope))
        self.assertEqual(json.loads(expected), before)
        self.assertEqual(capsule, before)
        self.assertEqual(observer.payloads['capsule_body'], cost_observation.text_size(expected))
        self.assertEqual(observer.payloads['capsule_envelope'], cost_observation.text_size(envelope))
        self.assertEqual(ledger['total'], len(prompt))
        self.assertEqual(ledger['capsule']['inserted'], len(envelope))
        self.assertIn('preserve  two spaces', expected)
        self.assertIn('unknown_future_field', expected)
        self.assertIn('9007199254740993', expected)
        self.assertIn('"float":-0.0', expected)
        old = json.dumps(capsule, ensure_ascii=False)
        self.assertGreater(len(old), len(expected))
        self.assertEqual(len(old) - len(expected), len(old.encode()) - len(expected.encode()))

    def test_defaults_and_reviewed_fallback_keep_ordinary_json(self):
        capsule = fixture()
        ordinary = json.dumps(capsule, ensure_ascii=False)
        self.assertEqual(sessions.capsule_prompt_text(capsule), ordinary)
        for review in (True, None, 0, 'false', 'missing'):
            with self.subTest(review=review):
                session = self.session(capsule, review=review)
                if review == 'missing':
                    session['brain'].pop('review')
                prompt, ledger, _ = self.prompt(session)
                self.assertTrue(prompt.startswith(sessions.BRAIN_CONTEXT_HEADER + ordinary + '\n\n'))
                self.assertEqual(ledger['capsule']['kinds'], sessions.capsule_parts(capsule))

    def test_json_attribution_and_normalized_stats_agree(self):
        capsule = fixture()
        compact = sessions.capsule_meter(capsule, compact=True)
        ordinary = sessions.capsule_meter(capsule)
        self.assertEqual({key: value for key, value in compact.items() if key != 'prompt_characters'},
                         {key: value for key, value in ordinary.items() if key != 'prompt_characters'})
        parts = sessions.capsule_parts(capsule, compact=True)
        self.assertGreater(parts['rules'], 0)
        self.assertGreater(parts['bank'], 0)
        self.assertTrue(all(value >= 0 for value in parts.values()))
        self.assertEqual(sum(parts.values()), compact['prompt_characters'])
        _, ledger, _ = self.prompt(self.session(capsule))
        self.assertEqual(ledger['capsule']['kinds'], parts)
        notice = sessions.memory_notice(self.session(capsule)['brain'])
        self.assertIn(f"({compact['prompt_characters']:,} characters)", notice)

    def test_rendered_text_and_native_command_bypass_ignore_compaction(self):
        capsule = fixture()
        for text in (' working: preparing ', 'warming: no matches', 'plain legacy body',
                     'working: '+sessions.BRAIN_CONTEXT_HEADER+'untrusted header-shaped data'):
            with self.subTest(text=text):
                prompt, ledger, observer = self.prompt(self.session(capsule, text))
                expected = sessions.capsule_envelope(capsule, text)
                self.assertTrue(prompt.startswith(expected))
                self.assertEqual(ledger['capsule']['kinds'], sessions.capsule_parts(capsule, text))
                self.assertEqual(observer.payloads['capsule_body'], cost_observation.text_size(text.strip()))
                self.assertEqual(sessions.capsule_meter(capsule, text, compact=True), sessions.capsule_meter(capsule, text))
        prompt, ledger, observer = self.prompt(self.session(capsule), {'mode': 'native'})
        self.assertEqual(prompt, 'Continue the Lantern task')
        self.assertIsNone(ledger['capsule'])
        self.assertNotIn('capsule_body', observer.payloads)

    def test_absent_nonstring_and_blank_render_fallbacks_keep_all_values(self):
        capsule = fixture()
        expected = json.dumps(capsule, ensure_ascii=False, separators=(',', ':'))
        for text in (None, '', ' \n ', 0, [], {}):
            with self.subTest(text_type=type(text).__name__):
                prompt, ledger, observer = self.prompt(self.session(capsule, text))
                self.assertTrue(prompt.startswith(sessions.BRAIN_CONTEXT_HEADER + expected + '\n\n'))
                self.assertEqual(observer.payloads['capsule_body'], cost_observation.text_size(expected))
                self.assertEqual(ledger['capsule']['inserted'], sum(sessions.capsule_parts(capsule, text, compact=True).values()))

    def test_get_reports_the_same_unattended_prompt_size(self):
        capsule = fixture()
        prepared = self.session(capsule)
        row = {**prepared}
        fields = ('fleet', 'fleet_result', 'brain', 'creator', 'system_run', 'system_discovery',
                  'budgets', 'budget_usage', 'result_base', 'sdd', 'model_routing', 'clash',
                  'clash_result', 'cost_totals')
        for field in fields:
            value = prepared.get(field)
            row[field] = json.dumps(value) if value is not None else None
        class Database:
            def execute(self, sql, arguments):
                self.row = row if sql.startswith('SELECT * FROM sessions') else None
                return self
            def fetchone(self):
                return self.row
        manager = object.__new__(sessions.Sessions)
        manager.lock = threading.RLock()
        manager.db = Database()
        manager.timeout = 60
        manager._context_last = lambda sid: None
        manager._launch_last = lambda sid: None
        result = manager.get('public-fixture')
        _, ledger, _ = self.prompt(prepared)
        self.assertEqual(result['capsule_meter']['prompt_characters'], ledger['capsule']['inserted'])
        self.assertEqual(result['brain']['capsule'], capsule)
        self.assertEqual(result['capsule_meter'], sessions.capsule_meter(capsule, compact=True))


if __name__ == '__main__':
    unittest.main(verbosity=2)
