"""Fresh real-CLI Cursor continuation checks on owned public temporary projects."""
from __future__ import annotations

from contextlib import closing
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
EDITION = ROOT / 'PHP Core'
FIXTURES = ROOT / 'tests/fixtures'


@unittest.skipUnless(shutil.which('git'), 'owned temporary Git projects require Git')
class CursorContinuationTests(unittest.TestCase):
    def setUp(self):
        self.public = json.loads((FIXTURES / 'cursor_continuation_public.json').read_text(encoding='utf-8'))
        self.expected = json.loads((FIXTURES / 'cursor_continuation_expected.json').read_text(encoding='utf-8'))
        self.projection = json.loads((FIXTURES / 'cursor_continuation_projection_public.json').read_text(encoding='utf-8'))
        self.boundary = json.loads((FIXTURES / 'cursor_continuation_boundary_public.json').read_text(encoding='utf-8'))
        temporary = tempfile.TemporaryDirectory(prefix='juniper-continuation-')
        self.addCleanup(temporary.cleanup)
        self.project = Path(temporary.name)
        scripts = self.project / 'memory-bank/scripts'
        shutil.copytree(EDITION / 'memory-bank/scripts', scripts,
                        ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
        for name in ('config', 'schemas'):
            shutil.copytree(EDITION / 'project-brain' / name,
                            self.project / 'project-brain' / name)
        self.cli = scripts / 'context.py'
        self.environment = {k: v for k, v in os.environ.items()
                            if not k.startswith(('GIT_', 'ACCELERATOR_', 'CONTEXT_'))}
        self.environment.update(GIT_OPTIONAL_LOCKS='0', PYTHONDONTWRITEBYTECODE='1')
        subprocess.run(['git', 'init', '-q', str(self.project)],
                       env=self.environment, check=True, capture_output=True, timeout=10)
        for path_key, text_key in (('source_path', 'source_text'),
                                   ('other_source_path', 'other_source_text')):
            path = self.project / self.public[path_key]
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(self.public[text_key], encoding='utf-8')
        self.rule = self.project / '.cursor/rules/working-memory.mdc'
        self.receipt = self.project / 'memory-bank/local/cursor-rule.json'

    def call(self, *arguments, report=None):
        result = subprocess.run([sys.executable, '-B', str(self.cli),
                                 '--root', str(self.project), *arguments],
                                env=self.environment, input=(json.dumps(report) if report is not None else None),
                                text=True, encoding='utf-8', capture_output=True, timeout=15)
        self.assertEqual(0, result.returncode, result.stderr)
        return json.loads(result.stdout)

    def start(self, task=None, goal=None):
        return self.call('start', '--task-id', task or self.public['task'],
                         '--goal', goal or self.public['goal'], '--json')

    def refresh(self, prompt, *, task=None, host='cursor', gate='shadow', sanitize=True):
        arguments = ['refresh', '--query', prompt, '--task-id', task or self.public['task'],
                     '--host', host, '--gate', gate, '--ephemeral', '--json']
        if sanitize:
            arguments.append('--sanitize')
        return self.call(*arguments)

    def publish(self, report, task=None):
        return self.call('cursor-rule', '--action', 'write', '--phase', 'prompt',
                         '--task-id', task or self.public['task'], '--report-json', report=report)

    def paths(self, report):
        return sorted(item['path'] for item in report['capsule']['semantic'])

    def seed(self, gate='shadow'):
        self.start()
        report = self.refresh(self.public['goal'], gate=gate)
        self.assertEqual(self.expected['continued_semantic_paths'], self.paths(report))
        self.assertEqual('written', self.publish(report)['state'])
        self.assertIn(self.expected['initial_excerpt'], self.rule.read_text(encoding='utf-8'))
        return report

    def test_plain_continuations_retrieve_current_task_sources(self):
        self.seed()
        for prompt in self.public['continuation_prompts']:
            with self.subTest(prompt=prompt):
                report = self.refresh(prompt)
                self.assertEqual(self.expected['continued_query_source'], report['capsule']['query_source'])
                self.assertEqual(self.expected['continued_semantic_paths'], self.paths(report))
                self.publish(report)
                self.assertIn(self.expected['initial_excerpt'], self.rule.read_text(encoding='utf-8'))
                self.assertIn('query: from task goal', self.rule.read_text(encoding='utf-8'))

    def test_continuation_renders_fresh_working_progress(self):
        self.seed()
        self.call('update', '--task-id', self.public['task'], '--revision', 'auto',
                  '--progress', self.public['progress'], '--next-step', self.public['next_step'], '--json')
        report = self.refresh('дальше')
        self.assertEqual(self.expected['progress'], report['capsule']['working']['progress'])
        self.assertIn(self.expected['next_step'], report['capsule_text'])
        self.publish(report)
        self.assertIn(self.expected['progress'], self.rule.read_text(encoding='utf-8'))
        self.assertIn(self.expected['initial_excerpt'], self.rule.read_text(encoding='utf-8'))

    def test_changed_canonical_source_replaces_old_excerpt(self):
        self.seed()
        (self.project / self.public['source_path']).write_text(self.public['changed_source_text'], encoding='utf-8')
        report = self.refresh('continue')
        self.assertEqual(self.expected['continued_semantic_paths'], self.paths(report))
        self.publish(report)
        self.assertIn(self.expected['changed_excerpt'], self.rule.read_text(encoding='utf-8'))
        self.assertNotIn(self.expected['initial_excerpt'], self.rule.read_text(encoding='utf-8'))

    def test_deleted_source_drops_old_rule_citation_under_all_gate_modes(self):
        self.seed()
        (self.project / self.public['source_path']).unlink()
        for mode in ('shadow', 'off', 'enforce'):
            with self.subTest(gate=mode):
                # Restore only the old owned render to test each mode independently.
                self.publish({'capsule_text': 'working: JUNIPER-RELAY — Juniper relay ownership\n'
                                             'memory specs/juniper-relay.md — durable ownership receipt\n'})
                report = self.refresh('go on', gate=mode)
                self.assertEqual(self.expected['deleted_semantic_paths'], self.paths(report))
                self.assertEqual(self.expected['zero_skip_publication'], self.publish(report)['state'])
                self.assertNotIn(self.public['source_path'], self.rule.read_text(encoding='utf-8'))
                self.assertNotIn(self.expected['initial_excerpt'], self.rule.read_text(encoding='utf-8'))

    def test_new_task_uses_its_current_goal(self):
        self.seed()
        self.start(self.public['other_task'], self.public['other_goal'])
        report = self.refresh('go on', task=self.public['other_task'])
        self.assertEqual(self.expected['continued_query_source'], report['capsule']['query_source'])
        self.assertEqual(self.expected['other_semantic_paths'], self.paths(report))
        self.publish(report, self.public['other_task'])
        self.assertNotIn(self.expected['initial_excerpt'], self.rule.read_text(encoding='utf-8'))
        self.assertIn(self.public['other_task'], self.rule.read_text(encoding='utf-8'))

    def test_meaningful_unmatched_and_negative_prompts_do_not_fallback(self):
        self.start()
        for prompt in [self.public['meaningful_no_match'], *self.public['negative_prompts']]:
            with self.subTest(prompt=prompt):
                report = self.refresh(prompt)
                self.assertEqual(self.expected['ordinary_query_source'], report['capsule']['query_source'])
                self.assertEqual([], self.paths(report))

    def test_meaningful_no_match_replaces_old_selected_rule(self):
        self.seed()
        for mode in ('shadow', 'off', 'enforce'):
            with self.subTest(gate=mode):
                self.publish({'capsule_text': 'working: JUNIPER-RELAY — Juniper relay ownership\n'
                                             'memory specs/juniper-relay.md — durable ownership receipt\n'})
                report = self.refresh(self.public['meaningful_no_match'], gate=mode)
                self.assertEqual('prompt', report['capsule']['query_source'])
                self.assertEqual([], self.paths(report))
                self.assertEqual('written', self.publish(report)['state'])
                self.assertIn(self.public['goal'], self.rule.read_text(encoding='utf-8'))
                self.assertNotIn(self.public['source_path'], self.rule.read_text(encoding='utf-8'))

    def test_direct_query_profiles_remain_explicit(self):
        self.start()
        for command in ('retrieve', 'context'):
            with self.subTest(command=command):
                capsule = self.call(command, 'go on', '--task-id', self.public['task'],
                                    '--host', 'cursor', '--ephemeral', '--json')
                self.assertEqual('explicit', capsule['query_source'])
                self.assertEqual([], capsule['semantic'])

    def test_privacy_withheld_and_modified_ack_do_not_fallback(self):
        self.start()
        withheld = self.refresh(self.public['privacy_only'])
        self.assertEqual(self.expected['withheld'], withheld.get('query_withheld'))
        self.assertIsNone(withheld['capsule'])
        report = self.refresh(self.public['privacy_modified_ack'])
        self.assertEqual(self.expected['ordinary_query_source'], report['capsule']['query_source'])
        self.assertEqual([], self.paths(report))

    def test_other_host_and_unsanitized_profiles_keep_prompt_query(self):
        self.start()
        for host, sanitize in (('codex', True), ('claude', True), ('cursor', False)):
            with self.subTest(host=host, sanitize=sanitize):
                report = self.refresh('go on', host=host, sanitize=sanitize)
                self.assertEqual(self.expected['direct_and_other_host_query_source'], report['capsule']['query_source'])

    def test_missing_and_auto_derived_task_do_not_seed_branch_query(self):
        missing = self.refresh('go on')
        self.assertEqual(self.expected['absent_binding_query_source'], missing['capsule']['query_source'])
        self.start(goal=self.boundary['auto_goal'])
        automatic = self.refresh('go on')
        self.assertEqual(self.expected['auto_goal_query_source'], automatic['capsule']['query_source'])

    def test_auto_derived_goal_with_manual_progress_can_continue(self):
        self.start(goal=self.boundary['auto_goal'])
        self.call('update', '--task-id', self.public['task'], '--revision', 'auto',
                  '--progress', self.projection['auto_task_progress'], '--json')
        report = self.refresh('go on')
        self.assertEqual(self.projection['expected_source'], report['capsule']['query_source'])
        self.assertEqual(self.projection['expected_semantic_paths'], self.paths(report))

    def test_raw_128_and_129_character_admission_is_bounded(self):
        self.start()
        for key in ('raw_128', 'raw_129'):
            with self.subTest(boundary=key):
                report = self.refresh(self.boundary[key])
                self.assertEqual(self.boundary['expected_' + key + '_source'], report['capsule']['query_source'])

    def test_terminal_or_broken_bound_task_never_seeds_old_goal(self):
        for status in self.boundary['terminal_statuses']:
            task = self.public['task'] + '-' + status
            with self.subTest(status=status):
                record = self.start(task=task)
                self.call('brain-update', '--record-id', record['task_uuid'], '--revision', 'auto',
                          '--transition', status, '--reason', 'Public terminal continuation control', '--json')
                report = self.refresh('go on', task=task)
                self.assertNotEqual(self.boundary['expected_terminal_source_not'],
                                    (report['capsule'] or {}).get('query_source'))
        self.start()
        # sqlite3's own context manager commits without closing, and Windows
        # cannot remove the temporary project while the database is open.
        with closing(sqlite3.connect(self.project / 'memory-bank/local/context.db')) as connection, connection:
            connection.execute('UPDATE task_bindings SET task_uuid=? WHERE external_id=?',
                               ('00000000-0000-4000-8000-000000000001', self.public['task']))
        broken = self.refresh('go on')
        self.assertNotEqual('task', (broken['capsule'] or {}).get('query_source'))

    def test_repeat_and_legacy_unknown_skip_preserve_owned_render(self):
        self.seed(gate='enforce')
        original = self.rule.read_bytes(), self.receipt.read_bytes()
        for key in ('repeat_skip_report', 'legacy_skip_report'):
            with self.subTest(profile=key):
                self.assertEqual(self.expected['repeat_skip_publication'], self.publish(self.boundary[key])['state'])
                self.assertEqual(original, (self.rule.read_bytes(), self.receipt.read_bytes()))

    def test_real_enforce_repeat_keeps_the_owned_prompt(self):
        self.seed(gate='enforce')
        original = self.rule.read_bytes(), self.receipt.read_bytes()
        report = self.refresh(self.public['goal'], gate='enforce')
        self.assertEqual('repeat-retrieval', report['capsule']['gate']['reason'])
        self.assertEqual('kept', self.publish(report)['state'])
        self.assertEqual(original, (self.rule.read_bytes(), self.receipt.read_bytes()))


if __name__ == '__main__':
    unittest.main()
