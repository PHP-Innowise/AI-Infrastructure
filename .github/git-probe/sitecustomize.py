"""Temporary instrumentation of the memory runtime's Git source probe.

Loaded into every context.py process the probe loop starts (PYTHONPATH names
this directory). It changes no decision the runtime makes: it times every Git
process, records each Git frame the runtime asks for, and, where a
SourceGitError is constructed, the line that raised it and the exception it
was raised from. One JSON file per process, at exit, in PROBE_TRACE_DIR.
"""
import os
import sys

_DIRECTORY = os.environ.get("PROBE_TRACE_DIR")
_CALL = os.environ.get("PROBE_CALL_ID")

if _DIRECTORY and _CALL:
    import atexit
    import json
    import subprocess
    import threading
    import time
    import traceback

    _T0 = time.perf_counter()
    _WALL0 = time.time()
    _EVENTS = []
    _LOCAL = threading.local()

    def _now():
        return round(time.perf_counter() - _T0, 6)

    def _text(value, limit=400):
        if value is None:
            return None
        if isinstance(value, bytes):
            value = value.decode("utf-8", "replace")
        return value[:limit]

    def _git_arguments(argv):
        # Everything after `-C <project>`: the subcommand and its options.
        items = [str(item) for item in argv]
        if "-C" in items:
            items = items[items.index("-C") + 2:]
        else:
            items = items[1:]
        return [item if len(item) < 80 else item[:30] + "..." + item[-30:] for item in items]

    _real_run = subprocess.run
    _real_execute_child = subprocess.Popen._execute_child

    def _execute_child(self, *arguments, **keywords):
        started = time.perf_counter()
        try:
            return _real_execute_child(self, *arguments, **keywords)
        finally:
            record = getattr(_LOCAL, "record", None)
            if record is not None:
                record["spawn"] = round(time.perf_counter() - started, 6)
                record["pid"] = getattr(self, "pid", None)

    def run(*arguments, **keywords):
        argv = arguments[0] if arguments else keywords.get("args")
        name = os.path.basename(str(argv[0])).lower() if isinstance(argv, (list, tuple)) and argv else ""
        if name not in ("git", "git.exe"):
            return _real_run(*arguments, **keywords)
        record = {"k": "git", "t": _now(), "wall": time.time(), "argv": _git_arguments(argv),
                  "timeout": keywords.get("timeout")}
        quiet = keywords.get("stderr") is subprocess.DEVNULL
        if quiet:
            keywords["stderr"] = subprocess.PIPE
        _LOCAL.record = record
        started = time.perf_counter()
        try:
            result = _real_run(*arguments, **keywords)
        except BaseException as error:
            record["dur"] = round(time.perf_counter() - started, 6)
            record["err"] = repr(error)[:300]
            if isinstance(error, subprocess.TimeoutExpired):
                record["stderr"] = _text(error.stderr)
            _EVENTS.append(record)
            raise
        finally:
            _LOCAL.record = None
        record["dur"] = round(time.perf_counter() - started, 6)
        record["rc"] = result.returncode
        if isinstance(result.stdout, (bytes, str)):
            record["out_len"] = len(result.stdout)
            if "rev-parse" in record["argv"] or result.returncode not in (0, 1):
                record["out"] = _text(result.stdout, 600)
        record["stderr"] = _text(result.stderr)
        if quiet:
            result.stderr = None
        _EVENTS.append(record)
        return result

    subprocess.Popen._execute_child = _execute_child
    subprocess.run = run

    def _site():
        frames = traceback.extract_stack()[:-2]
        return [(os.path.basename(frame.filename), frame.lineno, frame.name)
                for frame in frames[-10:] if not frame.filename.endswith("sitecustomize.py")]

    def _instrument(module):
        error_class = module.SourceGitError
        original_init = error_class.__init__

        def __init__(self, *arguments, **keywords):
            try:
                cause = sys.exc_info()[1]
                _EVENTS.append({"k": "raise", "t": _now(), "message": _text(str(arguments[0]) if arguments else ""),
                                "site": _site(), "cause": repr(cause)[:400] if cause is not None else None,
                                "cause_type": type(cause).__name__ if cause is not None else None})
            except Exception:
                pass
            original_init(self, *arguments, **keywords)

        error_class.__init__ = __init__

        def wrap(name, describe):
            original = getattr(module, name)

            def wrapper(*arguments, **keywords):
                record = {"k": name, "t": _now()}
                started = time.perf_counter()
                try:
                    value = original(*arguments, **keywords)
                except BaseException as error:
                    record["dur"] = round(time.perf_counter() - started, 6)
                    record["err"] = type(error).__name__
                    _EVENTS.append(record)
                    raise
                record["dur"] = round(time.perf_counter() - started, 6)
                try:
                    record["value"] = describe(arguments, keywords, value)
                except Exception as problem:
                    record["value"] = "describe failed: " + repr(problem)[:100]
                _EVENTS.append(record)
                return value

            setattr(module, name, wrapper)

        last_identity = [None]

        def identity(arguments, keywords, value):
            # In full only when it differs from the previous one in this process.
            text = None if value is None else repr(value)[:1500]
            if text is not None and text == last_identity[0]:
                return "same"
            last_identity[0] = text
            return text

        wrap("_source_git_frame", lambda a, k, v: None if v is None else [str(v[0]), v[1], str(v[2])])
        wrap("_source_git_ignored", lambda a, k, v: {"paths": len(a[1]) if len(a) > 1 else None,
                                                     "timeout": a[2] if len(a) > 2 else k.get("timeout"),
                                                     "ignored": len(v)})
        wrap("_frame_identity", identity)
        wrap("_source_git_checkout", lambda a, k, v: None if v is None else str(v))

    import importlib.abc
    import importlib.machinery

    class _Finder(importlib.abc.MetaPathFinder):
        def find_spec(self, name, path, target=None):
            if name != "workspace_roots":
                return None
            spec = importlib.machinery.PathFinder.find_spec(name, path)
            if spec is None or spec.loader is None:
                return spec
            real_exec = spec.loader.exec_module

            def exec_module(module):
                real_exec(module)
                try:
                    _instrument(module)
                except Exception as problem:
                    _EVENTS.append({"k": "instrument-failed", "error": repr(problem)[:300]})

            spec.loader.exec_module = exec_module
            return spec

    sys.meta_path.insert(0, _Finder())

    def _write():
        try:
            # A unittest run starts many processes with one call ID.
            path = os.path.join(_DIRECTORY, "%s-%d-%d.json" % (_CALL, os.getpid(), time.perf_counter_ns()))
            with open(path, "w", encoding="utf-8") as handle:
                json.dump({"call": _CALL, "pid": os.getpid(), "wall0": _WALL0, "end": _now(),
                           "argv": sys.argv[1:6], "events": _EVENTS}, handle)
        except Exception:
            pass

    atexit.register(_write)
