"""Temporary diagnostics for the memory suites on native Windows.

Prints the platform facts the failing suites depend on. Not a test; removed
once the suites pass on windows-latest.
"""
import importlib
import json
import os
import platform
import shutil
import sqlite3
import stat
import subprocess
import sys
import tempfile
import time
import traceback
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EDITION = ROOT / "PHP Core"
FIELDS = ("st_mode", "st_ino", "st_dev", "st_nlink", "st_size", "st_mtime_ns", "st_ctime_ns",
          "st_birthtime_ns", "st_file_attributes", "st_reparse_tag")


def show(title, value=""):
    print(f"== {title}: {value}", flush=True)


def fields(image):
    return {key: getattr(image, key, None) for key in FIELDS}


def diff(left, right):
    a, b = fields(left), fields(right)
    return {key: (a[key], b[key]) for key in FIELDS if a[key] != b[key]}


def experiment(function):
    print(f"\n######## {function.__name__}", flush=True)
    try:
        function()
    except Exception:
        traceback.print_exc(file=sys.stdout)
    sys.stdout.flush()


def long_path(text):
    if os.name != "nt":
        return text
    import ctypes
    from ctypes import wintypes
    function = ctypes.windll.kernel32.GetLongPathNameW
    function.argtypes = [wintypes.LPCWSTR, wintypes.LPWSTR, wintypes.DWORD]
    function.restype = wintypes.DWORD
    buffer = ctypes.create_unicode_buffer(32768)
    count = function(text, buffer, 32768)
    return buffer.value if count else f"<error {ctypes.get_last_error()}>"


def platform_facts():
    show("platform", platform.platform())
    show("python", sys.version)
    if hasattr(sys, "getwindowsversion"):
        show("windows version", sys.getwindowsversion())
    for name in ("TEMP", "TMP", "USERPROFILE", "HOME", "PYTHONUTF8", "PYTHONIOENCODING"):
        show(name, os.environ.get(name))
    temp = tempfile.gettempdir()
    show("gettempdir", temp)
    show("resolve", Path(temp).resolve())
    show("realpath", os.path.realpath(temp))
    show("absolute", Path(temp).absolute())
    show("GetLongPathNameW", long_path(temp))
    show("locale encoding", __import__("locale").getpreferredencoding(False))
    show("stdout encoding", sys.stdout.encoding)
    show("os.linesep", repr(os.linesep))


def file_stats():
    directory = Path(tempfile.mkdtemp(prefix="probe-stat-")).resolve()
    leaf = directory / "a.txt"
    leaf.write_bytes(b"first")
    named = os.lstat(leaf)
    followed = os.stat(leaf)
    with open(leaf, "rb") as stream:
        handle = os.fstat(stream.fileno())
    show("file lstat fields", fields(named))
    show("file lstat vs stat", diff(named, followed))
    show("file lstat vs fstat", diff(named, handle))
    show("file Path.lstat vs os.lstat", diff(leaf.lstat(), named))
    # Write in place while a handle is open.
    with open(leaf, "r+b") as stream:
        stream.write(b"second-longer")
        stream.flush()
        during_named, during_handle = os.lstat(leaf), os.fstat(stream.fileno())
        show("open-for-write lstat vs fstat", diff(during_named, during_handle))
    after_named = os.lstat(leaf)
    with open(leaf, "rb") as stream:
        after_handle = os.fstat(stream.fileno())
    show("after close lstat vs fstat", diff(after_named, after_handle))
    show("rewrite changed (lstat before/after)", diff(named, after_named))
    # Replace with a new file.
    temporary = directory / "a.tmp"
    temporary.write_bytes(b"replacement")
    os.replace(temporary, leaf)
    replaced = os.lstat(leaf)
    with open(leaf, "rb") as stream:
        replaced_handle = os.fstat(stream.fileno())
    show("after replace lstat vs fstat", diff(replaced, replaced_handle))
    show("replace changed (lstat)", diff(after_named, replaced))
    # Several reads of the same unchanged file.
    for attempt in range(3):
        with open(leaf, "rb") as stream:
            show(f"read {attempt} fstat vs lstat", diff(os.fstat(stream.fileno()), os.lstat(leaf)))
    shutil.rmtree(directory)


