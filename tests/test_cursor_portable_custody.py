"""Fresh rooted adapter controls plus actual-Windows-only NT acceptance.

POSIX adapter checks do not certify NT calls. No native models or network.
"""
import importlib
import ast
import json
import os
import ntpath
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / 'PHP Core/memory-bank/scripts'
PUBLIC = json.loads((ROOT / 'tests/fixtures/cursor_portable_custody_public.json').read_text(encoding='utf-8'))


class NtCleanupSourceTests(unittest.TestCase):
    """Exact installed function bodies with fake handles; not NT runtime proof."""
    def function(self, name, namespace):
        source = (SCRIPTS / 'context_retrieval.py').read_text(encoding='utf-8')
        factory = next(n for n in ast.parse(source).body if isinstance(n, ast.FunctionDef)
                       and n.name == '_cursor_nt_backend')
        node = next(n for n in factory.body if isinstance(n, ast.FunctionDef) and n.name == name)
        module = ast.Module(body=[node], type_ignores=[])
        exec(compile(ast.fix_missing_locations(module), 'installed-NT-function', 'exec'), namespace)
        return namespace[name]

    def root_namespace(self, public):
        class Handle:
            def __init__(self, value=None): self.value = value
        return {'os': SimpleNamespace(fspath=os.fspath, fsdecode=os.fsdecode, O_BINARY=0,
                                      close=Mock()), 'ntpath': ntpath, 'HANDLE': Handle,
            'ctypes': SimpleNamespace(c_void_p=Handle, WinError=lambda _code: OSError(),
                                      get_last_error=lambda: 0),
            'wintypes': SimpleNamespace(LPCWSTR=object, DWORD=object, LPVOID=object),
            '_kernel32': SimpleNamespace(CreateFileW=Mock(return_value=public['raw_handle'])),
            'FILE_LIST_DIRECTORY': 1, 'FILE_TRAVERSE': 32, 'FILE_READ_ATTRIBUTES': 128,
            'SYNCHRONIZE': 1048576, 'FILE_SHARE_READ': 1, 'FILE_SHARE_WRITE': 2,
            'FILE_DIRECTORY_FILE': 1, '_metadata': Mock(), '_close': Mock(),
            'msvcrt': SimpleNamespace(open_osfhandle=Mock(return_value=public['root_fd'])),
            '_open_relative': Mock(return_value=public['child_fd'])}

    def test_root_abort_and_previous_close_failure_release_current_owner_once(self):
        public = json.loads((ROOT / 'tests/fixtures/cursor_nt_cleanup_public.json').read_text(encoding='utf-8'))
        for case in public['cases'][1:]:
            with self.subTest(case=case['name']):
                ns = self.root_namespace(public)
                error = KeyboardInterrupt() if case['name'] == 'root_walk_abort' else OSError('synthetic')
                if case['name'] == 'root_walk_abort':
                    ns['_open_relative'].side_effect = error
                else:
                    def close(fd):
                        if fd == public['root_fd']: raise error
                    ns['os'].close.side_effect = close
                function = self.function('_open_root', ns)
                with self.assertRaises(type(error)) as caught:
                    function('C:\\pine')
                self.assertIs(error, caught.exception)
                self.assertEqual(case['expected_fd_closed'], [x.args[0] for x in ns['os'].close.call_args_list])
                self.assertEqual(case['expected_raw_closed'], [x.args[0] for x in ns['_close'].call_args_list])

    def test_relative_metadata_abort_closes_raw_handle_before_transfer(self):
        public = json.loads((ROOT / 'tests/fixtures/cursor_nt_cleanup_public.json').read_text(encoding='utf-8'))
        ns = self.root_namespace(public)
        error = KeyboardInterrupt(); ns['_metadata'].side_effect = error
        ns.update({'_component': lambda n: n, '_unicode': lambda _n: (None, object()),
                   'raw_handle': lambda fd: fd, 'OBJECT_ATTRIBUTES': lambda *a: object(),
                   'IO_STATUS_BLOCK': object, 'FILE_OPEN': 1, 'FILE_ATTRIBUTE_NORMAL': 128,
                   'OBJ_CASE_INSENSITIVE': 64, 'FILE_WRITE_DATA': 2, 'FILE_READ_DATA': 1,
                   'FILE_APPEND_DATA': 4, 'FILE_OPEN_REPARSE_POINT': 2097152,
                   'FILE_SYNCHRONOUS_IO_NONALERT': 32})
        ns['ctypes'].sizeof = lambda _t: 0; ns['ctypes'].pointer = lambda x: x
        ns['ctypes'].byref = lambda x: x
        def acquire(output, *unused): output.value = public['raw_handle']; return 0
        ns['_nt_create'] = acquire
        function = self.function('_open_relative', ns)
        with self.assertRaises(KeyboardInterrupt) as caught:
            function(10, 'owned-child', 1)
        self.assertIs(error, caught.exception)
        self.assertEqual(1, ns['_close'].call_count)
        self.assertEqual(public['raw_handle'], ns['_close'].call_args.args[0].value)
        ns['msvcrt'].open_osfhandle.assert_not_called()


