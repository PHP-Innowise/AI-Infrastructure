"""New Kestrel strict configuration observations; no historical case replay."""
from __future__ import annotations

import errno
import importlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
PUBLIC = json.loads((ROOT / 'tests/fixtures/memory_config_observation_public.json').read_text(encoding='utf-8'))
REPARSE = json.loads((ROOT / 'tests/fixtures/memory_config_reparse_public.json').read_text(encoding='utf-8'))


class ConfigObservationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='kestrel-config-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.config = self.root / 'project-brain/config/runtime.json'
        directory = Path(os.environ.get('KESTREL_ORIGINAL_RUNTIME', str(ROOT / 'PHP Core/memory-bank/scripts')))
        names = [item.stem for item in directory.glob('*.py')]
        previous = {name: sys.modules.pop(name) for name in names if name in sys.modules}
        def restore():
            for name in names:
                sys.modules.pop(name, None)
            sys.modules.update(previous)
        self.addCleanup(restore)
        sys.path.insert(0, str(directory))
        try:
            self.brain = importlib.import_module('brain_runtime')
            self.retrieval = importlib.import_module('context_retrieval')
        finally:
            sys.path.remove(str(directory))

    def write(self, raw=None):
        self.config.parent.mkdir(parents=True, exist_ok=True)
        self.config.write_bytes((PUBLIC['valid'][0]['raw'] if raw is None else raw).encode('utf-8'))

    def refused(self, function):
        with self.assertRaises(self.brain.BrainError) as caught:
            function(self.root)
        self.assertEqual(PUBLIC['required_error'] + str(self.config), str(caught.exception))

    def test_complete_optional_defaults_and_absent_fingerprint(self):
        for layout in PUBLIC['missing_layouts']:
            with self.subTest(layout=layout):
                with tempfile.TemporaryDirectory(prefix='kestrel-missing-') as temporary:
                    root = Path(temporary)
                    if layout == 'leaf':
                        (root / 'project-brain/config').mkdir(parents=True)
                    elif layout == 'config-parent':
                        (root / 'project-brain').mkdir()
                    elif layout == 'project-root':
                        root = root / 'not-created'
                    self.assertEqual(PUBLIC['defaults'], self.brain.load_config(root))
                    self.assertEqual('absent', self.retrieval.config_fingerprint(root))

    def test_complete_object_overlay_and_all_newline_fingerprints(self):
        for row in PUBLIC['valid']:
            with self.subTest(name=row['name']):
                self.write(row['raw'])
                self.assertEqual(row['normalized'], self.brain.read_runtime_config_text(self.root))
                self.assertEqual(row['expected'], self.brain.load_config(self.root))
                self.assertEqual(row['sha256'], self.retrieval.config_fingerprint(self.root))

    def test_readable_leaf_and_parent_links_preserve_whole_values(self):
        self.write()
        target = self.root / 'public-runtime.json'
        self.config.replace(target)
        try:
            self.config.symlink_to(target)
        except OSError as error:
            self.skipTest('native symlink creation unavailable: ' + type(error).__name__)
        row = PUBLIC['valid'][0]
        self.assertEqual(row['expected'], self.brain.load_config(self.root))
        self.assertEqual(row['sha256'], self.retrieval.config_fingerprint(self.root))
        self.config.unlink()
        directory = self.root / 'public-config'
        self.config.parent.replace(directory)
        (directory / 'runtime.json').write_bytes(row['raw'].encode('utf-8'))
        self.config.parent.symlink_to(directory, target_is_directory=True)
        self.assertEqual(row['expected'], self.brain.load_config(self.root))
        self.assertEqual(row['sha256'], self.retrieval.config_fingerprint(self.root))

    def test_directory_does_not_enable_defaults(self):
        self.config.mkdir(parents=True)
        self.refused(self.brain.load_config)

    def test_directory_is_not_an_absent_fingerprint(self):
        self.config.mkdir(parents=True)
        self.refused(self.retrieval.config_fingerprint)

    @unittest.skipUnless(hasattr(os, 'mkfifo') and hasattr(os, 'O_NONBLOCK'), 'POSIX FIFO profile only')
    def test_fifo_refuses_before_open(self):
        self.config.parent.mkdir(parents=True)
        os.mkfifo(self.config)
        with mock.patch.object(self.brain.os, 'open', side_effect=AssertionError('special file must not open')):
            self.refused(self.brain.load_config)
            self.refused(self.retrieval.config_fingerprint)

    def test_dangling_leaf_and_parent_links_do_not_enable_defaults(self):
        self.config.parent.mkdir(parents=True)
        try:
            self.config.symlink_to(self.root / 'missing-public-runtime.json')
        except OSError as error:
            self.skipTest('native symlink creation unavailable: ' + type(error).__name__)
        self.refused(self.brain.load_config)
        self.refused(self.retrieval.config_fingerprint)
        self.config.unlink()
        self.config.parent.rmdir()
        self.config.parent.symlink_to(self.root / 'missing-public-directory', target_is_directory=True)
        self.refused(self.brain.load_config)
        self.refused(self.retrieval.config_fingerprint)

    def test_invalid_utf8_is_fixed_configuration_error(self):
        self.config.parent.mkdir(parents=True)
        self.config.write_bytes(bytes.fromhex(PUBLIC['invalid_utf8_hex']))
        self.refused(self.brain.load_config)
        self.refused(self.retrieval.config_fingerprint)

    def test_known_reparse_parent_must_resolve_before_absence(self):
        original_lstat, original_stat = Path.lstat, Path.stat
        for row in REPARSE['cases']:
            with self.subTest(name=row['name']):
                def observed(item, *args, **kwargs):
                    if item == self.config:
                        raise FileNotFoundError(errno.ENOENT, 'public missing leaf')
                    if item == self.config.parent:
                        return SimpleNamespace(st_mode=stat.S_IFDIR | 0o755, st_reparse_tag=row['tag'])
                    return original_lstat(item, *args, **kwargs)
                def target(item, *args, **kwargs):
                    if item == self.config.parent:
                        if row['target'] == 'missing':
                            raise FileNotFoundError(errno.ENOENT, 'public missing link target')
                        return SimpleNamespace(st_mode=stat.S_IFDIR | 0o755)
                    return original_stat(item, *args, **kwargs)
                with mock.patch.object(Path, 'lstat', new=observed), mock.patch.object(Path, 'stat', new=target):
                    if row['expected'] == 'BrainError':
                        self.refused(self.brain.load_config)
                        self.refused(self.retrieval.config_fingerprint)
                    else:
                        self.assertEqual(PUBLIC['defaults'], self.brain.load_config(self.root))
                        self.assertEqual(REPARSE['expected_missing_leaf'], self.retrieval.config_fingerprint(self.root))

    def test_nonobjects_and_syntax_errors_keep_text_fingerprint(self):
        for row in PUBLIC['invalid']:
            with self.subTest(name=row['name']):
                self.write(row['raw'])
                with self.assertRaises(self.brain.BrainError):
                    self.brain.load_config(self.root)
                self.assertEqual(row['fingerprint'], self.retrieval.config_fingerprint(self.root))

    def test_chunk_boundary_newlines_and_empty_text(self):
        row = PUBLIC['supplement']
        self.write(row['boundary_raw'])
        self.assertEqual(row['boundary_normalized'], self.brain.read_runtime_config_text(self.root))
        self.write(row['empty_text'])
        self.assertEqual(row['empty_fingerprint'], self.retrieval.config_fingerprint(self.root))
        with self.assertRaises(self.brain.BrainError):
            self.brain.load_config(self.root)

    def test_permission_error_from_open_never_becomes_absence(self):
        self.write()
        with mock.patch.object(self.brain.os, 'open', side_effect=PermissionError(errno.EACCES, 'public refusal')):
            self.refused(self.brain.load_config)
            self.refused(self.retrieval.config_fingerprint)

    @unittest.skipUnless(os.name == 'posix' and getattr(os, 'geteuid', lambda: 0)() != 0, 'actual non-root POSIX permission profile only')
    def test_actual_unreadable_regular_configuration(self):
        self.write()
        self.config.chmod(0)
        self.addCleanup(self.config.chmod, 0o600)
        self.refused(self.brain.load_config)
        self.refused(self.retrieval.config_fingerprint)

    def test_observed_file_disappearance_does_not_enable_defaults(self):
        self.write()
        original = os.open
        def vanished(*args, **kwargs):
            self.config.unlink()
            return original(*args, **kwargs)
        with mock.patch.object(self.brain.os, 'open', side_effect=vanished):
            self.refused(self.brain.load_config)

    @unittest.skipUnless(hasattr(os, 'mkfifo') and hasattr(os, 'O_NONBLOCK'), 'POSIX FIFO replacement profile only')
    def test_regular_to_fifo_replacement_is_nonblocking_and_closed(self):
        self.write()
        original = os.open
        opened = []
        def swapped(filename, flags, *args, **kwargs):
            self.assertTrue(flags & os.O_NONBLOCK)
            self.config.unlink()
            os.mkfifo(self.config)
            descriptor = original(filename, flags, *args, **kwargs)
            opened.append(descriptor)
            return descriptor
        with mock.patch.object(self.brain.os, 'open', side_effect=swapped):
            self.refused(self.brain.load_config)
        self.assertEqual(1, len(opened))
        with self.assertRaises(OSError) as caught:
            os.fstat(opened[0])
        self.assertEqual(errno.EBADF, caught.exception.errno)

    def test_descriptor_cleanup_and_cancellation_precedence(self):
        self.write()
        original_open, original_close = os.open, os.close
        for row in PUBLIC['supplement']['cleanup']:
            with self.subTest(row=row):
                opened, closed = [], []
                def tracked(*args, **kwargs):
                    descriptor = original_open(*args, **kwargs)
                    opened.append(descriptor)
                    return descriptor
                def close(descriptor):
                    closed.append(descriptor)
                    original_close(descriptor)
                    if row['close'] == 'OSError':
                        raise OSError(errno.EIO, 'public close failure')
                    raise KeyboardInterrupt('public close cancellation')
                if row['read'] == 'success':
                    read = mock.patch.object(self.brain.os, 'read', wraps=os.read)
                else:
                    error = (KeyboardInterrupt('public read cancellation') if row['read'] == 'KeyboardInterrupt'
                             else OSError(errno.EIO, 'public read failure'))
                    read = mock.patch.object(self.brain.os, 'read', side_effect=error)
                expected = self.brain.BrainError if row['expected'] == 'BrainError' else KeyboardInterrupt
                with mock.patch.object(self.brain.os, 'open', side_effect=tracked), read, mock.patch.object(self.brain.os, 'close', side_effect=close):
                    with self.assertRaises(expected):
                        self.brain.load_config(self.root)
                self.assertEqual(row['close_attempts'], len(closed))
                self.assertEqual(opened, closed)
                with self.assertRaises(OSError) as caught:
                    os.fstat(opened[0])
                self.assertEqual(errno.EBADF, caught.exception.errno)


