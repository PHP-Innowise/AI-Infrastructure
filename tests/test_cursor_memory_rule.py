"""Prospective Cursor custody on owned Iris fixtures; no real native/model calls."""
from __future__ import annotations
import importlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
EDITION = ROOT / 'PHP Core'
TASK = 'BIRCH-BRANCH'
CAPSULE = 'working: BIRCH-BRANCH — Iris transfer\n- memory specs/iris-ledger.md § Owners — Iris ledger\n  Each iris transfer records one owner.'
CHANGED = CAPSULE.replace('one owner', 'two reviewers')


def clean_environment():
    # A test run from inside a Git hook inherits GIT_DIR and friends: `git
    # init` would then reinitialise the repository they name, and the runtime
    # would rightly keep its hands off the rule under an operator-selected
    # index. The fixtures are their own repositories.
    return {k: v for k, v in os.environ.items() if not k.startswith(('GIT_', 'ACCELERATOR_', 'CONTEXT_'))}


def install(root):
    scripts = root / 'memory-bank/scripts'
    shutil.copytree(EDITION / 'memory-bank/scripts', scripts,
                    ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
    subprocess.run(['git', 'init', '-q', str(root)], check=True, capture_output=True, env=clean_environment())
    return scripts / 'context.py'


def stub(root, body):
    """Only retrieval/checkpoint output is fake; maintenance uses copied runtime."""
    cli = root / 'memory-bank/scripts/context.py'
    if not cli.with_name('context-real.py').exists():
        if not cli.with_name('brain_runtime.py').exists():
            install(root)
        cli.rename(cli.with_name('context-real.py'))
    cli.write_text("import pathlib, runpy, sys\n"
        "if sys.argv[1:2] and sys.argv[1] in ('cursor-rule', 'cursor-rule-retire'):\n"
        "    runpy.run_path(str(pathlib.Path(__file__).with_name('context-real.py')), run_name='__main__')\n"
        "    sys.exit(0)\n" + body, encoding='utf-8')


@unittest.skipUnless(os.name == 'posix' and shutil.which('git'), 'POSIX anchored custody profile')
class CursorRuleTests(unittest.TestCase):
    def setUp(self):
        environment = {key: value for key, value in os.environ.items()
                       if not key.startswith(('GIT_', 'ACCELERATOR_', 'CONTEXT_'))}
        environment['GIT_OPTIONAL_LOCKS'] = '0'
        clean = mock.patch.dict(os.environ, environment, clear=True)
        clean.start(); self.addCleanup(clean.stop)
        sys.path.insert(0, str(EDITION / 'memory-bank/scripts'))
        try:
            self.r = importlib.import_module('context_retrieval')
        finally:
            sys.path.pop(0)
        folder = tempfile.TemporaryDirectory(prefix='iris-cursor-custody-')
        self.addCleanup(folder.cleanup)
        self.base = Path(folder.name)
        self.root = self.base / 'project'; self.root.mkdir()
        subprocess.run(['git', 'init', '-q', str(self.root)], check=True, capture_output=True)
        layout = mock.patch.object(self.r.workspace_roots, 'TOOLING_ROOT', self.root)
        layout.start(); self.addCleanup(layout.stop)
        self.rule = self.root / '.cursor/rules/working-memory.mdc'
        self.receipt = self.root / 'memory-bank/local/cursor-rule.json'

    def call(self, action='write', task=TASK, capsule=CAPSULE, **options):
        return self.r.cursor_rule(self.root, action, task=task, capsule=capsule, **options)

    def owned(self):
        self.assertEqual('written', self.call()['state'])
        return self.rule.read_bytes(), self.receipt.read_bytes()

    def preserved(self, old):
        self.assertEqual('preserved', self.call('retire')['state'])
        self.assertEqual(old, (self.rule.read_bytes(), self.receipt.read_bytes()))

    def test_exact_body_keeps_inode_and_changed_excerpt_renews_custody(self):
        self.owned(); inode = self.rule.stat().st_ino
        self.assertEqual('unchanged', self.call()['state'])
        self.assertEqual(inode, self.rule.stat().st_ino)
        self.assertEqual('written', self.call(capsule=CHANGED)['state'])
        self.assertIn(b'two reviewers', self.rule.read_bytes())
        self.assertEqual('removed', self.call('retire')['state'])
        self.assertFalse(self.rule.exists()); self.assertFalse(self.receipt.exists())

    def test_retirement_counts_actual_rule_bytes_and_leaves_other_policy(self):
        raw, _ = self.owned()
        policy = self.rule.with_name('iris-policy.mdc'); policy.write_text('User policy')
        self.assertEqual({'profile': 'cursor-rule-custody-v1', 'state': 'removed',
                          'bytes_removed': len(raw)}, self.call('retire'))
        self.assertEqual('User policy', policy.read_text())
        self.assertFalse((self.root / 'memory-bank/local/context.db').exists())

    def test_legacy_or_custom_rule_is_never_adopted_overwritten_or_removed(self):
        self.rule.parent.mkdir(parents=True)
        for raw in ('Custom policy', '# Working Memory (auto-rendered)\nworking: BIRCH-BRANCH'):
            self.rule.write_text(raw)
            self.assertEqual('preserved', self.call()['state'])
            self.assertEqual('preserved', self.call('retire')['state'])
            self.assertEqual(raw, self.rule.read_text())
        self.assertFalse(self.receipt.exists())

    def test_changed_bytes_preserve_and_exact_replaced_bytes_reconcile(self):
        old = self.owned(); self.rule.write_bytes(old[0] + b' human edit')
        self.preserved((self.rule.read_bytes(), old[1]))
        replacement = self.rule.with_name('replacement'); replacement.write_bytes(old[0])
        replacement.replace(self.rule)
        self.assertEqual('unchanged',self.call()['state'])
        self.assertEqual(old[0],self.rule.read_bytes())
        self.assertEqual(self.r._cursor_identity(self.rule.stat()),json.loads(self.receipt.read_bytes())['file_identity'])
        self.assertEqual('removed',self.call('retire')['state'])

    def test_corrupt_or_duplicate_receipt_is_preserved(self):
        raw, original = self.owned()
        for receipt in (b'{}', original[:-1] + b',"profile":"cursor-rule-custody-v1"}', b'[' * 1100):
            self.receipt.write_bytes(receipt)
            self.preserved((raw, receipt))

    def test_owned_orphan_receipt_recreates_only_the_absent_rule(self):
        raw, _ = self.owned(); self.rule.unlink()
        self.assertEqual('written', self.call()['state'])
        self.assertEqual(raw, self.rule.read_bytes())
        note = json.loads(self.receipt.read_bytes())
        self.assertEqual(self.r._cursor_identity(self.rule.stat()), note['file_identity'])
        self.rule.unlink()
        self.assertEqual('absent', self.call('retire')['state'])
        self.assertFalse(self.receipt.exists())

    def test_orphan_prompt_receipt_does_not_suppress_turn_recreation(self):
        public = json.loads((ROOT / 'tests/fixtures/memory_cursor_recovery_supplement.json').read_text())
        options = {'task': public['task'], 'capsule': public['capsule']}
        self.assertEqual('written', self.call(**options)['state'])
        self.rule.unlink()
        self.assertEqual('absent', self.call('prompt-held', **options)['state'])
        self.assertEqual(public['orphan_turn_state'], self.call(phase='turn', **options)['state'])
        self.assertEqual(public['expected_turn_rule'].encode('utf-8'), self.rule.read_bytes())
        note = json.loads(self.receipt.read_bytes())
        self.assertEqual('turn', note['phase'])
        self.assertEqual(self.r._cursor_identity(self.rule.stat()), note['file_identity'])

    def test_repository_selectors_preserve_an_otherwise_untracked_owned_rule(self):
        public = json.loads((ROOT / 'tests/fixtures/memory_cursor_recovery_supplement.json').read_text())
        old = self.owned()
        for name in public['repository_selectors_preserved']:
            with self.subTest(selector=name), mock.patch.dict(os.environ, {name: str(self.base / 'alternate')}):
                self.preserved(old)

    def test_harmless_git_environment_and_selector_overrides_use_default_index(self):
        public = json.loads((ROOT / 'tests/fixtures/memory_regression_remediation_public.json').read_text())
        with mock.patch.dict(os.environ, public['cursor_env']):
            self.owned()
            self.assertEqual('written', self.call(capsule=CHANGED)['state'])
            self.assertEqual('removed', self.call('retire')['state'])
            self.assertEqual(public['cursor_env']['GIT_EDITOR'], os.environ['GIT_EDITOR'])
        self.owned()
        subprocess.run(['git', '-C', str(self.root), 'add', '-f', str(self.rule)], check=True, capture_output=True)
        old = (self.rule.read_bytes(), self.receipt.read_bytes())
        with mock.patch.dict(os.environ, {'GIT_INDEX_FILE': str(self.base / 'empty-index'),
                'GIT_CONFIG_COUNT': '1', 'GIT_CONFIG_KEY_0': 'core.bare', 'GIT_CONFIG_VALUE_0': 'true'}):
            self.preserved(old)

    def test_legacy_rule_reports_missing_ownership_without_adoption(self):
        self.rule.parent.mkdir(parents=True)
        raw = b'# Working Memory (auto-rendered)\nworking: LEGACY-UNKNOWN\n'
        self.rule.write_bytes(raw)
        result = self.call()
        self.assertEqual('preserved', result['state'])
        self.assertEqual('ownership_unverified', result['reason'])
        self.assertEqual(raw, self.rule.read_bytes())
        self.assertFalse(self.receipt.exists())

    def test_metadata_only_change_reconciles_receipt_without_replacing_rule(self):
        raw, _ = self.owned(); inode = self.rule.stat().st_ino
        os.chmod(self.rule, 0o640)
        self.assertEqual('unchanged', self.call()['state'])
        self.assertEqual(raw, self.rule.read_bytes())
        self.assertEqual(inode, self.rule.stat().st_ino)
        self.assertEqual(self.r._cursor_identity(self.rule.stat()), json.loads(self.receipt.read_bytes())['file_identity'])
        self.assertEqual('removed', self.call('retire')['state'])

    def test_same_body_reconciliation_cannot_adopt_a_concurrent_replacement_inode(self):
        public = json.loads((ROOT / 'tests/fixtures/memory_cursor_recovery_supplement.json').read_text())
        raw, receipt = self.owned(); original_inode = self.rule.stat().st_ino
        os.chmod(self.rule, 0o640)
        read_image = self.r._cursor_image
        reads = []
        def swap_before_final_read(parent, name, limit):
            if name == 'working-memory.mdc':
                reads.append(True)
                if len(reads) == 3:
                    foreign = self.rule.with_name('same-bytes-foreign')
                    foreign.write_bytes(raw)
                    foreign.replace(self.rule)
            return read_image(parent, name, limit)
        with mock.patch.object(self.r, '_cursor_image', side_effect=swap_before_final_read):
            self.assertEqual(public['same_body_replacement_state'], self.call()['state'])
        self.assertEqual(3, len(reads))
        self.assertNotEqual(original_inode, self.rule.stat().st_ino)
        self.assertEqual(raw, self.rule.read_bytes())
        self.assertTrue(public['replacement_receipt_unchanged'])
        self.assertEqual(receipt, self.receipt.read_bytes())

    def test_orphan_receipt_cannot_adopt_concurrently_appearing_leaf(self):
        self.owned(); self.rule.unlink(); original = self.r._cursor_git_untracked
        calls = []
        def probe(root):
            calls.append(True)
            if len(calls) == 2:
                self.rule.write_text('Human replacement', encoding='utf-8')
            return original(root)
        with mock.patch.object(self.r, '_cursor_git_untracked', side_effect=probe):
            self.assertEqual('preserved', self.call()['state'])
        self.assertEqual('Human replacement', self.rule.read_text())

    def test_explicit_migration_binds_one_current_image_and_fresh_complete_render(self):
        public = json.loads((ROOT / 'tests/fixtures/memory_cursor_migration_public.json').read_text())
        self.rule.parent.mkdir(parents=True)
        self.rule.write_text(public['legacy'], encoding='utf-8')
        options = {'task': public['task'], 'capsule': public['capsule']}
        self.assertEqual('preserved', self.call('migrate', **options)['state'])
        self.assertEqual('preserved', self.call('migrate', approved_sha256='0' * 64, **options)['state'])
        self.assertEqual(public['legacy'], self.rule.read_text(encoding='utf-8'))
        result = self.call('migrate', approved_sha256=public['approved_sha256'], **options)
        self.assertEqual(public['expected_state'], result['state'])
        self.assertEqual(public['expected_rule'].encode('utf-8'), self.rule.read_bytes())
        note = json.loads(self.receipt.read_text())
        self.assertEqual(self.r._cursor_identity(self.rule.stat()), note['file_identity'])
        self.assertEqual(public['task'], note['task'])
        self.assertEqual('written', self.call(task=public['task'], capsule=public['capsule'] + 'fresh: source\n')['state'])

    def test_migration_cannot_override_tracking_or_a_changed_approved_image(self):
        public = json.loads((ROOT / 'tests/fixtures/memory_cursor_migration_public.json').read_text())
        self.rule.parent.mkdir(parents=True)
        self.rule.write_text(public['legacy'], encoding='utf-8')
        self.rule.write_text(public['legacy'] + 'human change', encoding='utf-8')
        self.assertEqual('preserved', self.call('migrate', approved_sha256=public['approved_sha256'])['state'])
        self.rule.write_text(public['legacy'], encoding='utf-8')
        subprocess.run(['git', '-C', str(self.root), 'add', '-f', str(self.rule)], check=True, capture_output=True)
        self.assertEqual('preserved', self.call('migrate', approved_sha256=public['approved_sha256'])['state'])
        self.assertEqual(public['legacy'], self.rule.read_text(encoding='utf-8'))
        self.assertFalse(self.receipt.exists())


    def test_report_json_enforce_skip_keeps_same_task_and_retires_foreign_task(self):
        import io
        import contextlib
        sys.path.insert(0, str(EDITION / 'memory-bank/scripts'))
        try:
            context = importlib.import_module('context')
        finally:
            sys.path.pop(0)
        raw, receipt = self.owned()
        report = {'capsule': {'gate': {'mode': 'enforce', 'decision': 'skip'}},
                  'capsule_text': 'working: WITHHELD\nsemantic: none\n'}
        def submit(task):
            with io.TextIOWrapper(io.BytesIO(json.dumps(report).encode())) as incoming, \
                    mock.patch.object(sys, 'stdin', incoming), \
                    mock.patch.object(sys, 'argv', ['context.py', '--root', str(self.root),
                        'cursor-rule', '--action', 'write', '--task-id', task, '--report-json']), \
                    contextlib.redirect_stdout(io.StringIO()) as output:
                self.assertEqual(0, context.main())
                return json.loads(output.getvalue())
        self.assertEqual('kept', submit(TASK)['state'])
        self.assertEqual((raw, receipt), (self.rule.read_bytes(), self.receipt.read_bytes()))
        self.assertEqual('removed', submit('OTHER-SKIP')['state'])
        self.assertFalse(self.rule.exists())

    def test_tracked_rule_or_receipt_is_preserved(self):
        old = self.owned()
        for path in (self.rule, self.receipt):
            subprocess.run(['git', '-C', str(self.root), 'add', '-f', '--', str(path)], check=True, capture_output=True)
            self.preserved(old)
            subprocess.run(['git', '-C', str(self.root), 'rm', '--cached', '-q', '--', str(path)], check=True, capture_output=True)

    def test_alternate_git_environment_cannot_hide_tracked_leaf(self):
        old = self.owned()
        with mock.patch.dict(os.environ, {'GIT_INDEX_FILE': str(self.base / 'alternate')}):
            self.preserved(old)
        with mock.patch.object(self.r.subprocess, 'run', side_effect=OSError('not retained')):
            self.preserved(old)

    def test_foreign_root_and_invalid_attachment_never_mutate(self):
        old = self.owned()
        with mock.patch.object(self.r.workspace_roots, 'TOOLING_ROOT', self.base / 'other'):
            self.preserved(old)
        with mock.patch.dict(os.environ, {'ACCELERATOR_HOME': str(self.root)}):
            self.preserved(old)

    def test_valid_attached_layout_leaves_shared_rule(self):
        old = self.owned(); state = self.base / 'state'; state.mkdir()
        project = self.base / 'outside-project'; project.mkdir()
        with mock.patch.dict(os.environ, {'ACCELERATOR_HOME': str(self.root),
                'ACCELERATOR_STATE_DIR': str(state), 'ACCELERATOR_PROJECT_DIR': str(project)}):
            self.assertEqual('attached', self.r.cursor_rule(state, 'retire')['state'])
        self.assertEqual(old, (self.rule.read_bytes(), self.receipt.read_bytes()))

    def test_linked_rule_or_hardlinked_rule_is_preserved(self):
        raw, receipt = self.owned(); self.rule.unlink()
        outside = self.base / 'foreign'; outside.write_bytes(raw)
        self.rule.symlink_to(outside)
        self.assertEqual('preserved', self.call('retire')['state'])
        self.assertEqual(raw, outside.read_bytes()); self.rule.unlink()
        os.link(outside, self.rule)
        self.preserved((raw, receipt)); self.assertEqual(raw, outside.read_bytes())

    def test_linked_parent_or_lock_directory_cannot_redirect_writes(self):
        old = self.owned()
        outside = self.base / 'foreign-directory'; outside.mkdir()
        for parent in (self.rule.parent, self.root / 'project-brain/local'):
            saved = parent.with_name(parent.name + '-saved'); parent.rename(saved)
            parent.symlink_to(outside, target_is_directory=True)
            self.assertEqual('preserved', self.call('retire')['state'])
            self.assertEqual([], list(outside.iterdir()))
            parent.unlink(); saved.rename(parent)
        self.assertEqual(old, (self.rule.read_bytes(), self.receipt.read_bytes()))

    def test_lock_fifo_or_hardlink_is_rejected_without_blocking(self):
        old = self.owned(); lock = self.root / 'project-brain/local/runtime.lock'; lock.unlink()
        os.mkfifo(lock)
        started = time.monotonic(); self.preserved(old)
        self.assertLess(time.monotonic() - started, 1)
        lock.unlink(); other = self.base / 'other-lock'; other.write_text('untouched')
        os.link(other, lock); self.preserved(old)
        self.assertEqual('untouched', other.read_text())

    def test_busy_native_lock_does_not_wait_for_release(self):
        import fcntl
        old = self.owned()
        with (self.root / 'project-brain/local/runtime.lock').open('r+') as held:
            fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
            started = time.monotonic(); self.preserved(old)
            self.assertLess(time.monotonic() - started, 1)

    def test_busy_native_process_registry_does_not_wait(self):
        old = self.owned()
        key = str(self.root / 'project-brain/local/runtime.lock')
        held = self.r.brain_runtime._LOCKS[key]; ready = threading.Event(); release = threading.Event()
        def hold():
            with held:
                ready.set(); release.wait(5)
        thread = threading.Thread(target=hold); thread.start()
        try:
            self.assertTrue(ready.wait(2)); self.preserved(old)
        finally:
            release.set(); thread.join(2)
        self.assertFalse(thread.is_alive())

    def test_partial_receipt_unlink_reports_actual_removal(self):
        raw, receipt = self.owned(); original = self.r.os.unlink
        def unlink(name, **kwargs):
            if name == 'cursor-rule.json':
                raise OSError('arbitrary error never retained')
            return original(name, **kwargs)
        with mock.patch.object(self.r.os, 'unlink', side_effect=unlink):
            # supports_dir_fd is checked before calling the mocked operation.
            supported = set(self.r.os.supports_dir_fd) | {self.r.os.unlink}
            with mock.patch.object(self.r.os, 'supports_dir_fd', supported):
                result = self.call('retire')
        self.assertEqual({'profile': 'cursor-rule-custody-v1', 'state': 'incomplete',
                          'bytes_removed': len(raw)}, result)
        self.assertFalse(self.rule.exists()); self.assertEqual(receipt, self.receipt.read_bytes())

    def test_receipt_publication_failure_leaves_unproven_rule(self):
        original = self.r._cursor_publish
        def publish(parent, name, *args):
            if name == 'cursor-rule.json':
                raise OSError('no raw diagnostic')
            return original(parent, name, *args)
        with mock.patch.object(self.r, '_cursor_publish', side_effect=publish):
            self.assertEqual('incomplete', self.call()['state'])
        raw = self.rule.read_bytes()
        self.assertEqual('preserved', self.call('retire')['state'])
        self.assertEqual(raw, self.rule.read_bytes()); self.assertFalse(self.receipt.exists())
        self.assertEqual(['working-memory.mdc'], sorted(p.name for p in self.rule.parent.iterdir()))

    def test_parent_swap_is_detected_without_outside_mutation(self):
        old = self.owned(); original = self.r._cursor_git_untracked
        saved = self.rule.parent.with_name('held-rules'); outside = self.base / 'outside'; outside.mkdir()
        calls = 0
        def tracked(root):
            nonlocal calls
            calls += 1
            if calls == 2:
                self.rule.parent.rename(saved); self.rule.parent.symlink_to(outside, target_is_directory=True)
            return original(root)
        with mock.patch.object(self.r, '_cursor_git_untracked', side_effect=tracked):
            self.assertEqual('preserved', self.call('retire')['state'])
        self.assertEqual([], list(outside.iterdir()))
        self.assertEqual(old[0], (saved / 'working-memory.mdc').read_bytes())

    def test_failed_admission_keeps_owned_current_and_retires_owned_foreign(self):
        for capsule in ('not a capsule', 'working:' + 'é' * 9000, 'working:\ud800'):
            self.owned()
            self.assertEqual('kept', self.call(capsule=capsule)['state'])
            self.assertEqual('removed', self.call(task='OTHER-BRANCH', capsule=capsule)['state'])
        self.owned(); self.assertEqual('removed', self.call(task='', capsule='')['state'])

    def test_prompt_phase_survives_stop_but_session_can_refresh(self):
        old = self.owned()
        self.assertEqual('kept', self.call(phase='turn', capsule=CHANGED)['state'])
        self.assertEqual(old, (self.rule.read_bytes(), self.receipt.read_bytes()))
        self.assertEqual('written', self.call(phase='session', capsule=CHANGED)['state'])

    def test_publication_bound_and_failed_write_leave_no_temporary(self):
        self.rule.parent.mkdir(parents=True)
        descriptor = os.open(self.rule.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            with self.assertRaises(OSError):
                self.r._cursor_publish(descriptor, 'working-memory.mdc', b'X' * 10, None, 9)
            with mock.patch.object(self.r.os, 'write', side_effect=OSError('refused')):
                with self.assertRaises(OSError):
                    self.r._cursor_publish(descriptor, 'working-memory.mdc', b'X', None, 9)
        finally:
            os.close(descriptor)
        self.assertEqual([], list(self.rule.parent.iterdir()))

    def test_copied_cli_handles_recursive_report_before_database_and_rejects_foreign_root(self):
        cli = install(self.root)
        command = [sys.executable, '-B', str(cli), 'cursor-rule', '--action', 'write', '--task-id', TASK, '--report-json']
        result = subprocess.run(command, input=b'[' * 1500 + b'0' + b']' * 1500,
                                capture_output=True, timeout=5)
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual('absent', json.loads(result.stdout)['state'])
        self.assertFalse((self.root / 'memory-bank/local/context.db').exists())
        foreign = self.base / 'foreign-root'; foreign.mkdir()
        result = subprocess.run([sys.executable, '-B', str(cli), '--root', str(foreign),
            'cursor-rule', '--action', 'write', '--task-id', TASK], input=CAPSULE.encode(), capture_output=True, timeout=5)
        self.assertEqual('preserved', json.loads(result.stdout)['state'])
        self.assertEqual([], list(foreign.iterdir()))


@unittest.skipUnless(os.name == 'posix' and shutil.which('bash'), 'Cursor shell profile')
class CursorRuleHookTests(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory(prefix='iris-cursor-hook-')
        self.addCleanup(folder.cleanup); self.root = Path(folder.name) / 'project'; self.root.mkdir()
        install(self.root)
        hooks = self.root / '.cursor/hooks'; hooks.mkdir(parents=True)
        for name in ('working-memory-read.sh', 'working-memory-write.sh', 'local-context.sh'):
            shutil.copy2(EDITION / '.cursor/hooks' / name, hooks / name)
        self.rule = self.root / '.cursor/rules/working-memory.mdc'
        self.cli = self.root / 'memory-bank/scripts/context.py'
        seeded = self.maintenance('write')
        self.assertEqual('written', json.loads(seeded.stdout)['state'], seeded.stderr)
        self.calls = self.root / 'actions.jsonl'
        stub(self.root, 'import json\n' + f"with open({str(self.calls)!r}, 'a') as handle: handle.write(sys.argv[1]+'\\n')\n" +
             f"if sys.argv[1] == 'refresh': print(json.dumps({{'capsule_text': CAPSULE}}))\n" +
             f"elif sys.argv[1] == 'hook-context': print({CAPSULE!r})\n")

    def maintenance(self, action):
        return subprocess.run([sys.executable, '-B', str(self.cli), 'cursor-rule', '--action', action,
            '--task-id', TASK], input=CAPSULE, capture_output=True, text=True, timeout=5,
            env=clean_environment())

    def hook(self, name='working-memory-read.sh', **env):
        environment = {k: v for k, v in os.environ.items() if not k.startswith(('GIT_', 'ACCELERATOR_', 'CONTEXT_'))}
        environment.update({'CONTEXT_TASK_ID': TASK, **env})
        return subprocess.run(['bash', str(self.root / '.cursor/hooks' / name)],
            input=json.dumps({'prompt': 'Check the Iris owner', 'conversation_id': 'iris-chat',
                              'cursor_version': 'fixture', 'hook_event_name': 'beforeSubmitPrompt'}),
            env=environment, cwd=self.root, capture_output=True, text=True, timeout=8)

    def test_host_delivery_retires_owned_rule_and_acknowledges_once_without_refresh(self):
        self.assertTrue(self.rule.exists())
        result = self.hook(CONTEXT_CAPSULE_DELIVERED='1')
        self.assertEqual((0, '{"continue": true}\n', ''), (result.returncode, result.stdout, result.stderr))
        self.assertFalse(self.rule.exists()); self.assertFalse(self.calls.exists())

    def test_host_delivery_preserves_custom_rule_and_recovery_leaves_owned_rule(self):
        raw = self.rule.read_bytes()
        result = self.hook(CONTEXT_CAPSULE_DELIVERED='1', CONTEXT_MEMORY_RECOVERY='1')
        self.assertEqual('{"continue": true}\n', result.stdout); self.assertEqual(raw, self.rule.read_bytes())
        self.rule.write_bytes(raw + b' custom edit')
        result = self.hook(CONTEXT_CAPSULE_DELIVERED='1')
        self.assertEqual('{"continue": true}\n', result.stdout)
        self.assertEqual(raw + b' custom edit', self.rule.read_bytes())

    def test_busy_lock_prompt_returns_without_waiting_and_rule_remains(self):
        import fcntl
        raw = self.rule.read_bytes()
        with (self.root / 'project-brain/local/runtime.lock').open('r+') as handle:
            fcntl.flock(handle, fcntl.LOCK_EX)
            started = time.monotonic(); result = self.hook(CONTEXT_CAPSULE_DELIVERED='1')
            self.assertLess(time.monotonic() - started, 3)
        self.assertEqual('{"continue": true}\n', result.stdout); self.assertEqual(raw, self.rule.read_bytes())

    def test_stop_checkpoints_before_retirement(self):
        result = self.hook('working-memory-write.sh', CONTEXT_CAPSULE_DELIVERED='1')
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual(['turn'], self.calls.read_text().splitlines())
        self.assertFalse(self.rule.exists())


class CursorRuleMetadataTests(unittest.TestCase):
    def context(self, response):
        from types import SimpleNamespace
        sys.path.insert(0, str(ROOT / 'harness/src'))
        try:
            from harness.task_context import TaskContext
        finally:
            sys.path.pop(0)
        knowledge = SimpleNamespace(sessions=SimpleNamespace(_workspace=lambda session: '/owned-fixture'),
                                    run=mock.Mock(return_value=response))
        return TaskContext(knowledge, wait=0), knowledge

    def test_native_result_count_and_state_admission_are_bounded(self):
        for state, count in (('removed', 0), ('removed', 16385), ('incomplete', 16385),
                             ('preserved', 1), ('absent', 1), ('attached', 1), ('removed', True)):
            with self.subTest(state=state, count=count):
                context, _ = self.context({'ok': True, 'result': {'profile': 'cursor-rule-custody-v1',
                                                               'state': state, 'bytes_removed': count}})
                self.assertEqual({'state': 'unavailable', 'bytes_removed': None},
                    context.retire_cursor_rule({'project_id': 'iris', 'brain': {'bank': 'memory-bank'}}))
        for state, count in (('removed', 123), ('incomplete', 123), ('preserved', 0), ('absent', 0), ('attached', 0)):
            context, _ = self.context({'ok': True, 'result': {'profile': 'cursor-rule-custody-v1',
                                                           'state': state, 'bytes_removed': count}})
            self.assertEqual({'state': state, 'bytes_removed': count},
                context.retire_cursor_rule({'project_id': 'iris', 'brain': {'bank': 'memory-bank'}}))

    def test_unavailable_runtime_and_guarded_scope_do_not_leak_error_or_body(self):
        context, knowledge = self.context({'ok': False, 'error': 'private raw error', 'result': {'body': 'private rule'}})
        context.guard = lambda: None
        self.assertEqual({'state': 'unavailable', 'bytes_removed': None},
            context.retire_cursor_rule({'project_id': 'iris', 'brain': {'bank': 'memory-bank'}}))
        args, options = knowledge.run.call_args
        self.assertEqual(('iris', {'action': 'cursor-rule-retire', 'bank': 'memory-bank'}), args)
        self.assertEqual('/owned-fixture', options['_root'])
        self.assertIs(context.guard, options['_guard'])


if __name__ == '__main__':
    unittest.main()
