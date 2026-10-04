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


if __name__ == '__main__':
    unittest.main()
