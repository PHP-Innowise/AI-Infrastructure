"""Keep a native process tree tied to the lifetime of its Harness owner.

Launch this file in a new session and pass only the watchdog pipe's read end:
    python process_guard.py READ_FD -- NATIVE_EXECUTABLE [ARGS...]
The owner holds the write end open. Native stdin/stdout/stderr pass through.
"""
from __future__ import annotations

import os
import ctypes
import signal
import stat
import subprocess
import sys
import threading
import time

TERMINATION_GRACE = 1.0
WINDOWS = os.name == 'nt'


def _windows_job() -> int:
    """Put this guard in a kill-on-close Job Object before it creates a child."""
    from ctypes import wintypes

    class Basic(ctypes.Structure):
        _fields_ = [('PerProcessUserTimeLimit', ctypes.c_longlong),
                   ('PerJobUserTimeLimit', ctypes.c_longlong),
                   ('LimitFlags', wintypes.DWORD), ('MinimumWorkingSetSize', ctypes.c_size_t),
                   ('MaximumWorkingSetSize', ctypes.c_size_t), ('ActiveProcessLimit', wintypes.DWORD),
                   ('Affinity', ctypes.c_size_t), ('PriorityClass', wintypes.DWORD),
                   ('SchedulingClass', wintypes.DWORD)]

    class Counters(ctypes.Structure):
        _fields_ = [(name, ctypes.c_ulonglong) for name in ('ReadOperationCount', 'WriteOperationCount',
                   'OtherOperationCount', 'ReadTransferCount', 'WriteTransferCount', 'OtherTransferCount')]

    class Extended(ctypes.Structure):
        _fields_ = [('BasicLimitInformation', Basic), ('IoInfo', Counters),
                   ('ProcessMemoryLimit', ctypes.c_size_t), ('JobMemoryLimit', ctypes.c_size_t),
                   ('PeakProcessMemoryUsed', ctypes.c_size_t), ('PeakJobMemoryUsed', ctypes.c_size_t)]

    kernel32 = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel32.CreateJobObjectW.argtypes = (ctypes.c_void_p, wintypes.LPCWSTR)
    kernel32.CreateJobObjectW.restype = wintypes.HANDLE
    kernel32.GetCurrentProcess.restype = wintypes.HANDLE
    kernel32.SetInformationJobObject.argtypes = (wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD)
    kernel32.SetInformationJobObject.restype = wintypes.BOOL
    kernel32.AssignProcessToJobObject.argtypes = (wintypes.HANDLE, wintypes.HANDLE)
    kernel32.AssignProcessToJobObject.restype = wintypes.BOOL
    kernel32.CloseHandle.argtypes = (wintypes.HANDLE,)
    kernel32.CloseHandle.restype = wintypes.BOOL
    job = kernel32.CreateJobObjectW(None, None)
    if not job:
        raise ctypes.WinError(ctypes.get_last_error())
    info = Extended()
    info.BasicLimitInformation.LimitFlags = 0x00002000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
    if not kernel32.SetInformationJobObject(job, 9, ctypes.byref(info), ctypes.sizeof(info)):
        error = ctypes.WinError(ctypes.get_last_error()); kernel32.CloseHandle(job); raise error
    if not kernel32.AssignProcessToJobObject(job, kernel32.GetCurrentProcess()):
        error = ctypes.WinError(ctypes.get_last_error()); kernel32.CloseHandle(job); raise error
    # Do not close this handle explicitly.  At normal process exit its final
    # close kills lingering descendants while retaining the native exit code.
    return int(job)


def _watchdog_eof(fd: int, stopped: threading.Event) -> threading.Thread:
    def wait() -> None:
        try:
            while os.read(fd, 4096):
                pass
        except OSError:
            pass
        stopped.set()
    thread = threading.Thread(target=wait, daemon=True)
    thread.start()
    return thread


def _windows_child(command: list[str], lock_fd: int | None, original_lock_fd: int | None) -> subprocess.Popen:
    """Pass a retained lock to the nested Fleet runner without broad inheritance."""
    from windows_commands import command_argv
    command = command_argv(command)
    if lock_fd is None:
        return subprocess.Popen(command, close_fds=True)
    import msvcrt
    from ctypes import wintypes

    kernel32 = ctypes.WinDLL('kernel32', use_last_error=True)
    duplicate = kernel32.DuplicateHandle
    duplicate.argtypes = (wintypes.HANDLE, wintypes.HANDLE, wintypes.HANDLE,
                          ctypes.POINTER(wintypes.HANDLE), wintypes.DWORD,
                          wintypes.BOOL, wintypes.DWORD)
    duplicate.restype = wintypes.BOOL
    close = kernel32.CloseHandle
    close.argtypes = (wintypes.HANDLE,); close.restype = wintypes.BOOL
    kernel32.GetCurrentProcess.restype = wintypes.HANDLE
    current = kernel32.GetCurrentProcess()
    handle = wintypes.HANDLE()
    if not duplicate(current, msvcrt.get_osfhandle(lock_fd), current, ctypes.byref(handle),
                     0, True, 0x00000002):
        raise ctypes.WinError(ctypes.get_last_error())
    rewritten = [*command]
    for index, value in enumerate(command[:-1]):
        if value == '--runner-lock' and command[index + 1] == str(original_lock_fd):
            rewritten[index:index + 2] = ['--runner-lock-handle', str(int(handle.value))]
            break
    startup = subprocess.STARTUPINFO()
    startup.lpAttributeList = {'handle_list': [int(handle.value)]}
    try:
        return subprocess.Popen(rewritten, close_fds=True, startupinfo=startup)
    finally:
        close(handle)


