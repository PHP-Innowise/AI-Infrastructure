"""Keep a native process group tied to the lifetime of its Harness owner.

Launch this file in a new session and pass only the watchdog pipe's read end:
    python process_guard.py READ_FD -- NATIVE_EXECUTABLE [ARGS...]
The owner holds the write end open. Native stdin/stdout/stderr pass through.
"""
from __future__ import annotations

import os
import selectors
import signal
import stat
import subprocess
import sys
import time

TERMINATION_GRACE = 1.0


def run(watchdog_fd: int, command: list[str], lock_fd: int | None = None) -> int:
    """Return the native exit status, or terminate this group after owner loss."""
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