@unittest.skipUnless(os.name != 'nt' and shutil.which('bash') and shutil.which('timeout'), 'actual POSIX hook profile; native Windows unrun')
class ConfigObservationNativeTests(unittest.TestCase):
    def test_actual_governed_cli_and_hooks_refuse_unknown_configuration(self):
        row = PUBLIC['native']
        with tempfile.TemporaryDirectory(prefix='kestrel-native-') as temporary:
            root = Path(temporary)
            for relative in ('memory-bank/scripts', 'memory-bank/templates', 'project-brain/config', 'project-brain/schemas', 'project-brain/templates'):
                shutil.copytree(ROOT / 'PHP Core' / relative, root / relative, ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
            original = os.environ.get('KESTREL_ORIGINAL_RUNTIME')
            if original:
                for item in Path(original).glob('*.py'):
                    shutil.copyfile(item, root / 'memory-bank/scripts' / item.name)
            for host in ('.claude', '.codex'):
                destination = root / host / 'hooks'
                destination.mkdir(parents=True)
                shutil.copyfile(ROOT / 'PHP Core' / host / 'hooks/working-memory-read.sh', destination / 'working-memory-read.sh')
            (root / 'memory-bank/chunks').mkdir()
            (root / '.gitignore').write_text('memory-bank/local/\nproject-brain/local/\n', encoding='utf-8')
            config = root / 'project-brain/config/runtime.json'
            config.write_text(json.dumps(row['config']), encoding='utf-8')
            source = root / row['source']
            source.parent.mkdir()
            source.write_text(row['source_text'], encoding='utf-8')
            environment = {key: value for key, value in os.environ.items() if not key.startswith(('GIT_', 'CONTEXT_', 'ACCELERATOR_', 'PROJECT_BRAIN_'))}
            environment.update(GIT_OPTIONAL_LOCKS='0', PYTHONDONTWRITEBYTECODE='1', PYTHONIOENCODING='utf-8', HOME=str(root / 'private-home'), TMPDIR=str(root / 'private-tmp'))
            Path(environment['HOME']).mkdir()
            Path(environment['TMPDIR']).mkdir()
            launcher = root / 'private-bin'
            launcher.mkdir()
            (launcher / 'python3').symlink_to(Path(sys.executable).resolve())
            environment['PATH'] = str(launcher) + os.pathsep + environment.get('PATH', '')
            subprocess.run(['git', '-C', str(root), 'init', '-q', '-b', row['task_id']], env=environment, check=True, capture_output=True)
            cli = root / 'memory-bank/scripts/context.py'
            def call(*arguments):
                return subprocess.run([sys.executable, '-B', str(cli), *arguments], cwd=root, env=environment, capture_output=True, timeout=4)
            started = call('start', '--task-id', row['task_id'], '--goal', row['goal'], '--source', row['source'], '--json')
            self.assertEqual(0, started.returncode, started.stderr)
            self.assertEqual(row['start_revision'], json.loads(started.stdout)['revision'])
            updated = call('update', '--task-id', row['task_id'], '--progress', row['progress'], '--next-step', row['next_step'], '--revision', str(row['start_revision']), '--json')
            self.assertEqual(0, updated.returncode, updated.stderr)
            self.assertEqual(row['update_revision'], json.loads(updated.stdout)['revision'])
            self.assertEqual(row['progress'], json.loads(updated.stdout)['progress'])
            database = root / 'memory-bank/local/context.db'
            database_bytes = database.read_bytes()
            shared = {str(item.relative_to(root)): item.read_bytes() for item in (root / 'project-brain').rglob('*') if item.is_file() and 'local' not in item.relative_to(root).parts and item != config}
            config.unlink()
            config.mkdir()
            result = call('refresh', '--host', 'codex', '--query', row['query'], '--task-id', row['task_id'], '--ephemeral', '--sanitize', '--read-only')
            expected = row['expected']
            self.assertEqual(expected['readonly_returncode'], result.returncode)
            self.assertEqual(expected['readonly_stdout'], result.stdout.decode('utf-8'))
            self.assertEqual(expected['readonly_stderr'].replace('{ROOT}', str(root)), result.stderr.decode('utf-8'))
            for host in ('.claude', '.codex'):
                hooked = subprocess.run(['bash', str(root / host / 'hooks/working-memory-read.sh')], cwd=root, env=environment, input=json.dumps({'prompt': row['query']}).encode('utf-8'), capture_output=True, timeout=6)
                self.assertEqual(expected['hook_returncode'], hooked.returncode)
                self.assertEqual(expected['hook_stdout'].replace('{ROOT}', str(root)), hooked.stdout.decode('utf-8'))
                self.assertEqual(b'', hooked.stderr)
            self.assertEqual(database_bytes, database.read_bytes())
            self.assertEqual(shared, {str(item.relative_to(root)): item.read_bytes() for item in (root / 'project-brain').rglob('*') if item.is_file() and 'local' not in item.relative_to(root).parts and item != config})


if __name__ == '__main__':
    unittest.main()