def directory_stats():
    directory = Path(tempfile.mkdtemp(prefix="probe-dir-")).resolve()
    sub = directory / "findings"
    sub.mkdir()
    time.sleep(0.05)
    before = os.lstat(sub)
    (sub / ".churn-0").write_bytes(b"x")
    after = os.lstat(sub)
    show("dir lstat fields", fields(before))
    show("dir lstat after create (immediately)", diff(before, after))
    time.sleep(0.05)
    later = os.lstat(sub)
    show("dir lstat after create (50 ms)", diff(before, later))
    with os.scandir(sub) as scanner:
        names = [entry.name for entry in scanner]
    show("scandir", names)
    again = os.lstat(sub)
    show("dir lstat after scandir", diff(before, again))
    (sub / ".churn-1").write_bytes(b"x")
    show("dir lstat after second create", diff(again, os.lstat(sub)))
    os.unlink(sub / ".churn-1")
    show("dir lstat after delete", diff(again, os.lstat(sub)))
    if os.name == "nt":
        sys.path.insert(0, str(EDITION / "memory-bank/scripts"))
        try:
            retrieval = importlib.import_module("context_retrieval")
        finally:
            sys.path.pop(0)
        files = retrieval._cursor_nt_backend()
        fd = files.open_root(sub)
        try:
            first = files.fstat(fd)
            (sub / ".churn-2").write_bytes(b"x")
            second = files.fstat(fd)
            show("NT handle dir fstat after create", diff(first, second))
            show("NT handle vs os.lstat (dir)", diff(second, os.lstat(sub)))
        finally:
            os.close(fd)
        parent = files.open_root(directory)
        try:
            leaf = directory / "b.txt"
            leaf.write_bytes(b"b")
            show("NT stat(dir_fd) vs os.lstat (file)", diff(files.stat("b.txt", dir_fd=parent, follow_symlinks=False),
                                                         os.lstat(leaf)))
        finally:
            os.close(parent)
    shutil.rmtree(directory)


def bash_and_git():
    show("which bash", shutil.which("bash"))
    show("which git", shutil.which("git"))
    show("which python3", shutil.which("python3"))
    show("which python", shutil.which("python"))
    show("which py", shutil.which("py"))
    try:
        result = subprocess.run(["bash", "-c", "echo bare-bash $BASH_VERSION"], capture_output=True, timeout=30)
        show("bare bash", (result.returncode, result.stdout[:200], result.stderr[:200]))
    except OSError as error:
        show("bare bash", error)
    git = shutil.which("git")
    for folder in Path(git).resolve().parents if git else ():
        if (folder / "bin" / "bash.exe").is_file():
            bash = str(folder / "bin" / "bash.exe")
            result = subprocess.run([bash, "-c", "echo git-bash $BASH_VERSION; command -v python3 python py"],
                                    capture_output=True, timeout=30)
            show("git bash", (bash, result.returncode, result.stdout[:300], result.stderr[:200]))
            break
    result = subprocess.run(["git", "--version"], capture_output=True, text=True)
    show("git version", result.stdout.strip())
    result = subprocess.run(["git", "config", "--list", "--show-origin", "--show-scope"], capture_output=True, text=True)
    for line in result.stdout.splitlines():
        if any(key in line for key in ("autocrlf", "safecrlf", "symlinks", "safe.directory", "fscache", "eol",
                                       "longpaths", "filemode")):
            show("git config", line)


