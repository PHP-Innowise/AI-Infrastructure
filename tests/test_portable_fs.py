"""Regression coverage for rooted filesystem primitives.

Windows-specific junction and alternate-data-stream cases are exercised on a
native Windows runner.  Linux coverage proves that the POSIX path preserves the
existing descriptor-relative, no-follow behavior.
"""
from __future__ import annotations

import os
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import portable_fs as fs


class PortableFsTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name) / 'root'
        self.root.mkdir()
        self.fd = fs.open_target_directory(self.root)
        self.addCleanup(os.close, self.fd)

    def write(self, name, body):
        fd = fs.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | fs.O_NOFOLLOW, 0o600,
                     dir_fd=self.fd)
        try:
            os.write(fd, body)
        finally:
            os.close(fd)

    def test_rooted_write_read_link_and_replace(self):
        self.write('temporary', b'first')
        self.write('target', b'old')
        fs.replace('temporary', 'target', src_dir_fd=self.fd, dst_dir_fd=self.fd)
        target = fs.open('target', os.O_RDONLY | fs.O_NOFOLLOW, dir_fd=self.fd)
        try:
            self.assertEqual(os.read(target, 20), b'first')
            self.assertEqual(fs.fstat(target).st_nlink, 1)
        finally:
            os.close(target)
        fs.link('target', 'linked', src_dir_fd=self.fd, dst_dir_fd=self.fd, follow_symlinks=False)
        linked = fs.open('linked', os.O_RDONLY | fs.O_NOFOLLOW, dir_fd=self.fd)
        try:
            self.assertEqual(os.read(linked, 20), b'first')
            self.assertGreaterEqual(fs.fstat(linked).st_nlink, 2)
        finally:
            os.close(linked)

    def test_no_follow_refuses_symlink_to_external_sentinel(self):
        outside = self.root.parent / 'outside.txt'
        outside.write_bytes(b'outside sentinel')
        try:
            (self.root / 'redirect').symlink_to(outside)
        except OSError as error:
            if os.name == 'nt':
                self.skipTest('Creating a Windows symlink requires a privilege or Developer Mode.')
            raise error
        with self.assertRaises(OSError):
            fs.open('redirect', os.O_RDONLY | fs.O_NOFOLLOW, dir_fd=self.fd)
        self.assertEqual(outside.read_bytes(), b'outside sentinel')

    def test_root_walk_refuses_an_ancestor_symlink(self):
        outside = self.root.parent / 'outside-directory'
        outside.mkdir()
        alias = self.root.parent / 'root-alias'
        try:
            alias.symlink_to(outside, target_is_directory=True)
        except OSError as error:
            if os.name == 'nt':
                self.skipTest('Creating a Windows symlink requires a privilege or Developer Mode.')
            raise error
        with self.assertRaises(OSError):
            fs.open_target_directory(alias / 'child')

    def test_open_lock_refuses_a_second_owner(self):
        lock = fs.open_lock(self.root / 'server.lock')
        try:
            with self.assertRaises(OSError):
                fs.open_lock(self.root / 'server.lock')
        finally:
            fs.close(lock)

    @unittest.skipUnless(os.name == 'nt', 'Windows reparse-point coverage')
    def test_windows_rejects_ads_device_names_and_junctions(self):
        outside = self.root.parent / 'outside'
        outside.mkdir()
        (outside / 'sentinel.txt').write_text('outside', encoding='utf-8')
        for name in ('name:stream', 'NUL', 'CON.txt', 'trailing.', 'trailing '):
            with self.subTest(name=name), self.assertRaises(ValueError):
                fs.open(name, os.O_RDONLY, dir_fd=self.fd)
        junction = self.root / 'junction'
        status = os.system('cmd /d /c mklink /J "' + str(junction) + '" "' + str(outside) + '" >NUL')
        self.assertEqual(status, 0)
        with self.assertRaises(OSError):
            fs.open('junction', os.O_RDONLY | fs.O_DIRECTORY | fs.O_NOFOLLOW, dir_fd=self.fd)
        self.assertEqual((outside / 'sentinel.txt').read_text(encoding='utf-8'), 'outside')

    @unittest.skipUnless(os.name == 'nt', 'Windows NT API smoke coverage')
    def test_windows_root_handle_supports_directory_stat_and_scandir(self):
        self.write('visible.txt', b'visible')
        metadata = fs.fstat(self.fd)
        self.assertTrue(os.path.isdir(self.root))
        self.assertTrue(metadata.st_mode & 0o040000)
        self.assertIn('visible.txt', fs.listdir(self.fd))
        with fs.scandir(self.fd) as entries:
            self.assertTrue(any(entry.name == 'visible.txt' and entry.is_file(follow_symlinks=False)
                                for entry in entries))


