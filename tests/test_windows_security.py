"""State-directory privacy checks; ACL and junction assertions run on Windows."""
from __future__ import annotations

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import windows_security


class PrivateDirectoryTests(unittest.TestCase):
    def test_posix_helper_keeps_a_private_directory(self):
        if os.name == "nt":
            self.skipTest("POSIX permissions")
        with tempfile.TemporaryDirectory() as temporary:
            state = windows_security.secure_private_dir(Path(temporary) / "state")
            self.assertEqual(state.stat().st_mode & 0o777, 0o700)


@unittest.skipUnless(os.name == "nt", "Windows ACL and junction coverage")
class WindowsPrivateDirectoryTests(unittest.TestCase):
    def test_existing_current_user_directory_becomes_private(self):
        with tempfile.TemporaryDirectory() as temporary:
            state = Path(temporary) / "state"
            state.mkdir()
            (state / "sessions.sqlite3").write_text("private session fixture", encoding="utf-8")
            self.assertEqual(windows_security.secure_private_dir(state), state)
            descriptor = windows_security.fs.open_target_directory(state, security=True)
            try:
                protected, entries = windows_security._acl_summary(descriptor)
            finally:
                windows_security.fs.close(descriptor)
            current = windows_security._CurrentUser().sid_text
            self.assertTrue(protected)
            self.assertEqual({sid for _, _, sid in entries}, {current, "S-1-5-18"})
            self.assertTrue(all(flags & 0x03 == 0x03 and mask == 0x1f01ff
                                for flags, mask, _ in entries))
            root = windows_security.fs.open_target_directory(state)
            child = windows_security.fs.open_security('sessions.sqlite3', dir_fd=root)
            try:
                protected, entries = windows_security._acl_summary(child)
                self.assertTrue(protected)
                self.assertEqual({sid for _, _, sid in entries}, {current, "S-1-5-18"})
            finally:
                windows_security.fs.close(child)
                windows_security.fs.close(root)


    def test_fresh_nested_root_and_live_lock_remain_private(self):
        with tempfile.TemporaryDirectory() as temporary:
            state = Path(temporary) / 'one' / 'two' / 'state'
            self.assertEqual(windows_security.secure_private_dir(state), state)
            lock = windows_security.fs.open_lock(state / 'server.lock')
            try:
                self.assertEqual(windows_security.secure_private_dir(state), state)
            finally:
                windows_security.fs.close(lock)

    def test_junction_is_refused_without_reading_the_external_sentinel(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            outside = root / "outside"
            outside.mkdir()
            sentinel = outside / "secret.txt"
            sentinel.write_text("OUTSIDE PRIVATE SENTINEL", encoding="utf-8")
            state = root / "state"
            completed = subprocess.run(
                ["cmd", "/d", "/c", "mklink", "/J", str(state), str(outside)],
                capture_output=True, text=True, check=False,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            with self.assertRaisesRegex(windows_security.StateSecurityError,
                                        "Harness state directory is unavailable or insecure") as error:
                windows_security.secure_private_dir(state)
            self.assertNotIn("OUTSIDE PRIVATE SENTINEL", str(error.exception))
            self.assertEqual(sentinel.read_text(encoding="utf-8"), "OUTSIDE PRIVATE SENTINEL")


if __name__ == "__main__":
    unittest.main()
