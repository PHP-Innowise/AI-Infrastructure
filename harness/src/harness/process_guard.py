"""Keep a native process tree tied to the lifetime of its Harness owner.

Launch this file in a new session and pass only the watchdog pipe's read end:
    python -S process_guard.py READ_FD -- NATIVE_EXECUTABLE [ARGS...]
The Harness starts process_guard_cli.py instead, which runs this file from
cached bytecode. The owner holds the write end open. Native
stdin/stdout/stderr pass through. Every guarded call waits for this
interpreter to start and to exit, so the POSIX path leaves out what only
Windows uses (ctypes, threads), starts its native process without subprocess
where posix_spawn can, runs without site, and imports neither os.py nor
signal's enums nor __future__ (annotations Python 3.9 cannot evaluate are
strings).
"""
import stat
import sys
import time

# signal's own functions and numbers, without the IntEnum wrappers signal.py
# adds; importing enum for them was ~1.6 ms of every guarded call.
import _signal as signal

TERMINATION_GRACE = 1.0
WINDOWS = sys.platform == 'win32'
if WINDOWS:
    import os
else:
    # The functions os takes from posix, without os.py and what it imports
    # (abc, _collections_abc, posixpath): ~1.3 ms of every guarded call.
    # This os has no os.path, and its environ holds bytes.
    import posix as os


def _windows_job() -> int:
    """Put this guard in a kill-on-close Job Object before it creates a child."""
    import ctypes
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


def _watchdog_eof(fd: int, stopped: 'threading.Event') -> 'threading.Thread':
    import threading

    def wait() -> None:
        try:
            while os.read(fd, 4096):
                pass
        except OSError:
            pass
        finally:
            try:
                os.close(fd)
            finally:
                stopped.set()
    thread = threading.Thread(target=wait, daemon=True)
    thread.start()
    return thread


def _windows_child(command: list[str], lock_fd: 'int | None', original_lock_fd: 'int | None') -> 'subprocess.Popen':
    """Pass a retained lock to the nested Fleet runner without broad inheritance."""
    import subprocess
    from windows_commands import command_argv
    command = command_argv(command)
    if lock_fd is None:
        return subprocess.Popen(command, close_fds=True)
    import ctypes
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


def _run_windows(watchdog_fd: int, command: list[str], lock_fd: 'int | None' = None,
                 original_lock_fd: 'int | None' = None) -> int:
    if watchdog_fd < 3 or not command:
        raise ValueError('A watchdog handle and native executable are required.')
    _windows_job()  # Fail closed before an unguarded native process can start.
    import threading
    stopped = threading.Event()
    # Windows CRT _close waits for a concurrent _read on the same descriptor.
    # Give the reader its own duplicate so normal child exit can close the
    # guard's descriptor even while the owner retains the watchdog writer.
    _watchdog_eof(os.dup(watchdog_fd), stopped)
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


class _Spawned:
    """A posix_spawn child with the part of Popen this guard uses."""

    def __init__(self, pid: int):
        self.pid = pid
        self.returncode = None

    def poll(self) -> 'int | None':
        if self.returncode is None:
            try:
                pid, status = os.waitpid(self.pid, os.WNOHANG)
            except ChildProcessError:
                # SIGCHLD was ignored when the child exited, so the kernel
                # reaped it; Popen.poll reports 0 for that too.
                self.returncode = 0
            else:
                if pid:
                    self.returncode = os.waitstatus_to_exitcode(status)
        return self.returncode


def _spawn_reports_exec_errors() -> bool:
    """Whether a failed exec raises from posix_spawn instead of exiting 127.

    subprocess trusts posix_spawn on the same platforms: macOS, where it is a
    system call, and Linux from glibc 2.24.
    """
    if not hasattr(os, 'posix_spawn'):
        return False
    if sys.platform == 'darwin':
        return True
    try:
        libc, version = os.confstr('CS_GNU_LIBC_VERSION').split()
        return sys.platform == 'linux' and libc == 'glibc' and tuple(map(int, version.split('.'))) >= (2, 24)
    except (AttributeError, ValueError, OSError):
        return False


