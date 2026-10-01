"""Portable process-runtime checks without native provider CLIs."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import threading
import unittest
from unittest.mock import patch, Mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "harness/src"))
from harness import process_runtime


class ProcessRuntimeTests(unittest.TestCase):
    def test_daemon_kwargs_match_the_host_process_model(self):
        options = process_runtime.daemon_kwargs()
        if os.name == "nt":
            self.assertEqual(options, {"creationflags": subprocess.DETACHED_PROCESS | subprocess.CREATE_NEW_PROCESS_GROUP})
        else:
            self.assertEqual(options, {"start_new_session": True})

    def test_guarded_launch_preserves_native_output_and_exit_code(self):
        read_fd, write_fd = os.pipe()
        process = None
        try:
            process = process_runtime.launch_guarded(
                [sys.executable, "-c", "import sys; print('guarded output'); print('guarded error', file=sys.stderr); sys.exit(7)"],
                read_fd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            )
            os.close(read_fd)
            read_fd = None
            stdout, stderr = process.communicate(timeout=5)
            self.assertEqual(process.returncode, 7)
            self.assertEqual(stdout.strip(), b"guarded output")
            self.assertEqual(stderr.strip(), b"guarded error")
        finally:
            if read_fd is not None:
                os.close(read_fd)
            os.close(write_fd)
            if process is not None and process.poll() is None:
                process_runtime.signal_tree(process, force=True)
                process.wait(timeout=3)

    def test_pipe_readers_preserve_chunks_with_bounded_per_pipe_queue(self):
        process = subprocess.Popen(
            [sys.executable, "-u", "-c", "import sys; sys.stdout.buffer.write(b'first\\nsecond\\r\\n'); sys.stdout.buffer.flush()"],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        )
        readers = process_runtime.PipeReaders(chunk_size=4)
        try:
            readers.register(process.stdout, "stdout")
            body = bytearray()
            while True:
                event = readers.read(timeout=2)
                self.assertIsNotNone(event)
                self.assertEqual(event.data, "stdout")
                if not event.body:
                    break
                body.extend(event.body)
            self.assertEqual(process.wait(timeout=3), 0)
            self.assertEqual(bytes(body), b"first\nsecond\r\n")
        finally:
            readers.close()
            if process.poll() is None:
                process.kill()
                process.wait(timeout=3)


    def test_threaded_selector_drains_after_process_exit_and_repeats_eof(self):
        process = subprocess.Popen([sys.executable, '-c', "import sys;sys.stdout.write('tail'*20000)"], stdout=subprocess.PIPE)
        with patch.object(process_runtime, 'WINDOWS', True):
            selector = process_runtime.PipeSelector()
        selector.register(process.stdout, 1)
        try:
            body = bytearray()
            while True:
                ready = selector.select(3)
                self.assertTrue(ready)
                chunk = selector.read(process.stdout, 65536)
                if not chunk:
                    break
                body.extend(chunk)
            self.assertEqual(process.wait(timeout=3), 0)
            self.assertEqual(body, b'tail'*20000)
            self.assertTrue(selector.select(0))
            self.assertEqual(selector.read(process.stdout, 65536), b'')
            selector.unregister(process.stdout)
            self.assertFalse(selector.get_map())
        finally:
            process_runtime.reap_tree(process)
            selector.close()

    def test_threaded_reader_io_error_produces_eof(self):
        readers = process_runtime.PipeReaders()
        stream = Mock()
        stream.fileno.side_effect = OSError('closed pipe')
        readers.register(stream)
        try:
            event = readers.read(2)
            self.assertIsNotNone(event)
            self.assertEqual(event.body, b'')
        finally:
            readers.close()

    def test_windows_signal_failure_does_not_mask_cleanup(self):
        process = Mock()
        process.terminate.side_effect = OSError('already terminated')
        with patch.object(process_runtime, 'WINDOWS', True):
            process_runtime.signal_tree(process)
        process.wait.side_effect = subprocess.TimeoutExpired('fixture', 3)
        with patch.object(process_runtime, 'WINDOWS', True):
            owner = Mock(stopping=threading.Event())
            self.assertFalse(process_runtime.reap_tree(process, owner=owner))
            self.assertTrue(owner.stopping.is_set())


@unittest.skipUnless(os.name == "nt", "Windows Job Objects require native Windows")
class WindowsJobObjectTests(unittest.TestCase):
    def wait_for(self, predicate, timeout=5):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(.02)
        self.fail("Timed out waiting for a guarded Windows process")

    @staticmethod
    def running(pid):
        # os.kill(pid, 0) terminates a process on Windows; use a query handle.
        import ctypes
        from ctypes import wintypes
        api = ctypes.WinDLL('kernel32', use_last_error=True)
        api.OpenProcess.argtypes = (wintypes.DWORD, wintypes.BOOL, wintypes.DWORD)
        api.OpenProcess.restype = wintypes.HANDLE
        api.GetExitCodeProcess.argtypes = (wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD))
        api.CloseHandle.argtypes = (wintypes.HANDLE,)
        handle = api.OpenProcess(0x1000, False, pid)
        if not handle:
            return False
        try:
            code = wintypes.DWORD()
            return bool(api.GetExitCodeProcess(handle, ctypes.byref(code))) and code.value == 259
        finally:
            api.CloseHandle(handle)

    def test_guard_retains_admission_lock_after_parent_closes_it(self):
        from harness.filesystem import fs, secure_private_dir
        with tempfile.TemporaryDirectory() as temporary:
            state = secure_private_dir(Path(temporary) / 'state')
            marker = state / 'started'
            lock = fs.open_lock(state / 'runner.lock')
            read_fd, write_fd = os.pipe()
            process = None
            try:
                code = "import pathlib,time;pathlib.Path(" + repr(str(marker)) + ").write_text('started');time.sleep(60)"
                process = process_runtime.launch_guarded([sys.executable, '-c', code], read_fd,
                    lock_fd=lock, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
                os.close(read_fd); read_fd = None
                self.wait_for(marker.exists)
                fs.close(lock); lock = None
                with self.assertRaises(OSError):
                    fs.open_lock(state / 'runner.lock')
                os.close(write_fd); write_fd = None
                process.wait(timeout=5)
                replacement = fs.open_lock(state / 'runner.lock')
                fs.close(replacement)
            finally:
                if read_fd is not None: os.close(read_fd)
                if write_fd is not None: os.close(write_fd)
                if lock is not None: fs.close(lock)
                if process is not None:
                    process_runtime.reap_tree(process)
                    process.stderr.close()

    def test_normal_exit_kills_descendant_while_watchdog_writer_remains_open(self):
        with tempfile.TemporaryDirectory() as temporary:
            marker = Path(temporary) / 'descendant.pid'
            read_fd, write_fd = os.pipe()
            process = None
            try:
                code = (
                    "import pathlib,subprocess,sys; "
                    "child=subprocess.Popen([sys.executable,'-c','import time;time.sleep(60)']); "
                    f"pathlib.Path({str(marker)!r}).write_text(str(child.pid)); sys.exit(7)"
                )
                process = process_runtime.launch_guarded(
                    [sys.executable, '-c', code], read_fd, stdin=subprocess.DEVNULL,
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                )
                os.close(read_fd); read_fd = None
                stdout, stderr = process.communicate(timeout=5)
                self.assertEqual(process.returncode, 7, stderr)
                self.assertEqual(stdout, b'')
                self.assertTrue(marker.exists())
                self.wait_for(lambda: not self.running(int(marker.read_text())))
            finally:
                if read_fd is not None: os.close(read_fd)
                os.close(write_fd)
                if process is not None:
                    process_runtime.reap_tree(process)
                    process.stdout.close(); process.stderr.close()

    def test_owner_loss_kills_direct_child_and_its_descendant(self):
        with tempfile.TemporaryDirectory() as temporary:
            marker = Path(temporary) / "descendant.pid"
            read_fd, write_fd = os.pipe()
            process = None
            try:
                code = (
                    "import pathlib,subprocess,sys,time; "
                    "child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)']); "
                    f"pathlib.Path({str(marker)!r}).write_text(str(child.pid)); time.sleep(60)"
                )
                process = process_runtime.launch_guarded(
                    [sys.executable, "-c", code], read_fd, stdin=subprocess.DEVNULL,
                    stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                )
                os.close(read_fd)
                read_fd = None
                self.wait_for(marker.exists)
                descendant = int(marker.read_text())
                os.close(write_fd)
                write_fd = None
                process.wait(timeout=5)
                self.wait_for(lambda: not self.running(descendant))
            finally:
                if read_fd is not None:
                    os.close(read_fd)
                if write_fd is not None:
                    os.close(write_fd)
                if process is not None:
                    if process.poll() is None:
                        process_runtime.signal_tree(process, force=True)
                    process.wait(timeout=5)
                    process.stderr.close()


if __name__ == "__main__":
    unittest.main()