def copy_runtime(root):
    for relative in ("memory-bank/scripts", "memory-bank/templates", "project-brain/config",
                     "project-brain/schemas", "project-brain/templates"):
        shutil.copytree(EDITION / relative, root / relative, ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
    (root / "project-brain/config/runtime.json").write_text(
        json.dumps({"mode": "governed", "automatic_promotion": False}), encoding="utf-8")


def learning_image():
    root = Path(tempfile.mkdtemp(prefix="probe-learning-")).resolve()
    copy_runtime(root)
    (root / "specs").mkdir()
    (root / "specs/kite.md").write_bytes(b"# Kite\n\nKites glide.\n")
    sys.path.insert(0, str(root / "memory-bank/scripts"))
    names = [leaf.stem for leaf in (root / "memory-bank/scripts").glob("*.py")]
    previous = {name: sys.modules.pop(name) for name in names if name in sys.modules}
    try:
        brain = importlib.import_module("brain_runtime")
        results = importlib.import_module("memory_results")
        record = brain.create_record(root, "finding", "KITE-1", "Kite finding", [], ["specs/kite.md"],
                                     owner="local", goal="Kite goal.")
        path, _, _ = brain.find_record(root, record["id"], include_archive=True)
        image = path.lstat()
        with path.open("rb") as stream:
            handle = os.fstat(stream.fileno())
        show("record lstat vs fstat", diff(image, handle))
        show("record stat vs lstat", diff(path.stat(), image))
        show("linked_reparse", brain.linked_reparse(image) if hasattr(brain, "linked_reparse") else "n/a")
        show("_learning_image", results._learning_image(root, record["id"]))
    finally:
        sys.path.pop(0)
        for name in names:
            sys.modules.pop(name, None)
        sys.modules.update(previous)
        shutil.rmtree(root, ignore_errors=True)


def cursor_topic():
    root = Path(tempfile.mkdtemp(prefix="probe-topic-")).resolve()
    copy_runtime(root)
    (root / "memory-bank/chunks").mkdir()
    (root / ".gitignore").write_bytes(b".cursor/rules/working-memory.mdc\nmemory-bank/local/\nproject-brain/local/\n")
    fixture = json.loads((ROOT / "tests/fixtures/cursor_continuation_state_public.json").read_text(encoding="utf-8"))
    for name, text in [(fixture["source_path"], fixture["source"]), (fixture["other_path"], fixture["other_source"]),
                       *fixture["background_sources"].items()]:
        leaf = root / name
        leaf.parent.mkdir(exist_ok=True)
        leaf.write_bytes(text.encode("utf-8"))
    env = {k: v for k, v in os.environ.items() if not k.startswith(("GIT_", "CONTEXT_", "ACCELERATOR_"))}
    env.update(GIT_OPTIONAL_LOCKS="0", PYTHONIOENCODING="utf-8", PYTHONDONTWRITEBYTECODE="1")
    subprocess.run(["git", "-C", str(root), "init", "-q", "-b", "main"], env=env, check=True)
    cli = root / "memory-bank/scripts/context.py"
    task = fixture["task_id"]
    result = subprocess.run([sys.executable, "-B", str(cli), "--root", str(root), "--mode", "governed", "refresh",
                             "--query", fixture["substantive_query"], "--task-id", task, "--host", "cursor",
                             "--sanitize", "--ephemeral", "--gate", "shadow", "--json",
                             "--delivery-profile", "cursor-rule"], env=env, cwd=root, capture_output=True, timeout=60)
    show("refresh rc", result.returncode)
    show("refresh stderr", result.stderr[-500:])
    reply = json.loads(result.stdout)
    show("refresh semantic", [item["path"] for item in reply["capsule"]["semantic"]])
    connection = sqlite3.connect(root / "memory-bank/local/context.db")
    connection.row_factory = sqlite3.Row
    try:
        row = connection.execute("SELECT value FROM index_state WHERE key='cursor-topic-v1'").fetchone()
        show("stored topic", row[0][:200] if row else None)
        scripts = root / "memory-bank/scripts"
        sys.path.insert(0, str(scripts))
        names = [leaf.stem for leaf in scripts.glob("*.py")]
        previous = {name: sys.modules.pop(name) for name in names if name in sys.modules}
        try:
            context = importlib.import_module("context")
            retrieval = importlib.import_module("context_retrieval")
            original = retrieval._cursor_git_config_read

            def traced(arguments, environment):
                value = original(arguments, environment)
                show("git config read", (arguments[-4:], value))
                return value

            retrieval._cursor_git_config_read = traced
            original_safe = retrieval._cursor_protected_safe_root

            def traced_safe(project):
                value = original_safe(project)
                show("protected safe root", value)
                return value

            retrieval._cursor_protected_safe_root = traced_safe
            leaf = context.brain_root(root) / "config/runtime.json"
            meta = leaf.lstat()
            with leaf.open("rb") as stream:
                after = os.fstat(stream.fileno())
            show("config lstat vs fstat", diff(meta, after))
            show("config lstat again", diff(meta, leaf.lstat()))
            show("scope", context._cursor_topic_scope(connection, root, task, "governed"))
        finally:
            sys.path.pop(0)
            for name in names:
                sys.modules.pop(name, None)
            sys.modules.update(previous)
    finally:
        connection.close()
        shutil.rmtree(root, ignore_errors=True)


def timestamp_granularity():
    directory = Path(tempfile.mkdtemp(prefix="probe-tick-")).resolve()
    times = []
    for index in range(40):
        (directory / f"f{index}").write_bytes(b"x")
        times.append(os.lstat(directory).st_mtime_ns)
    distinct = sorted(set(times))
    show("dir mtimes after 40 quick creates: distinct", len(distinct))
    show("smallest step (ns)", min((b - a for a, b in zip(distinct, distinct[1:])), default=None))
    leaf = directory / "f0"
    stamps = []
    for index in range(40):
        leaf.write_bytes(b"y" * index)
        stamps.append(os.lstat(leaf).st_mtime_ns)
    show("file mtimes after 40 quick rewrites: distinct", len(set(stamps)))
    sys.path.insert(0, str(EDITION / "memory-bank/scripts"))
    try:
        roots = importlib.import_module("workspace_roots")
    finally:
        sys.path.pop(0)
    with open(leaf, "rb") as stream:
        handle = os.fstat(stream.fileno())
    named = os.lstat(leaf)
    show("change_time_ns lstat == fstat", roots.change_time_ns(named) == roots.change_time_ns(handle))
    shutil.rmtree(directory)


def main():
    for function in (platform_facts, file_stats, directory_stats, bash_and_git, timestamp_granularity,
                     learning_image, cursor_topic):
        experiment(function)


if __name__ == "__main__":
    main()