def _start(command: list[str], lock_fd: 'int | None'):
    """Start *command* as Popen(command, close_fds=True, pass_fds=(lock_fd,)) does.

    Importing subprocess was about a third of this guard's start, which every
    guarded call waits for, so posix_spawn starts the child wherever a failed
    exec raises from it and the open descriptors can be listed: Linux with
    glibc 2.24+ and /proc, and macOS. Elsewhere Popen does.
    """
    descriptors = None
    if _spawn_reports_exec_errors():
        try:
            descriptors = os.listdir('/dev/fd' if sys.platform == 'darwin' else '/proc/self/fd')
        except OSError:
            pass
    if descriptors is None:
        import subprocess
        return subprocess.Popen(command, close_fds=True, pass_fds=() if lock_fd is None else (lock_fd,))
    # close_fds: posix_spawn closes only the descriptors marked close-on-exec.
    for descriptor in map(int, descriptors):
        if descriptor > 2 and descriptor != lock_fd:
            try:
                os.set_inheritable(descriptor, False)
            except OSError:
                pass  # The listing's own descriptor, closed by now.
    if lock_fd is not None:
        os.set_inheritable(lock_fd, True)  # pass_fds
    # restore_signals: what Python ignores, the native process gets at default.
    defaults = [getattr(signal, name) for name in ('SIGPIPE', 'SIGXFZ', 'SIGXFSZ') if hasattr(signal, name)]
    # Popen's execv hands the child this process's environment. posix_spawn
    # does that for None from Python 3.13; before, subprocess passed
    # os.environ, which decodes the very posix.environ passed here.
    environment = None if sys.version_info >= (3, 13) else os.environ
    executable = command[0]
    if '/' in executable:  # os.path.dirname(executable) is not empty.
        candidates = [executable]
    else:
        # Popen's PATH search: every entry in order, past each one it cannot
        # run, never through a shell. posix_spawnp stops at the first such
        # entry on glibc and hands a file without #! to /bin/sh on macOS.
        # What stat cannot find, exec cannot run: skipping it costs ~1 us,
        # a refused spawn ~0.1 ms. In bytes, as Popen searches: os.defpath
        # for an unset PATH, each entry joined as os.path.join joins it.
        name = executable.encode(sys.getfilesystemencoding(), sys.getfilesystemencodeerrors())
        candidates = []
        for directory in os.environ.get(b'PATH', b'/bin:/usr/bin').split(b':'):
            candidate = directory + name if directory[-1:] in (b'', b'/') else directory + b'/' + name
            try:
                os.stat(candidate)
            except (OSError, ValueError):  # os.path.exists
                continue
            candidates.append(candidate)
    failure: OSError = FileNotFoundError()
    for candidate in candidates:
        try:
            return _Spawned(os.posix_spawn(candidate, command, environment, setsigdef=defaults))
        except OSError as error:
            failure = error
    raise failure


def run(watchdog_fd: int, command: list[str], lock_fd: 'int | None' = None,
        original_lock_fd: 'int | None' = None) -> int:
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

    # Wait for events, never for a tick: the owner reads EOF only once this
    # guard exits, so a poll interval delayed every native call. A handled
    # signal does not end poll (PEP 475 resumes it), so the interpreter also
    # writes each caught signal's number to this pipe.
    wake_read, wake_write = os.pipe()
    os.set_blocking(wake_read, False)
    os.set_blocking(wake_write, False)
    signal.set_wakeup_fd(wake_write, warn_on_full_buffer=False)
    signal.signal(signal.SIGTERM, request_stop)
    signal.signal(signal.SIGINT, request_stop)
    import select
    poller = select.poll()
    poller.register(watchdog_fd, select.POLLIN)
    poller.register(wake_read, select.POLLIN)
    exited = None
    try:
        # No new group: the child shares ours, and of our descriptors gets only
        # stdin/stdout/stderr and the lock. Caught signal handlers reset to
        # defaults during exec.
        child = _start(command, lock_fd)
        try:
            exited = os.pidfd_open(child.pid)  # Linux 5.3+: readable once the child exits.
            poller.register(exited, select.POLLIN)
        except (AttributeError, OSError):
            # macOS and older kernels: SIGCHLD reaches the wakeup pipe instead.
            # The first poll() below sees an exit that preceded this handler.
            signal.signal(signal.SIGCHLD, lambda *_: None)
            signal.pthread_sigmask(signal.SIG_UNBLOCK, {signal.SIGCHLD})
        deadline = None
        while True:
            timeout = None
            if stopping:
                if deadline is None:
                    deadline = time.monotonic() + TERMINATION_GRACE
                    os.killpg(os.getpid(), signal.SIGTERM)
                child.poll()  # Reap a cooperative direct child before exiting.
                timeout = deadline - time.monotonic()
                if timeout <= 0:
                    # Do not return early when the direct child exits: a stubborn
                    # descendant may still be running in this process group.
                    os.killpg(os.getpid(), signal.SIGKILL)
            else:
                code = child.poll()
                if code is not None:
                    return code if code >= 0 else 128 - code
            # Milliseconds; poll would wait forever for a negative timeout.
            for descriptor, _ in poller.poll(None if timeout is None else max(timeout, 0) * 1000):
                if descriptor == exited:
                    if child.poll() is not None:
                        poller.unregister(exited)  # It stays readable after the exit.
                    continue
                try:
                    data = os.read(descriptor, 4096)
                except BlockingIOError:
                    continue
                if descriptor == wake_read:
                    stopping = stopping or signal.SIGTERM in data or signal.SIGINT in data
                elif not data:
                    stopping = True
                    poller.unregister(watchdog_fd)
    finally:
        signal.set_wakeup_fd(-1)
        for descriptor in (wake_read, wake_write, exited, watchdog_fd):
            if descriptor is not None:
                os.close(descriptor)


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
    status = main()
    if WINDOWS:
        sys.exit(status)  # Normal exit makes the final Job Object handle close.
    # The owner reads EOF only once this process is gone, and interpreter
    # finalization delayed that by ~3 ms; nothing is left to clean up.
    for stream in (sys.stdout, sys.stderr):
        if stream is not None:
            stream.flush()
    os._exit(status)
