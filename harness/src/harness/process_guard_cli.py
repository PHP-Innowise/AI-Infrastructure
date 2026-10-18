"""Run process_guard.py from its cached bytecode.

Every guarded call waits for the guard to start, and Python compiles a file
run as a script from source each time: for process_guard.py, its Windows code
included, that was ~2 ms of every call. Started through this file it is
exactly the same script - `__main__`, the same argv, `sys.argv[0]` naming
process_guard.py, its folder first on sys.path (where the Windows guard finds
windows_commands), the same exit - compiled once and read from __pycache__
after that. process_runtime.launch_guarded starts this file on POSIX and on
Windows.

As context_cli.py does for the memory runtime, the cache is written even when
the caller asked Python for none (PYTHONDONTWRITEBYTECODE or -B): the Harness
sets it for its memory-runtime, setup and result calls, and Claude Desktop
for the tools it starts. Only process_guard.py's own cache is written; what
the guard imports keeps the caller's setting. This file imports nothing that
Python has not already loaded at startup.
"""
import sys

try:
    # importlib.machinery's SourceFileLoader, from the module Python loaded at
    # startup: importing importlib for it was 0.1-0.5 ms of every call.
    from _frozen_importlib_external import SourceFileLoader
except ImportError:
    from importlib.machinery import SourceFileLoader


class CachingLoader(SourceFileLoader):
    def get_code(self, fullname):
        # Only reading and caching this one file; its code runs after this.
        saved, sys.dont_write_bytecode = sys.dont_write_bytecode, False
        try:
            return super().get_code(fullname)
        finally:
            sys.dont_write_bytecode = saved


if __name__ == "__main__":
    # The guard beside this file, found without os.path (the POSIX guard does
    # without os.py). A Windows path may end its folder with either slash; a
    # POSIX folder name may hold a backslash, but before the last slash.
    guard = __file__[:max(__file__.rfind("/"), __file__.rfind("\\")) + 1] + "process_guard.py"
    sys.argv[0] = guard
    exec(CachingLoader("__main__", guard).get_code("__main__"), {"__name__": "__main__", "__file__": guard})
