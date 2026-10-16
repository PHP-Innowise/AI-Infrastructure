"""Run context.py's command line from its cached bytecode.

Python compiles a file run as a script from source on every start, and
context.py is large enough that this was an eighth of every prompt hook.
Run as a module it is exactly the same script - `__main__`, the same argv,
`sys.argv[0]` naming context.py - compiled once and read from __pycache__
after that. The hooks call this file; people and agents call context.py
directly, with the same result.

The cache is written even when the caller asked Python for no bytecode
(PYTHONDONTWRITEBYTECODE or -B). Claude Desktop starts Claude Code that way
and its hooks inherit it, and the Harness sets it for every runtime call, so
the whole runtime was compiled again on each of them. The exception covers
this directory's own modules only - its __pycache__/ is ignored wherever the
runtime is installed - and every other import keeps the caller's setting.
"""
import importlib.machinery
import os
import runpy
import sys

RUNTIME = os.path.normcase(os.path.realpath(os.path.dirname(os.path.abspath(__file__))))


def in_runtime(entry):
    try:
        return os.path.normcase(os.path.realpath(entry or os.curdir)) == RUNTIME
    except (TypeError, ValueError, OSError):
        return False


class CachingLoader(importlib.machinery.SourceFileLoader):
    def get_code(self, fullname):
        # Read only where a module just compiled from source would be cached;
        # the module's own imports run later, under the caller's setting.
        saved, sys.dont_write_bytecode = sys.dont_write_bytecode, False
        try:
            return super().get_code(fullname)
        finally:
            sys.dont_write_bytecode = saved


def runtime_finder(entry):
    """Python's own finder for this directory, however sys.path spells it,
    with the source loader swapped; other entries go to the default hooks."""
    if not in_runtime(entry):
        raise ImportError("not the memory runtime directory")
    machinery = importlib.machinery
    return machinery.FileFinder(
        entry,
        (machinery.ExtensionFileLoader, machinery.EXTENSION_SUFFIXES),
        (CachingLoader, machinery.SOURCE_SUFFIXES),
        (machinery.SourcelessFileLoader, machinery.BYTECODE_SUFFIXES),
    )


if __name__ == "__main__":
    if sys.dont_write_bytecode:
        sys.path_hooks.insert(0, runtime_finder)
        for entry in [entry for entry in sys.path_importer_cache if in_runtime(entry)]:
            del sys.path_importer_cache[entry]
    runpy.run_module("context", run_name="__main__", alter_sys=True)