def _run_windows(watchdog_fd: int, command: list[str], lock_fd: int | None = None,
                 original_lock_fd: int | None = None) -> int:
    if watchdog_fd < 3 or not command:
        raise ValueError('A watchdog handle and native executable are required.')
    _windows_job()  # Fail closed before an unguarded native process can start.
    stopped = threading.Event()
    _watchdog_eof(watchdog_fd, stopped)
    try:
        child = _windows_child(command, lock_fd, original_lock_fd)
        while True:
            if stopped.wait(.05):
                # Returning exits this guard; its final Job Object handle close
                # terminates the direct child and every descendant as one tree.
                return 1
            code = child.poll()
            if code is not None:
                return code if code >= 0 else 128 - code
    finally:
        os.close(watchdog_fd)
        # Retain the share-deny lock until process teardown kills the Job tree.


def run(watchdog_fd: int, command: list[str], lock_fd: int | None = None,
        original_lock_fd: int | None = None) -> int:
    """Return the native exit status, or terminate this group after owner loss."""
    if WINDOWS:
        # Job membership is inherited by all descendants and persists until the
        # guard exits, so a POSIX inherited advisory lock is not needed here.
        return _run_windows(watchdog_fd, command, lock_fd, original_lock_fd)
    if os.getpgrp() != os.getpid():
        raise ValueError("The process guard must be started in its own session.")
    if watchdog_fd < 3 or not stat.S_ISFIFO(os.fstat(watchdog_fd).st_mode):
        raise ValueError("The watchdog must be a separate pipe descriptor.")
    if not command:
        raise ValueError("A native executable is required.")
    if lock_fd is not None and (lock_fd < 3 or lock_fd == watchdog_fd or not stat.S_ISREG(os.fstat(lock_fd).st_mode)):
        raise ValueError('The inherited runner lock must be a regular file descriptor.')
    os.set_blocking(watchdog_fd, False)
    os.set_inheritable(watchdog_fd, False)
    stopping = False

    def request_stop(*_):
        nonlocal stopping
        stopping = True

    signal.signal(signal.SIGTERM, request_stop)
    signal.signal(signal.SIGINT, request_stop)
    import selectors
    selector = selectors.DefaultSelector()
    selector.register(watchdog_fd, selectors.EVENT_READ)
    try:
        # No new group and no pass_fds: the child shares our group, but neither
        # watchdog end. Caught signal handlers reset to defaults during exec.
        child = subprocess.Popen(command, close_fds=True, pass_fds=() if lock_fd is None else (lock_fd,))
        deadline = None
        while True:
            for _, _ in selector.select(0.05):
                try:
                    if not os.read(watchdog_fd, 4096):
                        stopping = True
                        selector.unregister(watchdog_fd)
                except BlockingIOError:
                    pass
            if stopping:
                if deadline is None:
                    deadline = time.monotonic() + TERMINATION_GRACE
                    os.killpg(os.getpid(), signal.SIGTERM)
                child.poll()  # Reap a cooperative direct child before exiting.
                if time.monotonic() >= deadline:
                    # Do not return early when the direct child exits: a stubborn
                    # descendant may still be running in this process group.
                    os.killpg(os.getpid(), signal.SIGKILL)
            else:
                code = child.poll()
                if code is not None:
                    return code if code >= 0 else 128 - code
    finally:
        selector.close()
        os.close(watchdog_fd)


def main(argv=None) -> int:
    arguments = sys.argv[1:] if argv is None else argv
    try:
        if WINDOWS and len(arguments) >= 4 and arguments[0] == '--watchdog-handle':
            import msvcrt
            watchdog = msvcrt.open_osfhandle(int(arguments[1]), os.O_RDONLY)
            position, lock, original = 2, None, None
            if (len(arguments) >= 6 and arguments[position] == '--lock-handle'
                    and arguments[position + 2] == '--lock-fd'):
                lock = msvcrt.open_osfhandle(int(arguments[position + 1]), os.O_RDWR)
                original = int(arguments[position + 3])
                position += 4
            if len(arguments) <= position or arguments[position] != '--':
                os.close(watchdog)
                if lock is not None: os.close(lock)
                raise ValueError('Expected a watchdog handle, --, and a native command.')
            # Ownership transfers to these CRT descriptors and is closed by run.
            return run(watchdog, arguments[position + 1:], lock, original)
        lock_fd = None
        if len(arguments) >= 5 and arguments[1] == '--lock-fd':
            lock_fd = int(arguments[2])
            arguments = [arguments[0], *arguments[3:]]
        if len(arguments) < 3 or arguments[1] != "--":
            raise ValueError("Expected a watchdog descriptor, --, and a native command.")
        return run(int(arguments[0]), arguments[2:], lock_fd)
    except (OSError, ValueError):
        # Executable arguments and environment errors may contain private data.
        print("Harness process guard could not start the native process.", file=sys.stderr)
        return 127


if __name__ == "__main__":
    sys.exit(main())
