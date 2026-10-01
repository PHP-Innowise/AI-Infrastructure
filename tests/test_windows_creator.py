"""Creator sandbox contracts plus opt-in, model-free native Windows acceptance."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
import importlib.util
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'harness/src'))
from harness import creator, creator_runner, windows_creator
from harness import process_runtime


class WindowsCreatorContracts(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(); self.addCleanup(temp.cleanup)
        self.root = Path(temp.name).resolve()
        self.work = self.root / 'control/agent'; self.work.mkdir(parents=True)
        self.target = self.root / 'target'; self.target.mkdir()
        self.home = self.root / 'account'; self.home.mkdir()
        self.source = self.root / 'source'; self.source.mkdir()

    def command(self, **kwargs):
        with patch.object(windows_creator, 'command_argv', side_effect=lambda argv, *args: argv):
            return windows_creator.command('codex.exe', self.work, self.target,
                [sys.executable, '-c', 'print("argv with spaces")'],
                homes=kwargs.pop('homes', [self.home]), source=self.source, **kwargs)

    def test_enforced_random_profile_and_arbitrary_argv(self):
        argv = self.command()
        self.assertEqual(argv[:2], ['codex.exe', 'sandbox'])
        self.assertIn('windows.sandbox="elevated"', argv)
        self.assertIn('--include-managed-config', argv)
        name = argv[argv.index('-P') + 1]
        self.assertNotEqual(name, self.command()[self.command().index('-P') + 1])
        self.assertEqual(argv[argv.index('-C') + 1], str(self.work))
        profile = next(item for item in argv if item.startswith('permissions='))
        for path in (':root', self.target, self.work.parent, self.source):
            self.assertIn(json.dumps(str(path)) + '="read"', profile)
        for path in (self.work, self.home):
            self.assertIn(json.dumps(str(path)) + '="write"', profile)
        self.assertNotIn('danger-full-access', ' '.join(argv))
        self.assertEqual(argv[-3:], [sys.executable, '-c', 'print("argv with spaces")'])
        self.assertTrue((self.work / '.harness-tmp').is_dir())
        self.assertFalse(list(self.work.parent.glob('*.toml')))

    def test_rejects_all_provider_and_protected_root_overlaps(self):
        nested = self.target / 'profile'; nested.mkdir()
        for home in (self.target, nested, self.root, self.work.parent, self.source):
            with self.subTest(home=home), self.assertRaisesRegex(ValueError, 'overlaps'):
                self.command(homes=[home])
        original = self.home / 'original-project'; original.mkdir()
        with self.assertRaisesRegex(ValueError, 'overlaps'):
            self.command(protected_roots=[original])

    def test_backend_uses_configured_cli_and_rejects_missing_capability(self):
        executable = self.root / 'codex.exe'; executable.write_bytes(b'fixture')
        windows_creator._capability.cache_clear()
        with patch.object(windows_creator, 'command_argv', side_effect=lambda argv, *args: argv), \
             patch.object(windows_creator.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0,
                 b'--permission-profile --cd --include-managed-config')) as run:
            self.assertEqual(windows_creator.backend(str(executable)), ('windows-codex', str(executable)))
            windows_creator.backend(str(executable))
            self.assertEqual(run.call_count, 1)
            self.assertEqual(run.call_args.args[0][0], str(executable))
        windows_creator._capability.cache_clear()
        with patch.object(windows_creator, 'command_argv', side_effect=lambda argv, *args: argv), \
             patch.object(windows_creator.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, b'old CLI')):
            self.assertIsNone(windows_creator.backend(str(executable)))
        windows_creator._capability.cache_clear()

    def test_copy_preserves_accelerator_inputs_omits_app_and_caches(self):
        contents = {'AGENTS.md': b'policy', '.infra-manifest.json': b'{}',
            'memory-bank/scripts/context.py': b'print("context")',
            'project-brain/records/task.md': b'knowledge', '.agents/hooks/check.sh': b'#!/bin/bash\n',
            'memory-bank/local/context.db': b'original SQLite',
            'project-brain/local/lock': b'original lock', 'app/private.php': b'private app'}
        for name, value in contents.items():
            path = self.target / name; path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(value)
        snapshot = windows_creator.validation_copy(self.target, self.work)
        try:
            for name, value in contents.items():
                copied = snapshot / name
                if '/local/' in name or name.startswith('app/'):
                    self.assertFalse(copied.exists())
                else:
                    self.assertEqual(copied.read_bytes(), value)
                self.assertEqual((self.target / name).read_bytes(), value)
        finally:
            shutil.rmtree(snapshot)

    def test_copy_rejects_hardlinks_and_removes_partial_tree(self):
        path = self.target / 'AGENTS.md'; path.write_text('policy')
        os.link(path, self.target / 'CLAUDE.md')
        with self.assertRaises(creator.SessionError):
            windows_creator.validation_copy(self.target, self.work)
        self.assertFalse(list(self.work.glob('.harness-validation-*')))

    def test_copy_removes_partial_tree_when_target_open_fails(self):
        with self.assertRaises(OSError):
            windows_creator.validation_copy(self.target / 'missing', self.work)
        self.assertFalse(list(self.work.glob('.harness-validation-*')))

    def test_portable_publication_names_rejected_by_ownership_and_plan(self):
        pub = creator.publication_helpers()
        task = self.work / creator.TASK; task.mkdir(parents=True)
        invalid = ['memory-bank/file:stream', 'AGENTS.md:stream', 'x/CON', 'x/con.txt',
                   'x/CONIN$', 'x/CONOUT$', 'x/CON .txt', 'x/LPT¹', 'x/name.', 'x/name ', 'C:evil',
                   'a\\b', '../outside', '/absolute', './AGENTS.md', 'x//file', 'x/\x01']
        for name in invalid:
            with self.subTest(name=name), self.assertRaises(ValueError):
                pub.normalize_relative_path(name)
            (task / creator.PLAN_NAMES[0]).write_text(name + '\n', encoding='utf-8')
            with self.subTest(plan=name), self.assertRaises(creator.SessionError):
                creator.read_plan(task, creator.PLAN_NAMES[0])
        self.assertEqual(pub.normalize_relative_path('memory-bank/chunks/valid.md'),
                         'memory-bank/chunks/valid.md')

    def test_plan_rejects_non_utf8_bytes_without_exposing_contents(self):
        task = self.work / creator.TASK; task.mkdir(parents=True)
        for body in (b'x/LPT\xb9\n', b'PRIVATE PLAN SENTINEL\xff'):
            with self.subTest(body=body):
                (task / creator.PLAN_NAMES[0]).write_bytes(body)
                with self.assertRaisesRegex(creator.SessionError, 'must use UTF-8 text') as error:
                    creator.read_plan(task, creator.PLAN_NAMES[0])
                self.assertNotIn('PRIVATE PLAN SENTINEL', str(error.exception))

    def validator(self):
        creator.publication_helpers()
        spec = importlib.util.spec_from_file_location('windows_generated_validator', creator.SCRIPTS/'validate_generated.py')
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        return module

    def test_manifest_rejects_windows_device_and_stream_paths_before_file_access(self):
        validator = self.validator()
        for name in ('memory-bank/file:stream', 'memory-bank/CONIN$', 'x/CON .txt', 'x/name '):
            (self.target / '.infra-manifest.json').write_text(json.dumps({
                'generator_version': '1.0.0', 'task': 'TASK-001', 'mode': 'full', 'files': {name: '0'*64}}))
            errors = []; validator.validate_manifest(self.target, errors)
            self.assertTrue(any('invalid target-relative file path' in error for error in errors), errors)

    def test_hook_wiring_keeps_unix_execute_bit_gate_only_where_supported(self):
        validator = self.validator()
        hook = self.target / '.claude/hooks/check.sh'; hook.parent.mkdir(parents=True)
        hook.write_text('#!/bin/bash\nexit 0\n'); hook.chmod(0o600)
        wiring = self.target / '.claude/settings.json'
        wiring.write_text(json.dumps({'hooks': {'PreToolUse': [{'command': 'bash .claude/hooks/check.sh'}]}}))
        errors = []
        validator.validate_hook_wiring(self.target, ['claude'],
            {'.claude/settings.json': '0'*64, '.claude/hooks/check.sh': '0'*64}, errors)
        self.assertEqual(any('not executable' in error for error in errors), os.name != 'nt', errors)

    def test_trampoline_preserves_stdin_stdout_and_sets_private_temp(self):
        child = Path(windows_creator.__file__).with_name('windows_creator_child.py')
        result = subprocess.run([sys.executable, str(child), str(self.work), sys.executable, '-c',
            'import os,sys,json; print(json.dumps([sys.stdin.read(),os.environ["TEMP"],os.environ["TMP"]]))'],
            input='native input', text=True, capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout), ['native input', str(self.work), str(self.work)])

    def test_trampoline_preserves_binary_line_endings(self):
        child = Path(windows_creator.__file__).with_name('windows_creator_child.py')
        body = b'native input\r\nsecond line\n\x00\xff'
        result = subprocess.run([sys.executable, str(child), str(self.work), sys.executable, '-c',
            'import sys; sys.stdout.buffer.write(sys.stdin.buffer.read())'],
            input=body, capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, body)

    def test_installed_validation_copy_is_removed_when_runtime_validation_fails(self):
        task = self.work / creator.TASK; task.mkdir(parents=True)
        for name in creator.REPORTS[:3]:
            (task / name).write_text('{}')
        (self.target / 'memory-bank/local').mkdir(parents=True)
        original = self.target / 'memory-bank/local/context.db'; original.write_text('original')
        request = {'directory': str(self.work.parent), 'target': str(self.target), 'tools': ['codex'],
                   'approved': creator.profile_hashes(self.work.parent)}
        copies = []
        def failing_check(argv, cwd):
            if Path(argv[2]).name == 'validate_generated.py':
                copy = Path(argv[argv.index('--target') + 1]); copies.append(copy)
                self.assertNotEqual(copy, self.target)
                self.assertTrue(copy.is_dir())
                self.assertEqual(argv[argv.index('--evidence-target') + 1], str(self.target))
                (copy / 'memory-bank/local').mkdir()
                (copy / 'memory-bank/local/context.db').write_text('disposable')
                raise creator.SessionError('runtime failed')
        with patch.object(process_runtime, 'WINDOWS', True), \
             patch.object(creator_runner, 'sandbox_command', side_effect=lambda work,target,argv,**kw: argv), \
             patch.object(creator_runner, 'run_process', side_effect=failing_check):
            with self.assertRaisesRegex(creator.SessionError, 'runtime failed'):
                creator_runner.verify(request, generated=True, installed=True)
        self.assertEqual(len(copies), 1)
        self.assertFalse(copies[0].exists())
        self.assertEqual(original.read_text(), 'original')

    def test_windows_codex_relies_on_outer_boundary_and_never_runs_if_probe_fails(self):
        task = self.work / creator.TASK; task.mkdir(parents=True)
        fd = creator._root_fd(self.target)
        try:
            identity = list(creator._identity(windows_creator.fs.fstat(fd)))
        finally:
            windows_creator.fs.close(fd)
        request = {'directory': str(self.work.parent), 'target': str(self.target), 'identity': identity,
            'phase': 'scan', 'provider': 'codex', 'executable': 'codex.exe', 'tools': ['codex'],
            'operation': 'generate', 'model': None, 'thinking_effort': None,
            'agents_enabled': False, 'agent_count': 1, 'goal': '', 'answers': ''}
        launches = []
        def sandbox(work, target, argv, provider=None, **kwargs):
            launches.append(argv)
            return ['verified-outer-sandbox', *argv]
        with patch.object(process_runtime, 'WINDOWS', True), \
             patch.object(creator_runner, 'sandbox_command', side_effect=sandbox), \
             patch.object(creator_runner, 'verify'), patch.object(creator_runner, 'run_process'):
            self.assertEqual(creator_runner.execute(request), 'review')
        self.assertIn('windows_creator_probe.py', launches[0][2])
        self.assertEqual(launches[0][3], str(self.work))
        self.assertNotIn('--sandbox', launches[1])
        self.assertIn('default_permissions=":danger-full-access"', launches[1])
        launches.clear()
        with patch.object(process_runtime, 'WINDOWS', True), \
             patch.object(creator_runner, 'sandbox_command', side_effect=sandbox), \
             patch.object(creator_runner, 'run_process', side_effect=creator.SessionError('boundary failed')):
            with self.assertRaisesRegex(creator.SessionError, 'boundary failed'):
                creator_runner.execute(request)
        self.assertEqual(len(launches), 1, 'the native agent must not start after a failed preflight')

    @unittest.skipUnless(os.name == 'nt', 'native Windows Git path casing')
    def test_native_git_details_accepts_mixed_case_project_and_worktree(self):
        from harness.sessions import git_details
        git = shutil.which('git'); self.assertIsNotNone(git)
        subprocess.run([git, 'init', str(self.target)], check=True, capture_output=True)
        for key, value in (('user.name', 'Harness fixture'), ('user.email', 'fixture@example.invalid')):
            subprocess.run([git, '-C', str(self.target), 'config', key, value], check=True, capture_output=True)
        (self.target/'composer.json').write_text('{}')
        subprocess.run([git, '-C', str(self.target), 'add', 'composer.json'], check=True, capture_output=True)
        subprocess.run([git, '-C', str(self.target), 'commit', '-m', 'fixture'], check=True, capture_output=True)
        details = git_details(str(self.target).swapcase())
        self.assertTrue(details['worktree_available'])
        worktree = self.root / 'Git Worktree'
        subprocess.run([git, '-C', str(self.target), 'worktree', 'add', '--detach', str(worktree)],
                       check=True, capture_output=True)
        self.assertTrue(git_details(str(worktree).swapcase())['worktree_available'])


@unittest.skipUnless(os.name == 'nt' and os.environ.get('HARNESS_WINDOWS_CREATOR_SANDBOX') == '1',
                     'requires a configured native Windows elevated Codex sandbox')
class WindowsCreatorNative(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(); self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.work = self.root / 'control/agent'; self.work.mkdir(parents=True)
        self.target = self.root / 'project'; self.target.mkdir()
        self.codex = os.environ.get('HARNESS_CODEX_BIN') or shutil.which('codex')
        self.assertIsNotNone(windows_creator.backend(self.codex), windows_creator.REQUIRED)

    def test_real_boundary_read_write_descendants_and_runtime_copy(self):
        (self.target / 'composer.json').write_text('{}')
        (self.target / 'memory-bank/local').mkdir(parents=True)
        (self.target / 'memory-bank/local/context.db').write_text('original')
        (self.target / 'memory-bank/scripts').mkdir()
        (self.target / 'memory-bank/scripts/context.py').write_text('print("runtime")')
        probe = Path(windows_creator.__file__).with_name('windows_creator_probe.py')
        command = windows_creator.command(self.codex, self.work, self.target,
            [sys.executable, str(probe), str(self.work), str(self.target), str(self.work.parent)])
        result = subprocess.run(command, cwd=self.work, input='', capture_output=True, text=True, timeout=180)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        fixture = self.root / 'fixture.py'
        fixture.write_text('''import json,os,pathlib,subprocess,sys
work,target,control=map(pathlib.Path,sys.argv[1:])
assert (target/'composer.json').read_text()=='{}'
(work/'allowed').write_text('yes')
for root in (target,control):
 try: (root/'denied').write_text('bad')
 except PermissionError: pass
 else: raise RuntimeError('unconfined write')
child=subprocess.run([sys.executable,'-c',"from pathlib import Path; import sys; Path(sys.argv[1]).write_text('bad')",str(target/'child-denied')],capture_output=True)
assert child.returncode!=0
try: os.replace(work/'allowed', target/'renamed')
except PermissionError: pass
else: raise RuntimeError('unconfined rename')
print('native boundaries passed')
''')
        result = subprocess.run(windows_creator.command(self.codex, self.work, self.target,
            [sys.executable, str(fixture), str(self.work), str(self.target), str(self.work.parent)]),
            cwd=self.work, capture_output=True, text=True, timeout=180)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue((self.work/'allowed').exists())
        self.assertFalse((self.target/'denied').exists())
        copy = windows_creator.validation_copy(self.target, self.work)
        try:
            result = subprocess.run(windows_creator.command(self.codex, self.work, self.target,
                [sys.executable, '-c', 'from pathlib import Path; p=Path(__import__("sys").argv[1])/"memory-bank/local"; p.mkdir(); (p/"context.db").write_text("copy")', str(copy)]),
                cwd=self.work, capture_output=True, text=True, timeout=180)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual((self.target/'memory-bank/local/context.db').read_text(), 'original')
        finally:
            shutil.rmtree(copy)

    def test_git_common_directory_acl_and_existing_descendants_are_read_only(self):
        git = shutil.which('git'); self.assertIsNotNone(git)
        subprocess.run([git, 'init', str(self.target)], check=True, capture_output=True)
        for key, value in (('user.name', 'Harness fixture'), ('user.email', 'fixture@example.invalid')):
            subprocess.run([git, '-C', str(self.target), 'config', key, value], check=True, capture_output=True)
        (self.target / 'composer.json').write_text('{}')
        nested = self.target / 'existing/child'; nested.mkdir(parents=True)
        (nested / 'sentinel').write_text('original')
        subprocess.run([git, '-C', str(self.target), 'add', '.'], check=True, capture_output=True)
        subprocess.run([git, '-C', str(self.target), 'commit', '-m', 'fixture'], check=True, capture_output=True)
        worktree = self.root / 'linked-project'
        subprocess.run([git, '-C', str(self.target), 'worktree', 'add', '--detach', str(worktree)],
                       check=True, capture_output=True)
        common = self.target / '.git'
        probe = Path(windows_creator.__file__).with_name('windows_creator_probe.py')
        result = subprocess.run(creator.sandbox_command(self.work, worktree,
            [sys.executable, str(probe), str(self.work), str(worktree), str(common), str(self.work.parent)],
            sandbox_executable=self.codex, protected_roots=[self.target]),
            cwd=self.work, capture_output=True, text=True, timeout=180)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((worktree / 'existing/child/sentinel').read_text(), 'original')

    def test_owner_death_terminates_elevated_descendants_before_restart_admission(self):
        from harness.filesystem import fs
        lock = fs.open_lock(self.root / 'runner.lock')
        reader, writer = os.pipe()
        marker = self.work / 'processes.json'
        fixture = self.root / 'long-child.py'
        fixture.write_text('''import json,os,subprocess,sys,time
child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(300)'])
open(sys.argv[1],'w').write(json.dumps([os.getpid(),child.pid]))
time.sleep(300)
''')
        command = windows_creator.command(self.codex, self.work, self.target,
            [sys.executable, str(fixture), str(marker)])
        guard = process_runtime.launch_guarded(command, reader, lock_fd=lock, cwd=self.work,
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        os.close(reader)
        try:
            deadline = time.monotonic() + 120
            while not marker.exists() and time.monotonic() < deadline:
                self.assertIsNone(guard.poll(), 'elevated sandbox failed before child start')
                time.sleep(.1)
            self.assertTrue(marker.exists(), 'sandbox child did not start')
            pids = json.loads(marker.read_text())
            fs.close(lock); lock = None
            os.close(writer); writer = None
            guard.wait(timeout=20)
            # A replacement can only be admitted after the guard has exited.
            replacement = fs.open_lock(self.root / 'runner.lock')
            fs.close(replacement)
            import ctypes
            from ctypes import wintypes as w
            kernel = ctypes.WinDLL('kernel32', use_last_error=True)
            kernel.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
            kernel.OpenProcess.restype = w.HANDLE
            kernel.WaitForSingleObject.argtypes = [w.HANDLE, w.DWORD]
            kernel.WaitForSingleObject.restype = w.DWORD
            kernel.CloseHandle.argtypes = [w.HANDLE]
            for pid in pids:
                handle = kernel.OpenProcess(0x100000, False, pid)
                if handle:
                    try:
                        self.assertEqual(kernel.WaitForSingleObject(handle, 0), 0,
                            'a sandbox child survived restart admission')
                    finally:
                        kernel.CloseHandle(handle)
        finally:
            if writer is not None: os.close(writer)
            if lock is not None: fs.close(lock)
            if guard.poll() is None: process_runtime.reap_tree(guard)


if __name__ == '__main__':
    unittest.main()