class PortableCursorTests(unittest.TestCase):
    def setUp(self):
        if not shutil.which('git'):
            self.skipTest('Git tracking proof required')
        clean = {k: v for k, v in os.environ.items() if not k.startswith(('GIT_', 'ACCELERATOR_', 'CONTEXT_'))}
        clean['GIT_OPTIONAL_LOCKS'] = '0'
        env = patch.dict(os.environ, clean, clear=True); env.start(); self.addCleanup(env.stop)
        sys.path.insert(0, str(SCRIPTS))
        try:
            self.r = importlib.import_module('context_retrieval')
        finally:
            sys.path.pop(0)
        temp = tempfile.TemporaryDirectory(prefix='pine-portable-cursor-')
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name).resolve()
        subprocess.run(['git', 'init', '-q', str(self.root)], check=True, capture_output=True)
        layout = patch.object(self.r.workspace_roots, 'TOOLING_ROOT', self.root)
        layout.start(); self.addCleanup(layout.stop)
        self.rule = self.root / '.cursor/rules/working-memory.mdc'
        self.proof = self.root / 'memory-bank/local/cursor-rule.json'
        if os.name == 'posix':
            # Exercise the per-call rooted adapter route with real POSIX FDs.
            self.files = SimpleNamespace(WINDOWS=False, open=os.open, stat=os.stat,
                fstat=os.fstat, mkdir=os.mkdir, unlink=os.unlink, replace=os.replace,
                O_DIRECTORY=os.O_DIRECTORY, O_NOFOLLOW=os.O_NOFOLLOW, O_NONBLOCK=os.O_NONBLOCK,
                open_root=lambda p: os.open(p, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW))
            backend = patch.object(self.r, '_cursor_backend', return_value=self.files)
            backend.start(); self.addCleanup(backend.stop)
        else:
            self.files = self.r._cursor_backend()

    def call(self, action='write', **options):
        return self.r.cursor_rule(self.root, action, task=PUBLIC['task'],
                                  capsule=options.pop('capsule', PUBLIC['capsule']), **options)

    def owned(self):
        self.assertEqual('written', self.call()['state'])
        return self.rule.read_bytes(), self.proof.read_bytes()

    def test_complete_utf8_lifecycle_and_replacement_keep_whole_oracle(self):
        raw, _ = self.owned()
        self.assertEqual(PUBLIC['expected_prompt'].encode('utf-8'), raw)
        self.assertEqual(PUBLIC['expected_prompt_bytes'], len(raw))
        inode = self.rule.stat().st_ino
        self.assertEqual('unchanged', self.call()['state'])
        self.assertEqual(inode, self.rule.stat().st_ino)
        self.assertEqual('written', self.call(capsule=PUBLIC['replacement'])['state'])
        self.assertIn('three reviewers', self.rule.read_text(encoding='utf-8'))
        size = len(self.rule.read_bytes())
        result = self.call('retire')
        self.assertEqual('removed', result['state'])
        self.assertEqual(size, result['bytes_removed'])
        self.assertFalse(self.rule.exists()); self.assertFalse(self.proof.exists())

    def test_orphan_prompt_can_recreate_turn_and_proof(self):
        self.owned(); self.rule.unlink()
        self.assertEqual('absent', self.call('prompt-held')['state'])
        self.assertEqual('written', self.call(phase='turn')['state'])
        expected = PUBLIC['expected_prompt'].replace('Session context retrieved for a recent prompt',
                                                     'Session context as of end of previous turn')
        self.assertEqual(expected.encode('utf-8'), self.rule.read_bytes())
        self.assertEqual('turn', json.loads(self.proof.read_bytes())['phase'])

    def test_user_tracked_and_selector_bound_files_remain_untouched(self):
        self.rule.parent.mkdir(parents=True)
        self.rule.write_text(PUBLIC['foreign_sentinel'], encoding='utf-8')
        self.assertEqual('preserved', self.call()['state'])
        self.rule.unlink(); old = self.owned()
        subprocess.run(['git', '-C', str(self.root), 'add', '-f', str(self.rule)], check=True, capture_output=True)
        self.assertEqual('preserved', self.call('retire')['state'])
        self.assertEqual(old, (self.rule.read_bytes(), self.proof.read_bytes()))
        subprocess.run(['git', '-C', str(self.root), 'rm', '--cached', '-q', str(self.rule)], check=True, capture_output=True)
        with patch.dict(os.environ, {'GIT_INDEX_FILE': str(self.root / 'alternate-index')}):
            self.assertEqual('preserved', self.call('retire')['state'])
        self.assertEqual(old, (self.rule.read_bytes(), self.proof.read_bytes()))

    def test_destination_appearing_before_publication_is_never_clobbered(self):
        publish = self.r._cursor_publish
        def appear(parent, name, payload, expected, limit, **options):
            if name == 'working-memory.mdc' and expected is None:
                self.rule.write_text(PUBLIC['foreign_sentinel'], encoding='utf-8')
            return publish(parent, name, payload, expected, limit, **options)
        with patch.object(self.r, '_cursor_publish', side_effect=appear):
            self.assertEqual('incomplete', self.call()['state'])
        self.assertEqual(PUBLIC['foreign_sentinel'], self.rule.read_text(encoding='utf-8'))
        self.assertFalse(self.proof.exists())
        self.assertEqual([], list(self.rule.parent.glob('.cursor-rule-*.tmp')))

    def test_write_failure_cleans_only_its_temporary_and_can_try_fresh(self):
        with patch.object(self.r.os, 'write', side_effect=OSError('synthetic')):
            self.assertEqual('incomplete', self.call()['state'])
        self.assertFalse(self.rule.exists()); self.assertFalse(self.proof.exists())
        self.assertEqual([], list(self.rule.parent.glob('.cursor-rule-*.tmp')))
        self.assertEqual('written', self.call()['state'])

    def test_native_process_registry_reentry_uses_same_lock_owner(self):
        self.owned()
        with self.r.brain_runtime.mutation_lock(self.root):
            self.assertEqual('written', self.call(capsule=PUBLIC['replacement'])['state'])
        self.assertEqual('removed', self.call('retire')['state'])

    def test_registry_key_matches_native_resolved_path_even_with_lexical_alias(self):
        public = json.loads((ROOT / 'tests/fixtures/cursor_native_lock_key_public.json').read_text(encoding='utf-8'))
        self.owned()
        parent = self.files.open_root(self.root / 'project-brain/local')
        alias = self.root.parent / '..' / self.root.parent.name / self.root.name
        key = str((self.root / 'project-brain/local/runtime.lock').resolve())
        try:
            with self.r._cursor_lock(alias, parent, files=self.files):
                self.assertEqual(1, self.r.brain_runtime._LOCK_STATE.depths.get(key))
                with self.r.brain_runtime.mutation_lock(self.root):
                    self.assertEqual(public['expected_native_depth_under_cursor'],
                                     self.r.brain_runtime._LOCK_STATE.depths[key])
        finally:
            os.close(parent)

    @unittest.skipUnless(os.name == 'nt', 'actual Windows case-alias lock correspondence')
    def test_windows_existing_case_alias_uses_native_reentrant_key(self):
        public = json.loads((ROOT / 'tests/fixtures/cursor_native_lock_key_public.json').read_text(encoding='utf-8'))
        self.owned()
        lock = self.root / 'project-brain/local/runtime.lock'
        lock.rename(lock.with_name(public['Windows_existing_lock_spelling']))
        with self.r.brain_runtime.mutation_lock(self.root):
            self.assertEqual(public['expected_changed_capsule_state'],
                             self.call(capsule=PUBLIC['replacement'])['state'])

    def test_unsupported_backend_preserves_the_exact_owned_pair(self):
        old = self.owned()
        with patch.object(self.r, '_cursor_backend', return_value=None):
            result = self.call('retire')
        self.assertEqual('preserved', result['state'])
        self.assertEqual('unsupported_custody', result['reason'])
        self.assertEqual(old, (self.rule.read_bytes(), self.proof.read_bytes()))

    @unittest.skipUnless(os.name == 'nt', 'actual Windows NT reparse/handle coverage')
    def test_windows_junction_parent_is_refused_and_outside_sentinel_preserved(self):
        outside = self.root / 'outside'; outside.mkdir()
        sentinel = outside / 'working-memory.mdc'; sentinel.write_text(PUBLIC['foreign_sentinel'])
        cursor = self.root / '.cursor'; cursor.mkdir()
        subprocess.run(['cmd', '/d', '/c', 'mklink', '/J', str(cursor / 'rules'), str(outside)],
                       check=True, capture_output=True)
        self.assertEqual('preserved', self.call()['state'])
        self.assertEqual(PUBLIC['foreign_sentinel'], sentinel.read_text(encoding='utf-8'))
        self.assertFalse(self.proof.exists())

    @unittest.skipUnless(os.name == 'nt', 'actual Windows NT no-clobber rename')
    def test_windows_absent_rename_has_no_replace_flag(self):
        directory = self.root / 'publication'; directory.mkdir()
        (directory / 'temporary').write_bytes(b'new')
        (directory / 'foreign').write_bytes(b'foreign')
        fd = self.files.open_root(directory)
        try:
            with self.assertRaises(OSError):
                self.files.publish_absent('temporary', 'foreign', src_dir_fd=fd, dst_dir_fd=fd)
            self.assertEqual(b'foreign', (directory / 'foreign').read_bytes())
            self.files.publish_absent('temporary', 'published', src_dir_fd=fd, dst_dir_fd=fd)
            self.assertEqual(b'new', (directory / 'published').read_bytes())
            self.assertFalse((directory / 'temporary').exists())
        finally:
            os.close(fd)

    @unittest.skipUnless(os.name == 'nt', 'actual Windows Brain byte-lock interoperability')
    def test_windows_native_and_cursor_locks_conflict_in_both_directions(self):
        import msvcrt
        self.owned()
        lock_path = self.root / 'project-brain/local/runtime.lock'
        parent = self.files.open_root(lock_path.parent)
        try:
            with lock_path.open('a+') as handle:
                self.r.brain_runtime._file_lock(handle)
                try:
                    with self.assertRaises(OSError):
                        with self.r._cursor_lock(self.root, parent, files=self.files):
                            self.fail('Native lock bypassed')
                finally:
                    self.r.brain_runtime._file_lock(handle, unlock=True)
            with self.r._cursor_lock(self.root, parent, files=self.files):
                with lock_path.open('a+') as handle:
                    handle.seek(0)
                    with self.assertRaises(OSError):
                        msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
        finally:
            os.close(parent)

    @unittest.skipUnless(os.name == 'nt', 'actual Windows held-parent rename refusal')
    def test_windows_held_root_cannot_be_renamed(self):
        directory = self.root / 'held-parent'; directory.mkdir()
        fd = self.files.open_root(directory)
        try:
            with self.assertRaises(OSError):
                directory.rename(self.root / 'renamed-parent')
        finally:
            os.close(fd)
        directory.rename(self.root / 'renamed-parent')


if __name__ == '__main__':
    unittest.main()
