"""Portable ownership and stream helpers for Harness child processes.

The POSIX implementation deliberately keeps the existing process-session and
``pass_fds`` contract.  Windows starts the same guard in a new console process
group and passes exactly one duplicated watchdog handle through the documented
``STARTUPINFO`` handle list.  The guard owns the Windows Job Object; callers
must therefore always close the returned watchdog writer when work ends.
"""
from __future__ import annotations

from dataclasses import dataclass
import os
from pathlib import Path
import queue
import subprocess
import sys
import threading
import time
from typing import Any, BinaryIO, Iterable


WINDOWS = os.name == "nt"
GUARD = Path(__file__).with_name("process_guard.py")
_HANDLE_LAUNCH_LOCK = threading.Lock()


def daemon_kwargs() -> dict[str, Any]:
    """Return options for the detached browser-server daemon."""
    if WINDOWS:
        return {"creationflags": subprocess.DETACHED_PROCESS | subprocess.CREATE_NEW_PROCESS_GROUP}
    return {"start_new_session": True}


def guard_kwargs() -> dict[str, Any]:
    """Return options for a foreground guard owned by a Harness worker."""
    if WINDOWS:
        return {"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP}
    return {"start_new_session": True}


def signal_tree(process: subprocess.Popen[bytes], force: bool = False) -> None:
    """Stop a guard and everything it owns without touching an unrelated group."""
    try:
        if WINDOWS:
            # The guard is the only owner of its Job Object.  Ending it closes
            # the last job handle, which terminates all native descendants.
            (process.kill if force else process.terminate)()
        else:
            import signal
            os.killpg(process.pid, signal.SIGKILL if force else signal.SIGTERM)
    except OSError:
        pass


def reap_tree(process, timeout=3, *, owner=None):
    """Bound cleanup without masking the caller's original failure.

    Return False if the OS could not confirm termination; callers still release
    their bookkeeping, while the guard retains its admission lock.
    """
    try:
        signal_tree(process, force=True)
        process.wait(timeout=timeout)
        return True
    except (OSError, subprocess.TimeoutExpired):
        if owner is not None:
            owner.stopping.set()
        return False


def _duplicate_inheritable_handle(fd: int) -> int:
    """Duplicate *fd* as a narrowly inherited Windows HANDLE.

    ``close_fds=False`` would let an agent inherit unrelated process handles.
    A duplicate also avoids toggling inheritable state on the owner's original
    watchdog descriptor while another Harness thread may launch a process.
    """
    if not WINDOWS:
        raise OSError("Windows handles are unavailable on this platform.")
    import ctypes
    import msvcrt
    from ctypes import wintypes

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.GetCurrentProcess.restype = wintypes.HANDLE
    duplicate = kernel32.DuplicateHandle
    duplicate.argtypes = (wintypes.HANDLE, wintypes.HANDLE, wintypes.HANDLE,
                          ctypes.POINTER(wintypes.HANDLE), wintypes.DWORD,
                          wintypes.BOOL, wintypes.DWORD)
    duplicate.restype = wintypes.BOOL
    current = kernel32.GetCurrentProcess()
    output = wintypes.HANDLE()
    # DUPLICATE_SAME_ACCESS; bInheritHandle is deliberately true for the one
    # handle named in STARTUPINFO.lpAttributeList below.
    if not duplicate(current, msvcrt.get_osfhandle(fd), current, ctypes.byref(output),
                     0, True, 0x00000002):
        raise ctypes.WinError(ctypes.get_last_error())
    return int(output.value)


def _close_handle(handle: int) -> None:
    import ctypes
    from ctypes import wintypes
    close = ctypes.WinDLL("kernel32", use_last_error=True).CloseHandle
    close.argtypes = (wintypes.HANDLE,)
    close.restype = wintypes.BOOL
    if not close(handle):
        raise ctypes.WinError(ctypes.get_last_error())


def _windows_guard_popen(command: list[str], watchdog_fd: int, lock_fd: int | None,
                         options: dict[str, Any]) -> subprocess.Popen[bytes]:
    """Launch a guard while inheriting only its duplicated watchdog handle."""
    with _HANDLE_LAUNCH_LOCK:
        handles: list[int] = []
        try:
            handles.append(_duplicate_inheritable_handle(watchdog_fd))
            if lock_fd is not None:
                handles.append(_duplicate_inheritable_handle(lock_fd))
            startup = subprocess.STARTUPINFO()
            startup.lpAttributeList = {"handle_list": handles}
            creationflags = options.pop("creationflags", 0) | subprocess.CREATE_NEW_PROCESS_GROUP
            arguments = [sys.executable, str(GUARD), "--watchdog-handle", str(handles[0])]
            if lock_fd is not None:
                arguments.extend(["--lock-handle", str(handles[1]), "--lock-fd", str(lock_fd)])
            arguments.extend(["--", *command])
            return subprocess.Popen(arguments,
                                    close_fds=True, startupinfo=startup,
                                    creationflags=creationflags, **options)
        finally:
            for handle in handles:
                _close_handle(handle)


def launch_guarded(command: list[str], watchdog_fd: int, *, lock_fd: int | None = None,
                   **popen_options: Any) -> subprocess.Popen[bytes]:
    """Start ``process_guard`` around *command*.

    On POSIX the optional lock remains inherited by the native process, which
    preserves restart admission after a crashed owner. Windows inherits narrow
    duplicate watchdog/lock handles and uses a kill-on-close Job Object.
    """
    if not command:
        raise ValueError("A native executable is required.")
    if WINDOWS:
        return _windows_guard_popen(command, watchdog_fd, lock_fd,
                                    {**guard_kwargs(), **popen_options})
    arguments = [sys.executable, str(GUARD), str(watchdog_fd)]
    if lock_fd is not None:
        arguments.extend(["--lock-fd", str(lock_fd)])
    arguments.extend(["--", *command])
    options = {**guard_kwargs(), **popen_options}
    options["pass_fds"] = (watchdog_fd,) if lock_fd is None else (watchdog_fd, lock_fd)
    return subprocess.Popen(arguments, **options)


@dataclass(frozen=True)
class PipeChunk:
    """One bounded chunk from a registered child output stream."""
    data: Any
    body: bytes


class PipeReaders:
    """Read subprocess pipes portably, with one queued chunk per stream.

    Windows ``selectors`` cannot wait for anonymous pipes.  Reader threads are
    also safe on POSIX and bound memory: a noisy stream blocks its own reader
    after one pending chunk instead of accumulating output while consumers are
    busy with another stream.
    """
    def __init__(self, chunk_size: int = 65536):
        self.chunk_size = chunk_size
        self._closed = threading.Event()
        self._ready = threading.Event()
        self._readers: list[tuple[BinaryIO, Any, queue.Queue[bytes], threading.Thread]] = []

    def register(self, stream: BinaryIO, data: Any = None) -> None:
        pending: queue.Queue[bytes] = queue.Queue(maxsize=1)

        def read_stream() -> None:
            try:
                while not self._closed.is_set():
                    body = os.read(stream.fileno(), self.chunk_size)
                    while not self._closed.is_set():
                        try:
                            pending.put(body, timeout=.05)
                            self._ready.set()
                            break
                        except queue.Full:
                            continue
                    if not body:
                        return
            except (OSError, ValueError):
                # Deliver terminal readiness even if an OS read failed. A
                # consumer must not wait forever for an exited reader thread.
                while not self._closed.is_set():
                    try:
                        pending.put(b"", timeout=.05)
                        self._ready.set()
                        return
                    except queue.Full:
                        continue

        thread = threading.Thread(target=read_stream, daemon=True)
        self._readers.append((stream, data, pending, thread))
        thread.start()

    def read(self, timeout: float | None = None) -> PipeChunk | None:
        deadline = None if timeout is None else time.monotonic() + max(0, timeout)
        while True:
            for _, data, pending, _ in self._readers:
                try:
                    body = pending.get_nowait()
                except queue.Empty:
                    continue
                if not any(not item[2].empty() for item in self._readers):
                    self._ready.clear()
                return PipeChunk(data, body)
            if deadline is not None and time.monotonic() >= deadline:
                return None
            remaining = None if deadline is None else max(0, deadline - time.monotonic())
            self._ready.wait(.02 if remaining is None else min(.02, remaining))
            self._ready.clear()

    def close(self) -> None:
        self._closed.set()
        for stream, _, _, _ in self._readers:
            try:
                stream.close()
            except OSError:
                pass
        for _, _, _, thread in self._readers:
            thread.join(timeout=.2)


@dataclass(frozen=True)
class _PipeKey:
    fileobj: BinaryIO
    data: Any


class PipeSelector:
    """``selectors``-shaped subprocess-pipe reader usable on Windows.

    Callers retain their normal ``register/select/unregister/get_map`` control
    flow and replace direct ``os.read`` calls with :meth:`read`.  Windows uses
    :class:`PipeReaders`; POSIX delegates readiness detection to ``selectors``.
    """
    def __init__(self):
        self._windows = WINDOWS
        self._streams: dict[BinaryIO, Any] = {}
        if self._windows:
            self._readers = PipeReaders()
            self._pending: dict[BinaryIO, bytes] = {}
            self._eof: set[BinaryIO] = set()
        else:
            import selectors
            self._selector = selectors.DefaultSelector()

    def register(self, stream: BinaryIO, events: int, data: Any = None) -> _PipeKey:
        self._streams[stream] = data
        if self._windows:
            self._readers.register(stream, (stream, data))
            return _PipeKey(stream, data)
        return self._selector.register(stream, events, data)

    def unregister(self, stream: BinaryIO) -> None:
        self._streams.pop(stream, None)
        if self._windows:
            self._eof.discard(stream)
            self._pending.pop(stream, None)
        if not self._windows:
            self._selector.unregister(stream)

    def select(self, timeout: float | None = None):
        if not self._windows:
            return self._selector.select(timeout)
        if self._eof:
            return [(_PipeKey(stream, self._streams[stream]), 1) for stream in self._eof]
        event = self._readers.read(timeout)
        if event is None:
            return []
        stream, data = event.data
        if stream not in self._streams:
            return []
        if not event.body:
            self._eof.add(stream)
        self._pending[stream] = event.body
        return [(_PipeKey(stream, data), 1)]

    def read(self, stream: BinaryIO, size: int) -> bytes:
        if not self._windows:
            return os.read(stream.fileno(), size)
        return self._pending.pop(stream, b"")

    def get_map(self):
        return self._streams

    def close(self) -> None:
        if self._windows:
            self._readers.close()
        else:
            self._selector.close()