@unittest.skipUnless(os.name == 'nt', 'native Windows: the installer walks a path by NT handles there')
class InstallerHandleCallsTests(unittest.TestCase):
    """scripts/install_accelerator.py carries its own copy of these calls.

    The Harness runs the installer alone from a staging folder, so it cannot
    import this module; on Windows its reads and writes walk by the copy
    (`_HandleCalls`), which only a native runner can exercise.
    """

    def setUp(self):
        import install_accelerator
        self.installer = install_accelerator
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.project = self.root / 'project'
        self.project.mkdir()
        self.outside = self.root / 'outside'
        self.outside.mkdir()
        (self.outside / 'sentinel.txt').write_text('outside', encoding='utf-8')

    def test_the_walk_takes_the_handle_calls(self):
        self.assertIsNotNone(self.installer._HANDLE_CALLS)
        self.assertIs(self.installer._HANDLE_CALLS, self.installer._relative_calls())

    def test_write_read_replace_and_times(self):
        times = (1_577_880_000_000_000_000, 1_577_880_000_000_000_000)
        self.installer.write_confined(self.project, '.cursor/rules/mcp.json', b'first', times=times)
        path = self.project / '.cursor' / 'rules' / 'mcp.json'
        self.assertEqual(b'first', path.read_bytes())
        self.assertEqual(times[1], path.stat().st_mtime_ns)
        self.installer.write_confined(self.project, '.cursor/rules/mcp.json', b'second', durable=False)
        self.assertEqual(b'second', self.installer.read_confined(self.project, '.cursor/rules/mcp.json'))
        self.assertIsNone(self.installer.read_confined(self.project, '.cursor/rules/missing.json'))
        # Renamed into place: no temporary file is left beside it.
        self.assertEqual(['mcp.json'], sorted(entry.name for entry in path.parent.iterdir()))

    def test_a_junction_on_the_path_is_refused(self):
        junction = self.project / '.cursor'
        status = os.system('cmd /d /c mklink /J "' + str(junction) + '" "' + str(self.outside) + '" >NUL')
        self.assertEqual(status, 0)
        for attempt in (lambda: self.installer.write_confined(self.project, '.cursor/mcp.json', b'{}'),
                        lambda: self.installer.read_confined(self.project, '.cursor/sentinel.txt')):
            with self.assertRaises(self.installer.UnsafePathError) as caught:
                attempt()
            self.assertIn('.cursor is a symbolic link', str(caught.exception))
        self.assertEqual(['sentinel.txt'], sorted(entry.name for entry in self.outside.iterdir()))

    def test_a_folder_held_cannot_be_renamed(self):
        (self.project / '.cursor').mkdir()
        calls = self.installer._HANDLE_CALLS
        project = calls.open_project(self.project)
        try:
            folder = calls.open_folder('.cursor', project)
            try:
                with self.assertRaises(PermissionError):
                    os.rename(self.project / '.cursor', self.project / 'moved')
            finally:
                calls.close(folder)
        finally:
            calls.close(project)
        os.rename(self.project / '.cursor', self.project / 'moved')


if __name__ == '__main__':
    unittest.main()
