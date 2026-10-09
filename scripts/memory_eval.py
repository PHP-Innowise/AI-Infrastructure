#!/usr/bin/env python3
"""Measure the prompt-time memory capsule on each project as it was then.

The read hook puts a Task Capsule - excerpts of the project's memory - in
front of every prompt. Measured on real prompts, it looked useful three to
four times as often as it was: those measurements indexed each project's
current state, so knowledge written after a prompt, often about that prompt's
own work, was "retrieved" for it (leakage from the future). This stand
rebuilds every project as it was at the prompt's timestamp, overlays the
runtime and skills under test from this clone, runs the refresh the hook
runs, and scores what came back against human judgments.

    python3 scripts/memory_eval.py run --set SET --judgments JUDGMENTS \\
        [--passages PASSAGES] --projects-root DIR --out RESULT \\
        [--edition auto|NAME|DIR] [--as-of prompt|now] [--clock as-of|real] \\
        [--host claude|codex|cursor] [--ids A,B] [--limit N] [--cache DIR] \\
        [--timeout SECONDS] [--keep]
    python3 scripts/memory_eval.py report RESULT [--compare OTHER] [--show-unjudged]
    python3 scripts/memory_eval.py realized --project DIR [--claude-root DIR] \\
        [--codex-root DIR] [--since YYYY-MM-DD] [--json] [--paths]

`run`, per prompt, never writing into the project (all work is in the cache):

* Files: the newest commit reachable from the project's HEAD with a committer
  date at or before the prompt (`git rev-list -1 --before`), read blob by blob
  (`ls-tree` + `cat-file --batch`) into a per-tree cache. That is the exact
  committed content; `git archive` would apply the tree's `export-ignore` and
  `export-subst` attributes, which PHP packages use to drop docs and tests.
* Memory: Project Brain records (`project-brain/dynamic/**`) and handoffs,
  and Memory Bank chunks (`memory-bank/chunks/MEM-*.md`), created after the
  prompt are dropped; records and chunks that exist only in the project's
  working tree (often never committed) are added when they demonstrably
  existed at the prompt: a creation time at or before it (record
  `created_at`; for a chunk the date in its name, narrowed by the promotion
  that wrote it), or a file modification time at or before it.
* Runtime: the accelerator-owned files of every edition are removed and the
  edition under test is copied in from this clone's inventory, the way an
  install copies it; project-owned state (Brain records, archive and control
  files, chunks, the indexes, runtime.json, the spec manifest, the task
  counter) is never replaced, and `.gitignore`, `.gitattributes`, `AGENTS.md`
  and `.claude/CLAUDE.md` are merged as the installer merges them.
  `--edition auto` (the default) picks the edition per project.
* Git: the corpus is committed on a branch `eval/<id>` whose parent is the
  reconstructed commit, its history borrowed (`objects/info/alternates`) from
  a clone of the project kept in the cache, so ignore rules and history-based
  checks - the codebase map's drift, the default branch - see a real
  checkout. Borrowing from the project itself would write into it: Git
  refreshes the modification time of every borrowed object it re-adds.
* Clock: the runtime's clock reads the prompt's instant (`--clock as-of`), so
  review dates, validity windows and recency decay are those of the prompt
  and a result does not drift with the calendar.

Then `context.py index` warms the index and `context.py refresh --query
<prompt> --sanitize --ephemeral --json` is timed and scored. `--as-of now`
reproduces the old, leaky measurement on the current working tree.

Outputs carry prompt ids, document paths and counts - never prompt text,
capsule text or document bodies. Sets, judgments, passages, transcripts and
results are client data: keep them outside this repository. See
docs/MEMORY-EVAL.md. Standard library only, Python 3.9+, no network; never
installed into a project, and not run by CI (tests/test_memory_eval.py is).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time
from collections import Counter
from datetime import date, datetime, timedelta, timezone
from pathlib import Path, PurePosixPath
from typing import Any, Callable, Dict, Iterator, List, Optional, Sequence, Set, Tuple

ROOT = Path(__file__).resolve().parent.parent
if str(ROOT / "scripts") not in sys.path:
    sys.path.insert(0, str(ROOT / "scripts"))

import accelerator_attach  # noqa: E402  - edition detection from framework markers
import install_accelerator as installer  # noqa: E402  - inventories and install rules

UTC = timezone.utc
HOSTS = ("claude", "codex", "cursor")
LAYERS = ("semantic", "episodic", "procedural")
CLASSES = ("useful", "noise-only", "unjudged-only", "silent")
CANON_SKILLS = ".agents/skills/"
TOOL_SKILLS = (".claude/skills/", ".cursor/skills/", ".codex/skills/")
RECORDS = "project-brain/dynamic"
HANDOFFS = "project-brain/control/handoffs"
PROMOTIONS = "project-brain/control/promotions"
CHUNKS = "memory-bank/chunks"
LOCAL_STATE = ("memory-bank/local", "project-brain/local")
# State a project owns once the accelerator is installed: an overlay keeps the
# project's copy and seeds only a missing file, as an install does.
PROJECT_TREES = (
    "project-brain/dynamic/",
    "project-brain/archive/",
    "project-brain/control/",
    "memory-bank/chunks/",
)
PROJECT_FILES = frozenset(
    {"memory-bank/INDEX.md", "project-brain/config/runtime.json", *installer.SEED_ONLY_PATHS}
)
# Files a project may have of its own; an accelerator copy of them is
# recognised by its first line, which every edition release has kept.
FIRST_LINE_FILES = ("AGENTS.md", ".claude/CLAUDE.md", "README.md")
CHUNK_NAME = re.compile(r"^(MEM-(\d{8})-[0-9a-f]{8})-")
INSTALL_MANIFEST = installer.SYNC_MANIFEST
POLICY_LOCK = ".accelerator-policy-lock.json"
DEFAULT_TIMEOUT = 30.0
IDENTITY = ("memory-eval", "memory-eval@localhost")
# The refresh runs with these variables of the caller's environment and no
# others, so neither an attached-mode session, a task override nor a Git
# redirection of the caller can reach the corpus.
ENV_PASSTHROUGH = (
    "PATH", "LANG", "LC_ALL", "LC_CTYPE", "TMPDIR", "TEMP", "TMP", "SYSTEMROOT",
    "SYSTEMDRIVE", "COMSPEC", "PATHEXT", "WINDIR", "USER", "LOGNAME", "USERNAME",
)
STDERR_LINES = 6
STDERR_CHARACTERS = 600
PROMPT_WINDOW = 20

# Pins the clock of the memory runtime the stand starts: `date.today()` and
# `datetime.now()` read the prompt's instant, plus the time since start-up.
# Loaded as `sitecustomize` through PYTHONPATH, so every Python process of the
# refresh sees it before the runtime imports `datetime`.
CLOCK_SHIM = '''\
"""Written by scripts/memory_eval.py: pins the memory runtime's clock."""
import os as _os

_anchor = _os.environ.get("MEMORY_EVAL_CLOCK", "")
if _anchor:
    import datetime as _datetime
    import time as _time

    _offset = _datetime.datetime.fromisoformat(_anchor).timestamp() - _time.time()

    class date(_datetime.date):
        __slots__ = ()

        @classmethod
        def today(cls):
            return cls.fromtimestamp(_time.time() + _offset)

    class datetime(_datetime.datetime):
        __slots__ = ()

        @classmethod
        def now(cls, tz=None):
            return cls.fromtimestamp(_time.time() + _offset, tz)

        @classmethod
        def today(cls):
            return cls.fromtimestamp(_time.time() + _offset)

        @classmethod
        def utcnow(cls):
            return cls.utcfromtimestamp(_time.time() + _offset)

    date.__module__ = datetime.__module__ = "datetime"
    _datetime.date = date
    _datetime.datetime = datetime
'''


class EvalError(Exception):
    """A usage or input problem: the command cannot start."""


class Skip(Exception):
    """One prompt cannot be evaluated. The reason is a fixed word, never prompt text."""

    def __init__(self, reason: str) -> None:
        super().__init__(reason)
        self.reason = reason


# --------------------------------------------------------------------------
# Small helpers
# --------------------------------------------------------------------------


def parse_instant(value: object) -> Optional[datetime]:
    """An ISO 8601 date and time as aware UTC, or None. A bare date is not one."""
    if not isinstance(value, str):
        return None
    text = value.strip()
    if len(text) < 16 or text[10] not in "Tt ":
        return None
    if text[-1] in "Zz":
        text = text[:-1] + "+00:00"
    # Python 3.9 reads three or six fractional digits only.
    text = re.sub(r"\.(\d+)", lambda match: "." + (match.group(1) + "000000")[:6], text, count=1)
    text = re.sub(r"([+-]\d\d)(\d\d)$", r"\1:\2", text)
    try:
        moment = datetime.fromisoformat(text)
    except ValueError:
        return None
    if moment.tzinfo is None:
        moment = moment.replace(tzinfo=UTC)
    return moment.astimezone(UTC)


def parse_day(value: object) -> Optional[date]:
    if not isinstance(value, str) or len(value.strip()) < 10:
        return None
    try:
        return date.fromisoformat(value.strip()[:10])
    except ValueError:
        return None


def iso(moment: datetime) -> str:
    return moment.astimezone(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")


def now_iso() -> str:
    return iso(datetime.now(UTC))


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def read_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError, UnicodeDecodeError):
        return None


def write_json(path: Path, document: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp-{os.getpid()}")
    temporary.write_text(json.dumps(document, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def remove_tree(path: Path) -> None:
    """Remove a file or a directory tree, including read-only Git objects."""
    if path.is_symlink() or path.is_file():
        path.unlink()
        return
    if not path.exists():
        return

    def retry(function: Callable[..., Any], name: str, *_: Any) -> None:
        try:
            os.chmod(name, stat.S_IREAD | stat.S_IWRITE | stat.S_IEXEC)
        except OSError:
            pass
        function(name)

    if sys.version_info >= (3, 12):
        shutil.rmtree(path, onexc=retry)  # type: ignore[call-arg]
    else:
        shutil.rmtree(path, onerror=retry)


def prepare_parent(root: Path, target: Path) -> None:
    """Make every directory between root and target a real directory, so a
    write can never pass through a symlink or stop at a file in the way."""
    current = root
    for part in target.relative_to(root).parts[:-1]:
        current = current / part
        if current.is_symlink() or (current.exists() and not current.is_dir()):
            current.unlink()
        if not current.exists():
            current.mkdir()


def inside_clone(path: Path) -> bool:
    resolved = path.expanduser().resolve()
    return resolved == ROOT or ROOT in resolved.parents


def warn_if_inside_clone(label: str, path: Optional[Path]) -> None:
    if path is not None and inside_clone(path):
        print(
            f"warning: {label} {path} is inside this clone; sets, judgments, passages, "
            "transcripts and results are client data - keep them outside the repository",
            file=sys.stderr,
        )


def normalise_path(value: str) -> str:
    """One spelling per document: tool skill trees name the canonical `.agents/skills`."""
    path = value.strip().replace("\\", "/")
    while path.startswith("./"):
        path = path[2:]
    for tree in TOOL_SKILLS:
        if path.startswith(tree):
            return CANON_SKILLS + path[len(tree):]
    return path


def is_skill(path: str) -> bool:
    return path.startswith(CANON_SKILLS) or "/skills/" in path


def flatten(text: str) -> str:
    return " ".join(text.split()).casefold()


def safe_name(value: str) -> str:
    """A file and branch name for a prompt id: [A-Za-z0-9._-], short, never empty."""
    name = re.sub(r"[^A-Za-z0-9._-]+", "-", value)
    name = re.sub(r"\.{2,}", ".", name).strip(".-")[:80]
    if not name or name.endswith(".lock"):
        name = (name + "-p") if name else "p"
    return name


# --------------------------------------------------------------------------
# Git, read-only on the project
# --------------------------------------------------------------------------


def read_env() -> Dict[str, str]:
    """The caller's environment for read-only Git on a project: no redirection,
    no opportunistic index writes, no lazy fetch of a partial clone."""
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    env.update(GIT_OPTIONAL_LOCKS="0", GIT_TERMINAL_PROMPT="0", GIT_NO_LAZY_FETCH="1")
    return env


def read_git(directory: Path, *arguments: str) -> Optional[str]:
    try:
        result = subprocess.run(
            ["git", "-C", str(directory), *arguments], capture_output=True, env=read_env()
        )
    except OSError:
        return None
    if result.returncode != 0:
        return None
    return result.stdout.decode("utf-8", "surrogateescape").strip()


class BlobReader:
    """`git cat-file --batch`, one object at a time."""

    def __init__(self, project: Path) -> None:
        self.process = subprocess.Popen(
            ["git", "-C", str(project), "cat-file", "--batch"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            env=read_env(),
        )

    def __enter__(self) -> "BlobReader":
        return self

    def __exit__(self, *_: object) -> None:
        try:
            if self.process.stdin:
                self.process.stdin.close()
        except OSError:
            pass
        try:
            self.process.wait(timeout=30)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
        if self.process.stdout:
            self.process.stdout.close()

    def read(self, blob: bytes) -> Optional[bytes]:
        assert self.process.stdin is not None and self.process.stdout is not None
        self.process.stdin.write(blob + b"\n")
        self.process.stdin.flush()
        fields = self.process.stdout.readline().split()
        if len(fields) != 3 or fields[1] != b"blob":
            return None  # "<id> missing": an object this clone does not have
        size = int(fields[2])
        data = self.process.stdout.read(size)
        self.process.stdout.read(1)
        if len(data) != size:
            raise OSError("short read from git cat-file")
        return data


class Repository:
    """Read-only facts about the Git repository a project lives in."""

    def __init__(self, project: Path) -> None:
        self.project = project
        # None: not a repository. "": the project is the repository's root.
        self.prefix = read_git(project, "rev-parse", "--show-prefix")

    def commit_before(self, moment: datetime) -> Optional[str]:
        stamp = moment.astimezone(UTC).strftime("%Y-%m-%d %H:%M:%S +0000")
        return read_git(self.project, "rev-list", "-1", f"--before={stamp}", "HEAD") or None

    def head(self) -> Optional[str]:
        return read_git(self.project, "rev-parse", "--verify", "--quiet", "HEAD") or None

    def tree(self, commit: str) -> Optional[str]:
        spec = f"{commit}:{self.prefix.rstrip('/')}" if self.prefix else f"{commit}^{{tree}}"
        tree = read_git(self.project, "rev-parse", "--verify", "--quiet", spec)
        if tree and read_git(self.project, "cat-file", "-t", tree) == "tree":
            return tree
        return None

    def history(self, cache: Path, commit: str) -> Optional[Path]:
        """An object store in the cache holding `commit` and its history.

        A corpus commit takes the project's commit as its parent only when the
        project is the repository root, where both trees share their paths.
        It borrows the history from a clone in the cache, never from the
        project itself: Git refreshes the modification time of every borrowed
        object it would otherwise have written, and that is a write into the
        project. The clone is made once per project and fetched into when a
        newer commit is needed.
        """
        if self.prefix != "":
            return None
        key = hashlib.sha256(str(self.project).encode("utf-8")).hexdigest()[:16]
        mirror = cache / "history" / f"{key}.git"
        if not mirror.is_dir():
            mirror.parent.mkdir(parents=True, exist_ok=True)
            staging = Path(tempfile.mkdtemp(prefix=".clone-", dir=str(mirror.parent)))
            try:
                cloned = subprocess.run(
                    [
                        "git", "clone", "--bare", "--quiet", "--no-hardlinks",
                        str(self.project), str(staging / "repo.git"),
                    ],
                    capture_output=True,
                    env=read_env(),
                )
                if cloned.returncode != 0:
                    return None
                try:
                    os.rename(staging / "repo.git", mirror)
                except OSError:
                    if not mirror.is_dir():
                        raise
            finally:
                remove_tree(staging)
        wanted = f"{commit}^{{commit}}"
        if read_git(mirror, "cat-file", "-e", wanted) is None:
            subprocess.run(
                [
                    "git", "-C", str(mirror), "fetch", "--quiet", "--no-tags",
                    str(self.project), "+HEAD:refs/memory-eval/head",
                ],
                capture_output=True,
                env=read_env(),
            )
            if read_git(mirror, "cat-file", "-e", wanted) is None:
                return None
        return mirror / "objects"


def materialize(project: Path, tree: str, cache: Path) -> Path:
    """The committed files of `tree`, exactly, in `cache/trees/<tree>` (reused)."""
    target = cache / "trees" / tree
    if target.is_dir():
        return target
    target.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=f".{tree[:12]}-", dir=str(target.parent)))
    try:
        listing = subprocess.run(
            ["git", "-C", str(project), "ls-tree", "-r", "-z", tree], capture_output=True, env=read_env()
        )
        if listing.returncode != 0:
            raise Skip("unreadable-commit")
        blobs: Dict[bytes, List[Tuple[PurePosixPath, bool]]] = {}
        for entry in listing.stdout.split(b"\0"):
            meta, _, raw = entry.partition(b"\t")
            fields = meta.split()
            # Symlinks (120000) and submodules (160000) are not materialized:
            # a link could point a later write outside the corpus.
            if len(fields) != 3 or fields[1] != b"blob" or fields[0] not in (b"100644", b"100755"):
                continue
            path = PurePosixPath(raw.decode("utf-8", "surrogateescape"))
            if path.is_absolute() or any(part in ("", ".", "..", ".git") for part in path.parts):
                continue
            blobs.setdefault(fields[2], []).append((path, fields[0] == b"100755"))
        with BlobReader(project) as reader:
            for blob, paths in blobs.items():
                data = reader.read(blob)
                if data is None:
                    continue
                for path, executable in paths:
                    destination = staging.joinpath(*path.parts)
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    destination.write_bytes(data)
                    if executable:
                        os.chmod(destination, 0o755)
        try:
            os.rename(staging, target)
        except OSError:
            if not target.is_dir():
                raise
            remove_tree(staging)  # another run materialized it first
        return target
    except BaseException:
        remove_tree(staging)
        raise


def snapshot_worktree(project: Path, target: Path) -> Path:
    """The project's current working tree as the index sees it (`--as-of now`).

    What Git lists (tracked and untracked, not ignored) plus every file of the
    memory trees, which the runtime reads whatever the ignore rules say;
    without `.git`, local state and symlinks. Ignored build output elsewhere
    (vendor/, node_modules/) is never indexed and is not copied.
    """
    if target.is_dir():
        return target
    staging = Path(tempfile.mkdtemp(prefix=".snapshot-", dir=str(target.parent)))
    try:
        listed = subprocess.run(
            ["git", "-C", str(project), "ls-files", "-z", "-c", "-o", "--exclude-standard"],
            capture_output=True,
            env=read_env(),
        )
        relatives: Set[str] = set()
        if listed.returncode == 0:
            relatives.update(
                item.decode("utf-8", "surrogateescape") for item in listed.stdout.split(b"\0") if item
            )
            walked: Tuple[str, ...] = ("project-brain", "memory-bank", "specs", "tasks")
        else:
            walked = (".",)
        for top in walked:
            base = project / top
            if not base.is_dir() or base.is_symlink():
                continue
            for directory, folders, files in os.walk(base):
                folders[:] = [name for name in folders if name != ".git"]
                for name in files:
                    relatives.add((Path(directory) / name).relative_to(project).as_posix())
        for relative in sorted(relatives):
            parts = PurePosixPath(relative).parts
            if ".git" in parts or any(relative.startswith(local + "/") for local in LOCAL_STATE):
                continue
            source = project / relative
            if source.is_symlink() or not source.is_file():
                continue
            destination = staging / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
        os.rename(staging, target)
        return target
    except BaseException:
        remove_tree(staging)
        raise


# --------------------------------------------------------------------------
# Memory as of the prompt
# --------------------------------------------------------------------------

Window = Tuple[datetime, datetime]  # earliest and latest instant of creation
EARLIEST = datetime.min.replace(tzinfo=UTC)
LATEST = datetime.max.replace(tzinfo=UTC)
FENCED = re.compile(r"\A(?:\ufeff)?---[ \t]*\r?\n(.*?)\r?\n---[ \t]*(?:\r?\n|\Z)", re.S)


def day_window(day: date) -> Window:
    start = datetime(day.year, day.month, day.day, tzinfo=UTC)
    return start, start + timedelta(days=1)


def window_of(value: object) -> Optional[Window]:
    instant = parse_instant(value)
    if instant is not None:
        return instant, instant
    day = parse_day(value)
    return day_window(day) if day is not None else None


def narrowest(windows: Sequence[Optional[Window]]) -> Optional[Window]:
    present = [window for window in windows if window is not None]
    if not present:
        return None
    earliest = max(window[0] for window in present)
    latest = min(window[1] for window in present)
    if earliest <= latest:
        return earliest, latest
    # Disagreeing evidence: trust the most precise piece.
    return min(present, key=lambda window: window[1] - window[0])


def document_metadata(path: Path) -> Dict[str, Any]:
    """The JSON front matter of a record or chunk (or a JSON file's object);
    for anything else, the few keys this stand needs, read by pattern."""
    try:
        with path.open("rb") as handle:
            text = handle.read(262144).decode("utf-8", "replace")
    except OSError:
        return {}
    match = FENCED.match(text)
    raw = match.group(1) if match else text
    try:
        value = json.loads(raw)
    except ValueError:
        value = None
    if isinstance(value, dict):
        return value
    found: Dict[str, Any] = {}
    for key in ("id", "created_at", "updated_at", "created", "last_verified"):
        hit = re.search(r'(?m)^[ \t]*"?%s"?[ \t]*:[ \t]*["\']?([^"\'\n,]+)' % key, raw)
        if hit:
            found[key] = hit.group(1).strip()
    return found


def memory_documents(root: Path) -> Iterator[Tuple[str, str]]:
    """(relative path, kind) of each Brain record, handoff and Memory Bank chunk."""
    for base, kind in ((RECORDS, "record"), (HANDOFFS, "handoff"), (CHUNKS, "chunk")):
        top = root / base
        if not top.is_dir() or top.is_symlink():
            continue
        for directory, folders, files in os.walk(top):
            if kind != "record":
                folders[:] = []  # the index reads handoffs and chunks one level deep
            folders.sort()
            for name in sorted(files):
                if name.startswith(".") or not name.endswith((".md", ".json")):
                    continue
                if kind != "record" and not name.endswith(".md"):
                    continue
                path = Path(directory) / name
                if path.is_symlink() or not path.is_file():
                    continue
                yield path.relative_to(root).as_posix(), kind


def promotion_windows(root: Path) -> Dict[str, Window]:
    """When each promoted chunk was written: after its promotion was proposed,
    at or before the promotion was marked applied."""
    windows: Dict[str, Window] = {}
    top = root / PROMOTIONS
    if not top.is_dir():
        return windows
    for path in sorted(top.glob("*.json")):
        meta = document_metadata(path)
        memory_id = meta.get("destination_memory_id")
        if not isinstance(memory_id, str) or not memory_id:
            continue
        earliest = parse_instant(meta.get("created_at")) or EARLIEST
        later = [parse_instant(meta.get(key)) for key in ("updated_at", "reviewed_at")]
        known = [moment for moment in later if moment is not None]
        latest = max(known) if known else LATEST
        if earliest is EARLIEST and latest is LATEST:
            continue
        windows[memory_id] = (earliest, latest)
    return windows


def creation_window(path: Path, kind: str, promotions: Dict[str, Window]) -> Optional[Window]:
    meta = document_metadata(path)
    if kind != "chunk":
        return window_of(meta.get("created_at"))
    windows: List[Optional[Window]] = []
    named = CHUNK_NAME.match(path.name)
    memory_id = meta.get("id") if isinstance(meta.get("id"), str) else (named.group(1) if named else None)
    if named:
        digits = named.group(2)
        day = parse_day(f"{digits[:4]}-{digits[4:6]}-{digits[6:]}")
        windows.append(day_window(day) if day else None)
    else:
        windows.append(window_of(meta.get("created")))
    if isinstance(memory_id, str) and memory_id in promotions:
        windows.append(promotions[memory_id])
    return narrowest(windows)


def changed_after(path: Path, kind: str, moment: datetime) -> bool:
    """Whether the document says it was edited after `moment`."""
    meta = document_metadata(path)
    if kind == "chunk":
        verified = parse_day(meta.get("last_verified"))
        return verified is not None and verified > moment.date()
    updated = parse_instant(meta.get("updated_at"))
    return updated is not None and updated > moment


def modified_at(path: Path) -> datetime:
    return datetime.fromtimestamp(path.stat().st_mtime, UTC)


def reconstruct_memory(
    project: Path, corpus: Path, moment: datetime, updated_after: str = "drop"
) -> Dict[str, int]:
    """Bring the corpus's memory to `moment`: drop what was written later, add
    what existed then in the project's working tree only.

    A working-tree document created before the prompt but edited after it
    cannot be rewound: its body may hold the very answer the prompt's work
    produced. `drop` (the default, the strict as-of corpus) leaves it out and
    counts it as `unreconstructable_updated`; `keep` copies today's body and
    counts it as `updated_after`, and the run marks the turn contaminated.
    """
    counts = {"from_git": 0, "from_worktree": 0, "dropped_future": 0, "undetermined": 0, "updated_after": 0,
              "unreconstructable_updated": 0}
    promotions = promotion_windows(corpus)
    promotions.update(promotion_windows(project))
    committed: Set[str] = set()
    for relative, kind in list(memory_documents(corpus)):
        committed.add(relative)
        window = creation_window(corpus / relative, kind, promotions)
        if window is not None and window[0] > moment:
            (corpus / relative).unlink()
            counts["dropped_future"] += 1
        else:
            counts["from_git"] += 1
    for relative, kind in list(memory_documents(project)):
        # A handoff is a rolling summary: the working tree holds today's text.
        if relative in committed or kind == "handoff":
            continue
        source = project / relative
        window = creation_window(source, kind, promotions)
        if window is not None and window[0] > moment:
            counts["dropped_future"] += 1
            continue
        try:
            modified = modified_at(source)
        except OSError:
            continue
        # A file unmodified since before the prompt existed then, as it is now.
        if (window is not None and window[1] <= moment) or modified <= moment:
            # The document's own edit time decides: a copy tool can preserve
            # or restore an older mtime over a body written after the prompt.
            edited = changed_after(source, kind, moment)
            if edited and updated_after == "drop":
                counts["unreconstructable_updated"] += 1
                continue
            target = corpus / relative
            prepare_parent(corpus, target)
            shutil.copy2(source, target)
            counts["from_worktree"] += 1
            if edited:
                counts["updated_after"] += 1
        else:
            counts["undetermined"] += 1
    return counts


def count_present(corpus: Path, moment: datetime) -> Dict[str, int]:
    """`--as-of now`: what the current tree holds that the prompt could not have seen."""
    counts = {"from_git": 0, "from_worktree": 0, "dropped_future": 0, "future_present": 0, "undetermined": 0}
    promotions = promotion_windows(corpus)
    for relative, kind in memory_documents(corpus):
        path = corpus / relative
        counts["from_worktree"] += 1
        window = creation_window(path, kind, promotions)
        if window is not None and window[0] > moment:
            counts["future_present"] += 1
        elif not ((window is not None and window[1] <= moment) or modified_at(path) <= moment):
            counts["undetermined"] += 1
    return counts


# --------------------------------------------------------------------------
# Editions: which one, and the overlay
# --------------------------------------------------------------------------


def edition_aliases() -> Dict[str, str]:
    aliases: Dict[str, str] = {}
    for name, relative in installer.EDITION_PATHS.items():
        for alias in (
            name,
            relative.as_posix(),
            relative.name,
            name.replace(" ", "-"),
            installer.inventory_path(name).stem,
        ):
            aliases[alias.casefold()] = name
    return aliases


def edition_name(value: object) -> Optional[str]:
    """This clone's edition for a name ("PHP Core", "WordPress", "php-core"),
    an edition directory ("Cms/wordpress") or a path to one."""
    if not isinstance(value, str) or not value.strip():
        return None
    text = value.strip()
    named = edition_aliases().get(text.casefold().rstrip("/"))
    if named:
        return named
    candidate = Path(text).expanduser()
    for base in (Path.cwd(), ROOT):
        resolved = (candidate if candidate.is_absolute() else base / candidate).resolve()
        for name, relative in installer.EDITION_PATHS.items():
            if resolved == (ROOT / relative).resolve():
                return name
    return None


def detect_edition(project: Path) -> Tuple[str, str]:
    """The edition a project runs, and where that was read; read-only.

    The installer's sync manifest names it; then the policy lock the install
    copied; then the framework markers attached mode reads; else PHP Core.
    """
    manifest = read_json(project / INSTALL_MANIFEST)
    if isinstance(manifest, dict):
        named = edition_name(manifest.get("edition"))
        if named:
            return named, "install-manifest"
    lock = read_json(project / POLICY_LOCK)
    if isinstance(lock, dict):
        named = edition_name(lock.get("edition"))
        if named:
            return named, "policy-lock"
    detected, _evidence = accelerator_attach.detect_edition(project)
    named = edition_name(detected)
    if named:
        return named, "markers"
    return "PHP Core", "default"


class Edition:
    """One edition of this clone: its inventory, the rules to copy it, and a
    digest of the files an overlay installs, so a result names the runtime it
    measured even when the clone has uncommitted changes."""

    def __init__(self, name: str) -> None:
        self.name = name
        self.directory = ROOT / installer.edition_path(name)
        self.data = installer.load_inventory(name, ROOT)
        self.bits = installer.source_executable_bits(ROOT, name)
        self.files = installer.selected_files(self.data, list(installer.TOOLS))
        digest = hashlib.sha256()
        for _component, path in self.files:
            digest.update(path.encode("utf-8") + b"\0" + self.source(path).read_bytes() + b"\0")
        self.digest = digest.hexdigest()

    def source(self, path: str) -> Path:
        return self.directory / PurePosixPath(self.data["source_overrides"].get(path, path))

    def describe(self) -> Dict[str, Any]:
        status = read_git(ROOT, "status", "--porcelain", "--untracked-files=all", "--", self.directory.as_posix())
        return {
            "path": installer.edition_path(self.name).as_posix(),
            "release": self.data.get("release"),
            "digest": self.digest,
            "dirty": None if status is None else bool(status),
        }


_OWNED: Optional[Set[str]] = None
_FIRST_LINES: Optional[Dict[str, Set[str]]] = None


def accelerator_paths() -> Set[str]:
    """Every path any edition of this clone installs."""
    global _OWNED
    if _OWNED is None:
        owned: Set[str] = set()
        for name in installer.EDITIONS:
            for paths in installer.load_inventory(name, ROOT)["installed"].values():
                owned.update(paths)
        _OWNED = owned
    return _OWNED


def first_line(text: str) -> str:
    return text.lstrip("\ufeff").split("\n", 1)[0].strip()


def accelerator_first_lines() -> Dict[str, Set[str]]:
    global _FIRST_LINES
    if _FIRST_LINES is None:
        lines: Dict[str, Set[str]] = {path: set() for path in FIRST_LINE_FILES}
        for relative in installer.EDITION_PATHS.values():
            for path in FIRST_LINE_FILES:
                try:
                    line = first_line((ROOT / relative / path).read_text(encoding="utf-8"))
                except OSError:
                    continue
                if line:
                    lines[path].add(line)
        _FIRST_LINES = lines
    return _FIRST_LINES


def project_owned(path: str) -> bool:
    return path in PROJECT_FILES or path.startswith(PROJECT_TREES)


def overlay(corpus: Path, edition: Edition) -> Dict[str, int]:
    """Replace the corpus's accelerator with `edition`, keeping what the project owns."""
    counts = {"removed": 0, "copied": 0, "merged": 0, "kept": 0}
    for local in LOCAL_STATE:
        remove_tree(corpus / local)
    merged_files = installer.ADDITIVE_FILES | installer.MANAGED_POLICY_FILES | {"README.md"}
    # An older install - of any edition - may be committed: its copies go
    # first, so nothing the edition under test does not ship stays indexed.
    for path in sorted(accelerator_paths()):
        if project_owned(path) or path in merged_files:
            continue
        target = corpus / PurePosixPath(path)
        if target.is_symlink() or target.is_file():
            target.unlink()
            counts["removed"] += 1
        elif target.is_dir():
            remove_tree(target)
            counts["removed"] += 1
    lines = accelerator_first_lines()
    for _component, path in edition.files:
        source_path = edition.data["source_overrides"].get(path, path)
        source = edition.source(path)
        target = corpus / PurePosixPath(path)
        prepare_parent(corpus, target)
        if target.is_symlink():
            target.unlink()
        elif target.exists() and not target.is_file():
            remove_tree(target)
        payload = source.read_bytes()
        if target.is_file():
            if project_owned(path):
                counts["kept"] += 1
                continue
            existing = target.read_bytes().decode("utf-8", "replace")
            source_text = payload.decode("utf-8", "replace")
            if path in installer.ADDITIVE_FILES:
                payload = installer.merge_additive_file(existing, source_text).encode("utf-8")
                counts["merged"] += 1
            elif path in installer.MANAGED_POLICY_FILES:
                copied = first_line(existing) in lines.get(path, set())
                if installer.AGENTS_BEGIN in existing or not copied:
                    try:
                        payload = installer.merge_agents_file(existing, source_text).encode("utf-8")
                        counts["merged"] += 1
                    except installer.InventoryError:
                        counts["copied"] += 1  # a malformed managed block: the edition's file
                else:
                    counts["copied"] += 1  # an older accelerator copy: replaced
            elif path == "README.md" and first_line(existing) not in lines.get(path, set()):
                counts["kept"] += 1  # the project's own README
                continue
            else:
                counts["copied"] += 1
        else:
            counts["copied"] += 1
        target.write_bytes(payload)
        executable = installer.installs_executable(path, source, source_path, edition.bits)
        os.chmod(target, 0o755 if executable else 0o644)
    return counts


# --------------------------------------------------------------------------
# The corpus and the runtime
# --------------------------------------------------------------------------


def runtime_env(cache: Path, stamp: datetime, clock: Optional[datetime]) -> Dict[str, str]:
    env = {key: os.environ[key] for key in ENV_PASSTHROUGH if key in os.environ}
    home = cache / "home"
    epoch = f"{int(stamp.timestamp())} +0000"
    env.update(
        {
            "HOME": str(home),
            "XDG_CONFIG_HOME": str(home / ".config"),
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_CONFIG_GLOBAL": str(cache / "gitconfig"),
            "GIT_TERMINAL_PROMPT": "0",
            "GIT_NO_LAZY_FETCH": "1",
            "GIT_AUTHOR_NAME": IDENTITY[0],
            "GIT_AUTHOR_EMAIL": IDENTITY[1],
            "GIT_COMMITTER_NAME": IDENTITY[0],
            "GIT_COMMITTER_EMAIL": IDENTITY[1],
            "GIT_AUTHOR_DATE": epoch,
            "GIT_COMMITTER_DATE": epoch,
            "TZ": "UTC",
            "PYTHONHASHSEED": "0",
            "PYTHONDONTWRITEBYTECODE": "1",
            "PYTHONUTF8": "1",
            "PYTHONIOENCODING": "utf-8",
        }
    )
    if clock is not None:
        env["MEMORY_EVAL_CLOCK"] = clock.isoformat()
        env["PYTHONPATH"] = str(cache / "shim")
    return env


def commit_corpus(
    corpus: Path, env: Dict[str, str], parent: Optional[str], objects: Optional[Path], branch: str
) -> None:
    """`git init` and one commit of everything, on `branch` (and on main)."""

    def git(*arguments: str) -> str:
        result = subprocess.run(
            [
                "git", "-c", "core.autocrlf=false", "-c", "core.safecrlf=false", "-c", "gc.auto=0",
                "-c", "commit.gpgsign=false", "-c", "core.fsmonitor=false", *arguments,
            ],
            cwd=str(corpus),
            env=env,
            capture_output=True,
        )
        if result.returncode != 0:
            raise Skip("corpus-git-failed")
        return result.stdout.decode("utf-8", "replace").strip()

    git("init", "-q")
    git("symbolic-ref", "HEAD", "refs/heads/main")
    linked = bool(parent and objects)
    if linked:
        info = corpus / ".git" / "objects" / "info"
        info.mkdir(parents=True, exist_ok=True)
        (info / "alternates").write_text(str(objects) + "\n", encoding="utf-8")
    git("add", "-A")
    tree = git("write-tree")
    arguments = ["commit-tree", tree, "-m", "memory-eval corpus"]
    if linked:
        arguments += ["-p", str(parent)]
    commit = git(*arguments)
    git("update-ref", "refs/heads/main", commit)
    git("branch", branch)
    git("symbolic-ref", "HEAD", f"refs/heads/{branch}")


def run_runtime(corpus: Path, arguments: Sequence[str], env: Dict[str, str], timeout: float) -> Dict[str, Any]:
    command = [sys.executable, str(corpus / "memory-bank" / "scripts" / "context.py"), *arguments]
    started = time.monotonic()
    try:
        result = subprocess.run(command, cwd=str(corpus), env=env, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired as error:
        return {
            "exit": None,
            "seconds": round(time.monotonic() - started, 4),
            "stdout": error.stdout or b"",
            "stderr": error.stderr or b"",
        }
    return {
        "exit": result.returncode,
        "seconds": round(time.monotonic() - started, 4),
        "stdout": result.stdout,
        "stderr": result.stderr,
    }


def refresh_json(stdout: bytes) -> Optional[Dict[str, Any]]:
    text = stdout.decode("utf-8", "replace").strip()
    for candidate in [text] + [line for line in reversed(text.splitlines()) if line.startswith("{")]:
        try:
            value = json.loads(candidate)
        except ValueError:
            continue
        if isinstance(value, dict):
            return value
    return None


def quotes_prompt(line: str, prompt: str) -> bool:
    if not prompt:
        return False
    if len(prompt) < PROMPT_WINDOW:
        return prompt in line
    return any(line[start:start + PROMPT_WINDOW] in prompt for start in range(len(line) - PROMPT_WINDOW + 1))


def stderr_tail(raw: bytes, prompt: str) -> str:
    """The last lines of stderr, with any line that quotes the prompt withheld."""
    flat_prompt = flatten(prompt)
    lines = [line.rstrip() for line in raw.decode("utf-8", "replace").splitlines() if line.strip()]
    kept = [
        "[line withheld: it quotes the prompt]" if quotes_prompt(flatten(line), flat_prompt) else line[:240]
        for line in lines[-STDERR_LINES:]
    ]
    return "\n".join(kept)[-STDERR_CHARACTERS:]


# --------------------------------------------------------------------------
# Scoring
# --------------------------------------------------------------------------


def load_judgments(path: Path) -> Dict[str, Dict[str, int]]:
    """{prompt id: {path: grade}}; a grade that is not an integer is ignored."""
    raw = read_json(path)
    if not isinstance(raw, dict):
        raise EvalError(f"{path}: judgments must be a JSON object of {{prompt id: {{path: grade}}}}")
    judgments: Dict[str, Dict[str, int]] = {}
    for prompt_id, grades in raw.items():
        if not isinstance(grades, dict):
            continue
        clean: Dict[str, int] = {}
        for document, grade in grades.items():
            if not isinstance(document, str) or isinstance(grade, bool) or not isinstance(grade, int):
                continue
            key = normalise_path(document)
            clean[key] = max(grade, clean.get(key, grade))
        judgments[str(prompt_id)] = clean
    return judgments


def load_passages(path: Optional[Path]) -> Dict[str, Dict[str, Dict[str, Any]]]:
    """{prompt id: {path: {"useful": bool, "passages": [str, ...]}}}."""
    if path is None:
        return {}
    raw = read_json(path)
    if not isinstance(raw, dict):
        raise EvalError(f"{path}: passages must be a JSON object of {{prompt id: {{path: {{...}}}}}}")
    passages: Dict[str, Dict[str, Dict[str, Any]]] = {}
    for prompt_id, documents in raw.items():
        if not isinstance(documents, dict):
            continue
        passages[str(prompt_id)] = {
            normalise_path(document): entry
            for document, entry in documents.items()
            if isinstance(document, str) and isinstance(entry, dict)
        }
    return passages


def delivered_paths(capsule: object) -> List[str]:
    """Item paths of the three layers, in capsule order, each once."""
    found: List[str] = []
    if not isinstance(capsule, dict):
        return found
    for layer in LAYERS:
        items = capsule.get(layer)
        for item in items if isinstance(items, list) else []:
            if isinstance(item, dict) and isinstance(item.get("path"), str) and item["path"].strip():
                path = normalise_path(item["path"])
                if path not in found:
                    found.append(path)
    return found


def answer_in_text(useful: Sequence[str], passages: Dict[str, Dict[str, Any]], capsule_text: str) -> bool:
    """A labelled passage of a useful delivered document is in the text the model reads."""
    haystack = flatten(capsule_text)
    if not haystack:
        return False
    for path in useful:
        entry = passages.get(path)
        if not isinstance(entry, dict) or entry.get("useful") is False:
            continue
        for passage in entry.get("passages") or []:
            if isinstance(passage, str):
                needle = flatten(passage)
                if needle and needle in haystack:
                    return True
    return False


def classify(delivered: Sequence[str], useful: Sequence[str], noise: Sequence[str], withheld: bool) -> str:
    if withheld or not delivered:
        return "silent"
    if useful:
        return "useful"
    if noise:
        return "noise-only"
    return "unjudged-only"


def score(
    result: Optional[Dict[str, Any]],
    grades: Dict[str, int],
    passages: Dict[str, Dict[str, Any]],
    existed_useful: Sequence[str],
    answer_existed: Sequence[str] = (),
) -> Dict[str, Any]:
    """Score one refresh result. Paths and counts only: no capsule text."""
    result = result if isinstance(result, dict) else {}
    withheld = bool(result.get("query_withheld"))
    delivered = delivered_paths(result.get("capsule"))
    useful = [path for path in delivered if path in grades and grades[path] >= 1]
    noise = [path for path in delivered if path in grades and grades[path] < 1]
    unjudged = [path for path in delivered if path not in grades]
    text = result.get("capsule_text") if isinstance(result.get("capsule_text"), str) else ""
    return {
        "class": classify(delivered, useful, noise, withheld),
        "delivered": delivered,
        "useful_delivered": useful,
        "noise_delivered": noise,
        "unjudged": unjudged,
        "existed_useful": list(existed_useful),
        "could_help": bool(existed_useful),
        "answer_existed": list(answer_existed),
        "answer_could_help": bool(answer_existed),
        "answer_in_text": answer_in_text(useful, passages, text),
        "query_withheld": withheld,
        "capsule_chars": len(text),
    }


def corpus_file(corpus: Path, path: str) -> Optional[Path]:
    """The regular file a judged path names in the corpus, or None.

    A skill is judged under its canonical path; the overlay installs it under
    whichever tool directories the edition carries.
    """
    if not installer.is_safe_relative_path(path):
        return None
    candidates = [path]
    if path.startswith(CANON_SKILLS):
        candidates += [tool + path[len(CANON_SKILLS):] for tool in TOOL_SKILLS]
    for candidate in candidates:
        target = corpus / PurePosixPath(candidate)
        if target.is_file() and not target.is_symlink():
            return target
    return None


def existing_useful(corpus: Path, grades: Dict[str, int]) -> List[str]:
    """Judged-useful paths present in the corpus - skills included, only where
    the edition under test installed them."""
    return [path for path, grade in sorted(grades.items()) if grade >= 1 and corpus_file(corpus, path)]


def answer_existing(corpus: Path, grades: Dict[str, int], passages: Dict[str, Dict[str, Any]]) -> List[str]:
    """Judged-useful documents whose labelled answer was already in them.

    Judgments are per file, and a file can exist at the prompt without the
    part that answered it: a changelog's entry, a spec's section written for
    the very work the prompt started. A labelled passage present in the
    corpus's copy is the stricter ceiling for "answer in the capsule text".
    Skills count like any document, read from wherever the overlay put them,
    so the ceiling and the numerator cover the same documents.
    """
    found = []
    for path, grade in sorted(grades.items()):
        entry = passages.get(path)
        if grade < 1 or not isinstance(entry, dict) or entry.get("useful") is False:
            continue
        target = corpus_file(corpus, path)
        try:
            text = flatten(target.read_text(encoding="utf-8", errors="replace")) if target else ""
        except OSError:
            text = ""
        if text and any(isinstance(passage, str) and flatten(passage) and flatten(passage) in text
                        for passage in entry.get("passages") or []):
            found.append(path)
    return found


def percentile(values: Sequence[float], share: float) -> Optional[float]:
    if not values:
        return None
    ordered = sorted(values)
    return round(ordered[max(0, math.ceil(share * len(ordered)) - 1)], 4)


def summarize(items: Dict[str, Dict[str, Any]]) -> Dict[str, Any]:
    evaluated = [item for item in items.values() if item.get("status") == "ok"]
    skipped = Counter(str(item.get("reason") or "unknown") for item in items.values() if item.get("status") != "ok")
    classes = Counter(str(item.get("class")) for item in evaluated)
    count = len(evaluated)

    def mean(key: str) -> Optional[float]:
        return round(sum(len(item.get(key) or []) for item in evaluated) / count, 3) if count else None

    seconds = [
        float(item["refresh"]["seconds"])
        for item in evaluated
        if isinstance((item.get("refresh") or {}).get("seconds"), (int, float))
    ]
    return {
        "prompts": len(items),
        "evaluated": count,
        "skipped": dict(sorted(skipped.items())),
        "classes": {name: classes.get(name, 0) for name in CLASSES},
        "useful": classes.get("useful", 0),
        "could_help": sum(1 for item in evaluated if item.get("could_help")),
        "useful_among_could_help": sum(
            1 for item in evaluated if item.get("could_help") and item.get("class") == "useful"
        ),
        "answer_in_text": sum(1 for item in evaluated if item.get("answer_in_text")),
        "answer_could_help": sum(1 for item in evaluated if item.get("answer_could_help")),
        "contaminated": sum(1 for item in evaluated if item.get("contaminated")),
        "answer_in_text_among_answer_could_help": sum(
            1 for item in evaluated if item.get("answer_could_help") and item.get("answer_in_text")
        ),
        "noise_only": classes.get("noise-only", 0),
        "unjudged_only": classes.get("unjudged-only", 0),
        "silent": classes.get("silent", 0),
        "mean_delivered": mean("delivered"),
        "mean_noise": mean("noise_delivered"),
        "mean_unjudged": mean("unjudged"),
        "latency_p50": percentile(seconds, 0.50),
        "latency_p95": percentile(seconds, 0.95),
    }


# --------------------------------------------------------------------------
# run
# --------------------------------------------------------------------------


def load_set(path: Path) -> List[Dict[str, Any]]:
    raw = read_json(path)
    if not isinstance(raw, list):
        raise EvalError(f"{path}: the set must be a JSON list of prompt objects")
    prompts: List[Dict[str, Any]] = []
    seen: Set[str] = set()
    for index, entry in enumerate(raw):
        if not isinstance(entry, dict):
            raise EvalError(f"{path}: entry {index} is not an object")
        missing = [key for key in ("id", "project", "ts", "prompt") if key not in entry]
        if missing:
            raise EvalError(f"{path}: entry {index} lacks {', '.join(missing)}")
        prompt_id = entry["id"]
        if isinstance(prompt_id, bool) or not isinstance(prompt_id, (str, int)) or not str(prompt_id).strip():
            raise EvalError(f"{path}: entry {index} has an invalid id")
        if not all(isinstance(entry[key], str) for key in ("project", "ts", "prompt")):
            raise EvalError(f"{path}: entry {index}: project, ts and prompt must be strings")
        key = str(prompt_id)
        if key in seen:
            raise EvalError(f"{path}: duplicate id {key!r}")
        seen.add(key)
        prompts.append({"id": key, "project": entry["project"], "ts": entry["ts"], "prompt": entry["prompt"]})
    return prompts


def default_cache() -> Path:
    owner = str(os.getuid()) if hasattr(os, "getuid") else re.sub(r"\W+", "-", os.environ.get("USERNAME", "user"))
    return Path(tempfile.gettempdir()) / f"memory-eval-{owner}"


def prepare_cache(cache: Path) -> None:
    if inside_clone(cache):
        raise EvalError(f"--cache {cache} is inside this clone: corpora are copies of client projects")
    for directory in (cache, cache / "trees", cache / "runs", cache / "home", cache / "shim"):
        directory.mkdir(parents=True, exist_ok=True)
    try:
        os.chmod(cache, 0o700)
    except OSError:
        pass
    (cache / "gitconfig").write_text("", encoding="utf-8")
    (cache / "shim" / "sitecustomize.py").write_text(CLOCK_SHIM, encoding="utf-8")


def resolve_project(value: str, projects_root: Optional[Path]) -> Optional[Path]:
    candidate = Path(value).expanduser()
    if not candidate.is_absolute():
        if projects_root is None:
            return None
        candidate = projects_root.expanduser() / candidate
    candidate = candidate.resolve()
    return candidate if candidate.is_dir() else None


class Run:
    """One `run`: its settings, caches, and the output being written."""

    def __init__(self, arguments: argparse.Namespace) -> None:
        self.arguments = arguments
        self.prompts = load_set(arguments.set)
        self.judgments = load_judgments(arguments.judgments)
        self.passages = load_passages(arguments.passages)
        self.forced: Optional[str] = None
        if str(arguments.edition).strip().casefold() != "auto":
            self.forced = edition_name(arguments.edition)
            if self.forced is None:
                choices = ", ".join(f'"{name}"' for name in installer.EDITIONS)
                raise EvalError(f"--edition {arguments.edition!r} is not an edition of this clone ({choices}, or auto)")
        self.cache = (arguments.cache or default_cache()).expanduser().resolve()
        prepare_cache(self.cache)
        self.work = self.cache / "runs" / f"{time.strftime('%Y%m%dT%H%M%S')}-{os.getpid()}"
        self.work.mkdir(parents=True)
        self.editions: Dict[str, Edition] = {}
        self.detected: Dict[Path, Tuple[str, str]] = {}
        self.snapshots: Dict[Path, Path] = {}
        self.items: Dict[str, Dict[str, Any]] = {}
        self.meta: Dict[str, Any] = {
            "tool": "scripts/memory_eval.py",
            "edition": self.forced or "auto",
            "editions": {},
            "clone_commit": read_git(ROOT, "rev-parse", "HEAD"),
            "host": arguments.host,
            "as_of": arguments.as_of,
            "clock": "pinned" if self.pinned else "real",
            "updated_after": arguments.updated_after,
            "timeout": arguments.timeout,
            "set_sha256": sha256_file(arguments.set),
            "judgments_sha256": sha256_file(arguments.judgments),
            "passages_sha256": sha256_file(arguments.passages) if arguments.passages else None,
            "selection": {"ids": self.requested_ids(), "limit": arguments.limit},
            "python": platform.python_version(),
            "started": now_iso(),
            "finished": None,
        }

    @property
    def pinned(self) -> bool:
        return self.arguments.as_of == "prompt" and self.arguments.clock == "as-of"

    def requested_ids(self) -> Optional[List[str]]:
        if not self.arguments.ids:
            return None
        return [part.strip() for part in self.arguments.ids.split(",") if part.strip()]

    def selected(self) -> List[Dict[str, Any]]:
        prompts = self.prompts
        wanted = self.requested_ids()
        if wanted is not None:
            known = {prompt["id"] for prompt in prompts}
            for missing in [prompt_id for prompt_id in wanted if prompt_id not in known]:
                print(f"warning: --ids names {missing!r}, which the set does not have", file=sys.stderr)
            prompts = [prompt for prompt in prompts if prompt["id"] in set(wanted)]
        if self.arguments.limit is not None:
            prompts = prompts[: max(0, self.arguments.limit)]
        return prompts

    def edition(self, name: str) -> Edition:
        if name not in self.editions:
            self.editions[name] = Edition(name)
            self.meta["editions"][name] = self.editions[name].describe()
        return self.editions[name]

    def choose_edition(self, project: Path) -> Tuple[str, str]:
        if self.forced is not None:
            return self.forced, "forced"
        if project not in self.detected:
            self.detected[project] = detect_edition(project)
        return self.detected[project]

    def document(self) -> Dict[str, Any]:
        return {"meta": self.meta, "items": self.items, "summary": summarize(self.items)}

    def evaluate(self, prompt: Dict[str, Any]) -> Dict[str, Any]:
        item: Dict[str, Any] = {"project": prompt["project"], "ts": prompt["ts"], "status": "skipped"}
        corpus: Optional[Path] = None
        try:
            moment = parse_instant(prompt["ts"])
            if moment is None:
                raise Skip("bad-timestamp")
            project = resolve_project(prompt["project"], self.arguments.projects_root)
            if project is None:
                raise Skip("no-project")
            name, source = self.choose_edition(project)
            item.update(edition=name, edition_source=source)
            edition = self.edition(name)
            repository = Repository(project)
            corpus = self.work / f"{safe_name(prompt['id'])}-{hashlib.sha256(prompt['id'].encode()).hexdigest()[:8]}"
            parent: Optional[str] = None
            if self.arguments.as_of == "prompt":
                commit = repository.commit_before(moment) if repository.prefix is not None else None
                tree = repository.tree(commit) if commit else None
                if tree is None:
                    raise Skip("no-history")
                item["commit"] = commit
                shutil.copytree(materialize(project, tree, self.cache), corpus)
                item["provenance"] = reconstruct_memory(project, corpus, moment, self.arguments.updated_after)
                # Today's body of a document edited after the prompt is in the corpus.
                item["contaminated"] = bool(item["provenance"].get("updated_after"))
                # Configuration, not knowledge: when the commit lacks the
                # project's runtime.json (a retrieval gate, a privacy scope),
                # its working-tree copy beats the edition's defaults.
                config = PurePosixPath("project-brain/config/runtime.json")
                if not (corpus / config).is_file() and (project / config).is_file():
                    prepare_parent(corpus, corpus / config)
                    shutil.copy2(project / config, corpus / config)
                    item["provenance"]["config_from_worktree"] = 1
                parent = commit
            else:
                if project not in self.snapshots:
                    (self.work / "now").mkdir(exist_ok=True)
                    key = hashlib.sha256(str(project).encode()).hexdigest()[:16]
                    self.snapshots[project] = snapshot_worktree(project, self.work / "now" / key)
                shutil.copytree(self.snapshots[project], corpus)
                item["commit"] = repository.head() if repository.prefix is not None else None
                item["provenance"] = count_present(corpus, moment)
                parent = item["commit"]
            item["overlay"] = overlay(corpus, edition)
            grades = self.judgments.get(prompt["id"], {})
            existed = existing_useful(corpus, grades)
            answered = answer_existing(corpus, grades, self.passages.get(prompt["id"], {}))
            env = runtime_env(self.cache, moment, moment if self.pinned else None)
            objects = repository.history(self.cache, parent) if parent else None
            branch = f"eval/{safe_name(prompt['id'])}"
            commit_corpus(corpus, env, parent if objects else None, objects, branch)
            item["history_linked"] = objects is not None
            # A fresh corpus starts with no index; the hook's index is warm.
            # Building it first keeps the timed refresh the per-turn cost.
            warm = run_runtime(corpus, ["index"], env, self.arguments.timeout)
            refresh = run_runtime(
                corpus,
                [
                    "refresh", "--host", self.arguments.host, "--query", prompt["prompt"],
                    "--task-id", branch, "--ephemeral", "--sanitize", "--json",
                ],
                env,
                self.arguments.timeout,
            )
            result = refresh_json(refresh["stdout"])
            item["refresh"] = {
                "exit": refresh["exit"],
                "seconds": refresh["seconds"],
                "index_seconds": warm["seconds"],
                "index_exit": warm["exit"],
                "phases": {
                    key: value
                    for key, value in ((result or {}).get("phases") or {}).items()
                    if isinstance(value, (int, float))
                },
                "warnings": len((result or {}).get("warnings") or []),
                "capsule_missing": bool(result) and result.get("capsule") is None and not result.get("query_withheld"),
                "stderr_tail": stderr_tail(refresh["stderr"], prompt["prompt"]),
            }
            if result is None:
                raise Skip("refresh-timeout" if refresh["exit"] is None else "refresh-error")
            item.update(score(result, grades, self.passages.get(prompt["id"], {}), existed, answered))
            item["status"] = "ok"
        except Skip as skip:
            item["reason"] = skip.reason
        except (OSError, shutil.Error, subprocess.SubprocessError) as error:
            item["reason"] = f"error-{type(error).__name__}"
        finally:
            if corpus is not None and corpus.exists():
                if self.arguments.keep:
                    item["corpus"] = str(corpus)
                else:
                    remove_tree(corpus)
        return item

    def execute(self) -> int:
        selected = self.selected()
        out = self.arguments.out
        try:
            for index, prompt in enumerate(selected, 1):
                item = self.evaluate(prompt)
                self.items[prompt["id"]] = item
                write_json(out, self.document())
                if item["status"] == "ok":
                    note = f"{item['class']}  delivered={len(item['delivered'])}  {item['refresh']['seconds']:.2f}s"
                else:
                    note = f"skipped: {item.get('reason')}"
                print(f"[{index}/{len(selected)}] {prompt['id']}  {note}", file=sys.stderr)
        except KeyboardInterrupt:
            self.meta["interrupted"] = True
            write_json(out, self.document())
            raise
        finally:
            if not self.arguments.keep:
                remove_tree(self.work)
        self.meta["finished"] = now_iso()
        write_json(out, self.document())
        print(format_report([("result", out, self.document())]))
        print(f"wrote {out}")
        evaluated = sum(1 for item in self.items.values() if item.get("status") == "ok")
        # Nothing evaluated - an empty set, ids that match nothing, every
        # prompt skipped - is not a measurement, whatever was selected.
        return 0 if evaluated else 1


def command_run(arguments: argparse.Namespace) -> int:
    if arguments.timeout <= 0:
        raise EvalError("--timeout must be positive")
    for label, path in (("--set", arguments.set), ("--judgments", arguments.judgments),
                        ("--passages", arguments.passages), ("--out", arguments.out)):
        warn_if_inside_clone(label, path)
    return Run(arguments).execute()


# --------------------------------------------------------------------------
# report
# --------------------------------------------------------------------------


def load_result(path: Path) -> Dict[str, Any]:
    document = read_json(path)
    if not isinstance(document, dict) or not isinstance(document.get("items"), dict):
        raise EvalError(f"{path}: not a memory_eval result (no items)")
    return document


def _share(part: int, whole: int) -> str:
    return f"{part} ({100.0 * part / whole:.1f}%)" if whole else str(part)


ROWS: Tuple[Tuple[str, str], ...] = (
    ("prompts", "prompts"),
    ("evaluated", "evaluated"),
    ("skipped", "skipped_total"),
    ("useful turns", "useful"),
    ("could help (a useful document existed)", "could_help"),
    ("useful among could-help", "useful_among_could_help"),
    ("answer in the capsule text", "answer_in_text"),
    ("could answer (a labelled answer existed)", "answer_could_help"),
    ("contaminated (today's body of a later edit)", "contaminated"),
    ("answer in text among could-answer", "answer_in_text_among_answer_could_help"),
    ("noise-only turns", "noise_only"),
    ("unjudged-only turns", "unjudged_only"),
    ("silent turns", "silent"),
    ("mean delivered per turn", "mean_delivered"),
    ("mean noise per turn", "mean_noise"),
    ("mean unjudged per turn", "mean_unjudged"),
    ("latency p50 (s)", "latency_p50"),
    ("latency p95 (s)", "latency_p95"),
)
SHARED_ROWS = {"useful", "answer_in_text", "noise_only", "unjudged_only", "silent", "could_help",
               "answer_could_help", "contaminated"}


def row_value(summary: Dict[str, Any], key: str) -> Optional[float]:
    if key == "skipped_total":
        return float(sum(summary.get("skipped", {}).values()))
    value = summary.get(key)
    return float(value) if isinstance(value, (int, float)) else None


def row_cell(summary: Dict[str, Any], key: str) -> str:
    value = row_value(summary, key)
    if value is None:
        return "-"
    if key in SHARED_ROWS:
        return _share(int(value), int(summary.get("evaluated") or 0))
    if key == "useful_among_could_help":
        return f"{int(value)}/{int(summary.get('could_help') or 0)}"
    if key == "answer_in_text_among_answer_could_help":
        return f"{int(value)}/{int(summary.get('answer_could_help') or 0)}"
    if key.startswith("latency"):
        return f"{value:.3f}"
    if key.startswith("mean"):
        return f"{value:.2f}"
    return str(int(value))


def describe_meta(meta: Dict[str, Any], items: Dict[str, Dict[str, Any]]) -> str:
    edition = str(meta.get("edition"))
    if edition == "auto":
        used = Counter(str(item.get("edition")) for item in items.values() if item.get("edition"))
        edition += " (" + ", ".join(f"{name} {count}" for name, count in sorted(used.items())) + ")"
    commit = str(meta.get("clone_commit") or "?")[:10]
    dirty = any((info or {}).get("dirty") for info in (meta.get("editions") or {}).values())
    return (
        f"edition={edition} as_of={meta.get('as_of')} clock={meta.get('clock')} "
        f"host={meta.get('host')} clone={commit}{' (edition dirty)' if dirty else ''}"
    )


def format_report(results: Sequence[Tuple[str, Path, Dict[str, Any]]]) -> str:
    summaries = [summarize(document["items"]) for _, _, document in results]
    lines = []
    for (label, path, document), _summary in zip(results, summaries):
        lines.append(f"{label}: {path}  {describe_meta(document.get('meta') or {}, document['items'])}")
    width = max(len(title) for title, _ in ROWS) + 2
    cell = 18
    header = " " * width + "".join(label.rjust(cell) for label, _, _ in results)
    if len(results) == 2:
        header += "delta".rjust(cell)
    lines += ["", header]
    for title, key in ROWS:
        row = title.ljust(width) + "".join(row_cell(summary, key).rjust(cell) for summary in summaries)
        if len(results) == 2:
            first, second = (row_value(summary, key) for summary in summaries)
            if first is not None and second is not None:
                delta = second - first
                row += (f"{delta:+.3f}" if key.startswith(("mean", "latency")) else f"{int(delta):+d}").rjust(cell)
            else:
                row += "-".rjust(cell)
        lines.append(row)
    for (label, _, _), summary in zip(results, summaries):
        if summary["skipped"]:
            reasons = ", ".join(f"{reason} {count}" for reason, count in summary["skipped"].items())
            lines.append(f"skipped in {label}: {reasons}")
    return "\n".join(lines)


def paired_diff(
    first: Dict[str, Dict[str, Any]], second: Dict[str, Dict[str, Any]]
) -> Tuple[int, Dict[str, Tuple[List[str], List[str]]]]:
    """Prompts evaluated in both, and per measure the ids B gained and lost."""
    ids = [
        prompt_id
        for prompt_id, item in first.items()
        if item.get("status") == "ok" and (second.get(prompt_id) or {}).get("status") == "ok"
    ]
    tests: Dict[str, Callable[[Dict[str, Any]], bool]] = {
        "useful": lambda item: item.get("class") == "useful",
        "noise-only": lambda item: item.get("class") == "noise-only",
        "answer": lambda item: bool(item.get("answer_in_text")),
    }
    changes = {}
    for name, test in tests.items():
        gained = [prompt_id for prompt_id in ids if test(second[prompt_id]) and not test(first[prompt_id])]
        lost = [prompt_id for prompt_id in ids if test(first[prompt_id]) and not test(second[prompt_id])]
        changes[name] = (gained, lost)
    return len(ids), changes


def unjudged_pairs(items: Dict[str, Dict[str, Any]]) -> List[Tuple[str, str]]:
    return sorted(
        {(prompt_id, path) for prompt_id, item in items.items() for path in item.get("unjudged") or []}
    )


def command_report(arguments: argparse.Namespace) -> int:
    first = load_result(arguments.result)
    results: List[Tuple[str, Path, Dict[str, Any]]] = [("A", arguments.result, first)]
    if arguments.compare is not None:
        results.append(("B", arguments.compare, load_result(arguments.compare)))
    print(format_report(results))
    if len(results) == 2:
        second = results[1][2]
        meta_a, meta_b = first.get("meta") or {}, second.get("meta") or {}
        for key in ("set_sha256", "judgments_sha256", "passages_sha256"):
            if meta_a.get(key) != meta_b.get(key):
                print(f"note: {key} differs between A and B; the pairing is by prompt id only")
        count, changes = paired_diff(first["items"], second["items"])
        print("")
        print(f"paired over {count} prompt(s) evaluated in both (B against A)")
        for name, (gained, lost) in changes.items():
            print(f"  {name:<11} +{len(gained)} -{len(lost)}")
            if gained:
                print(f"    gained: {', '.join(gained)}")
            if lost:
                print(f"    lost:   {', '.join(lost)}")
    if arguments.show_unjudged:
        pairs = sorted(set().union(*(unjudged_pairs(document["items"]) for _, _, document in results)))
        print("")
        print(f"{len(pairs)} delivered (id, path) pair(s) without a judgment:")
        for prompt_id, path in pairs:
            print(f"{prompt_id}\t{path}")
    return 0


# --------------------------------------------------------------------------
# realized: do agents use what the hook delivered?
# --------------------------------------------------------------------------

DOC_PATH = re.compile(
    r"(?<![\w-])((?:project-brain|memory-bank|specs|docs|tasks|\.agents/skills|\.claude/skills"
    r"|\.cursor/skills|\.codex/skills)/[\w./@-]+\.md|CHANGELOG\.md)"
)
KINDS = ("skill", "brain", "chunk", "task", "spec-doc", "changelog")
# The first line of a capsule as a hook prints it: the old banner, the
# working line, or a layer/health line printed ahead of it when unhealthy.
CAPSULE_START = re.compile(
    r"^(?:Memory refresh|working:|warming:|warning:|procedural:|semantic:|episodic:"
    r"|memory review:|codebase map:|phases:|brain-validation:)"
)
GENERIC_NAMES = {"skill.md", "readme.md", "index.md", "manifest.md"}
COUNTERS = (
    "hook_turns", "turns_with_capsule", "turns_with_documents", "delivered_items",
    "delivered_touched", "delivered_mentioned", "turns_touching_delivered",
    "turns_mentioning_delivered", "touched_not_delivered", "turns_touching_other_docs",
    "agent_retrieve_cli", "agent_retrieve_mcp", "agent_context_cli",
)


def doc_kind(path: str) -> str:
    if is_skill(path):
        return "skill"
    if path.startswith("project-brain/"):
        return "brain"
    if path.startswith("memory-bank/chunks/"):
        return "chunk"
    if path.startswith("tasks/"):
        return "task"
    if path == "CHANGELOG.md":
        return "changelog"
    return "spec-doc"


def doc_paths(text: str) -> Set[str]:
    return {normalise_path(match) for match in DOC_PATH.findall(text.replace("\\/", "/"))}


def mention_keys(path: str) -> List[str]:
    """What names a document in prose: its path, its file name, or - for a
    generic file name like SKILL.md - its folder; a chunk's MEM id, a
    record's UUID."""
    pure = PurePosixPath(path)
    keys = [path]
    if pure.name.casefold() in GENERIC_NAMES:
        if pure.parent.name:
            keys.append(pure.parent.name)
        return keys
    keys.append(pure.name)
    memory = re.match(r"MEM-\d{8}-[0-9a-f]{8}", pure.stem)
    if memory:
        keys.append(memory.group(0))
    elif re.fullmatch(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", pure.stem):
        keys.append(pure.stem)
    return keys


def mentioned(path: str, text: str) -> bool:
    return any(
        re.search(r"(?<![\w-])" + re.escape(key) + r"(?![\w-])", text) for key in mention_keys(path)
    )


def tool_calls(blob: str, name: str, turn: Dict[str, Any]) -> None:
    """Count the memory calls one tool call makes."""
    if name.endswith("memory_retrieve"):
        turn["retrieve_mcp"] += 1
    for match in re.finditer(r"context\.py", blob):
        turn["context_cli"] += 1
        tail = re.sub(r"^[\s\"',\\\]\[]+", "", blob[match.end():match.end() + 80])
        if re.match(r"(?:retrieve|context)\b", tail):
            turn["retrieve_cli"] += 1


def new_turn(started: object) -> Dict[str, Any]:
    return {
        "started": parse_instant(started),
        "output": False,
        "delivered": set(),
        "touched": set(),
        "text": [],
        "active": False,
        "retrieve_cli": 0,
        "retrieve_mcp": 0,
        "context_cli": 0,
    }


def read_records(path: Path) -> Iterator[Dict[str, Any]]:
    try:
        with path.open(encoding="utf-8", errors="replace") as handle:
            for line in handle:
                try:
                    record = json.loads(line)
                except ValueError:
                    continue
                if isinstance(record, dict):
                    yield record
    except OSError:
        return


def claude_turns(path: Path) -> List[Dict[str, Any]]:
    """Turns of one Claude Code transcript: each starts at the prompt hook's
    output; the agent's tool calls and text belong to it until the next
    human prompt. Hook outputs before any agent activity share one turn."""
    turns: List[Dict[str, Any]] = []
    current: Optional[Dict[str, Any]] = None
    for record in read_records(path):
        kind = record.get("type")
        if kind == "attachment":
            attachment = record.get("attachment")
            if (
                isinstance(attachment, dict)
                and attachment.get("type") == "hook_success"
                and attachment.get("hookEvent") == "UserPromptSubmit"
            ):
                if current is None or current["active"]:
                    current = new_turn(record.get("timestamp"))
                    turns.append(current)
                stdout = str(attachment.get("stdout") or "")
                content = attachment.get("content")
                content_text = content if isinstance(content, str) else (json.dumps(content) if content else "")
                current["output"] = current["output"] or bool(stdout.strip() or content_text.strip())
                current["delivered"] |= doc_paths(stdout + "\n" + content_text)
            continue
        if current is None:
            continue
        message = record.get("message") if isinstance(record.get("message"), dict) else {}
        if kind == "user":
            if not record.get("toolUseResult") and not record.get("isMeta") and isinstance(message.get("content"), str):
                if current["active"]:
                    current = None
            continue
        if kind != "assistant":
            continue
        current["active"] = True
        for part in message.get("content") or []:
            if not isinstance(part, dict):
                continue
            if part.get("type") == "tool_use":
                blob = json.dumps(part.get("input") or {})
                current["touched"] |= doc_paths(blob)
                tool_calls(blob, str(part.get("name") or ""), current)
            elif part.get("type") == "text" and isinstance(part.get("text"), str):
                current["text"].append(part["text"])
    return turns


def codex_turns(path: Path) -> List[Dict[str, Any]]:
    """Turns of one Codex rollout: each starts at a developer message that is
    the prompt hook's output (the capsule), as in claude_turns."""
    turns: List[Dict[str, Any]] = []
    current: Optional[Dict[str, Any]] = None
    for record in read_records(path):
        if record.get("type") != "response_item":
            continue
        payload = record.get("payload") if isinstance(record.get("payload"), dict) else {}
        item_type = payload.get("type")
        if item_type == "message":
            text = "\n".join(
                part["text"]
                for part in payload.get("content") or []
                if isinstance(part, dict) and isinstance(part.get("text"), str)
            )
            role = payload.get("role")
            if role == "developer" and CAPSULE_START.match(text.lstrip()):
                if current is None or current["active"]:
                    current = new_turn(record.get("timestamp"))
                    turns.append(current)
                current["output"] = True
                current["delivered"] |= doc_paths(text)
            elif role == "user" and not text.lstrip().startswith("<"):
                if current is not None and current["active"]:
                    current = None
            elif role == "assistant" and current is not None:
                current["active"] = True
                current["text"].append(text)
        elif item_type in ("function_call", "custom_tool_call", "local_shell_call") and current is not None:
            current["active"] = True
            raw = payload.get("arguments") or payload.get("input") or payload.get("action") or ""
            blob = raw if isinstance(raw, str) else json.dumps(raw)
            current["touched"] |= doc_paths(blob)
            tool_calls(blob, str(payload.get("name") or ""), current)
    return turns


def tally(turns: Sequence[Dict[str, Any]], paths: Optional[Dict[str, Counter]] = None) -> Dict[str, Any]:
    stats: Counter = Counter()
    by_kind = {kind: Counter() for kind in KINDS}
    for turn in turns:
        stats["hook_turns"] += 1
        stats["turns_with_capsule"] += bool(turn["output"])
        stats["agent_retrieve_cli"] += turn["retrieve_cli"]
        stats["agent_retrieve_mcp"] += turn["retrieve_mcp"]
        stats["agent_context_cli"] += turn["context_cli"]
        delivered = turn["delivered"]
        if not delivered:
            continue
        stats["turns_with_documents"] += 1
        text = "\n".join(turn["text"])
        touched = delivered & turn["touched"]
        said = {path for path in delivered if mentioned(path, text)}
        stats["delivered_items"] += len(delivered)
        stats["delivered_touched"] += len(touched)
        stats["delivered_mentioned"] += len(said)
        stats["turns_touching_delivered"] += bool(touched)
        stats["turns_mentioning_delivered"] += bool(said)
        for path in delivered:
            kind = doc_kind(path)
            by_kind[kind]["delivered"] += 1
            by_kind[kind]["touched"] += path in touched
            by_kind[kind]["mentioned"] += path in said
            if paths is not None:
                counter = paths.setdefault(path, Counter())
                counter["delivered"] += 1
                counter["touched"] += path in touched
                counter["mentioned"] += path in said
        # The baseline: documents of the same kinds the agent opened that
        # the capsule did not deliver.
        other = {path for path in turn["touched"] if path not in delivered}
        stats["touched_not_delivered"] += len(other)
        stats["turns_touching_other_docs"] += bool(other)
    summary: Dict[str, Any] = {name: stats.get(name, 0) for name in COUNTERS}
    summary["by_kind"] = {
        kind: {measure: by_kind[kind].get(measure, 0) for measure in ("delivered", "touched", "mentioned")}
        for kind in KINDS
    }
    return summary


class Membership:
    """Whether a session's working directory is the project, inside it, or
    another worktree of the same repository."""

    def __init__(self, project: Path) -> None:
        self.project = project
        self.common = self.common_dir(project)
        self.known: Dict[str, bool] = {}

    @staticmethod
    def common_dir(directory: Path) -> Optional[Path]:
        common = read_git(directory, "rev-parse", "--git-common-dir")
        return (directory / common).resolve() if common else None

    def __call__(self, cwd: object) -> bool:
        if not isinstance(cwd, str) or not cwd:
            return False
        if cwd not in self.known:
            path = Path(cwd)
            answer = path == self.project or self.project in path.parents
            if not answer and self.common is not None and path.is_dir():
                answer = self.common_dir(path) == self.common
            self.known[cwd] = answer
        return self.known[cwd]


def first_cwd(folder: Path) -> Optional[str]:
    for transcript in sorted(folder.glob("*.jsonl")):
        for index, record in enumerate(read_records(transcript)):
            if isinstance(record.get("cwd"), str):
                return record["cwd"]
            if index > 200:
                break
    return None


def claude_sources(root: Path, project: Path, member: Membership) -> Tuple[List[Path], int, int]:
    """Transcripts of the project's Claude Code folders: its own, and the
    worktree variants whose sessions ran in it; a same-prefix folder of
    another project (`next-gen` beside `next`) is skipped and counted."""
    mangled = re.sub(r"[^A-Za-z0-9]", "-", str(project))
    folders: List[Path] = []
    skipped = 0
    if root.is_dir():
        for folder in sorted(root.iterdir()):
            name = folder.name
            if not folder.is_dir():
                continue
            if name == mangled or name.startswith(mangled + "--claude-worktrees-"):
                folders.append(folder)
            elif name.startswith(mangled + "-"):
                if member(first_cwd(folder)):
                    folders.append(folder)
                else:
                    skipped += 1
    files = [transcript for folder in folders for transcript in sorted(folder.glob("*.jsonl"))]
    return files, len(folders), skipped


def codex_sources(root: Path, member: Membership) -> List[Path]:
    found = []
    if root.is_dir():
        for rollout in sorted(root.glob("*/*/*/*.jsonl")):
            for record in read_records(rollout):
                payload = record.get("payload") if isinstance(record.get("payload"), dict) else {}
                if member(payload.get("cwd")):
                    found.append(rollout)
                break
    return found


def command_realized(arguments: argparse.Namespace) -> int:
    project = Path(arguments.project).expanduser().resolve()
    if not project.is_dir():
        raise EvalError(f"--project {arguments.project} is not a directory")
    since: Optional[datetime] = None
    if arguments.since:
        day = parse_day(arguments.since)
        if day is None or len(arguments.since.strip()) != 10:
            raise EvalError("--since must be YYYY-MM-DD")
        since = day_window(day)[0]
    member = Membership(project)
    claude_root = Path(arguments.claude_root).expanduser()
    codex_root = Path(arguments.codex_root).expanduser()
    claude_files, folders, skipped_folders = claude_sources(claude_root, project, member)
    codex_files = codex_sources(codex_root, member)

    def recent(path: Path) -> bool:
        return since is None or modified_at(path) >= since

    def kept(turns: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
        return [turn for turn in turns if since is None or turn["started"] is None or turn["started"] >= since]

    paths: Optional[Dict[str, Counter]] = {} if arguments.paths else None
    claude_used = [path for path in claude_files if recent(path)]
    codex_used = [path for path in codex_files if recent(path)]
    claude_all = [turn for path in claude_used for turn in kept(claude_turns(path))]
    codex_all = [turn for path in codex_used for turn in kept(codex_turns(path))]
    report: Dict[str, Any] = {
        "project": str(project),
        "since": arguments.since,
        "claude": {
            "transcripts": len(claude_used),
            "folders": folders,
            "folders_skipped": skipped_folders,
            **tally(claude_all, paths),
        },
        "codex": {"transcripts": len(codex_used), **tally(codex_all, paths)},
        "total": {"transcripts": len(claude_used) + len(codex_used), **tally(claude_all + codex_all)},
    }
    if paths is not None:
        report["paths"] = {
            path: dict(counter)
            for path, counter in sorted(paths.items(), key=lambda entry: (-entry[1]["delivered"], entry[0]))
        }
    if arguments.json:
        print(json.dumps(report, indent=1, ensure_ascii=False))
        return 0
    print(format_realized(report))
    return 0


REALIZED_ROWS = (
    ("transcripts read", "transcripts"),
    ("prompt-hook turns", "hook_turns"),
    ("turns with a capsule", "turns_with_capsule"),
    ("turns with documents", "turns_with_documents"),
    ("delivered items", "delivered_items"),
    ("  touched by the agent", "delivered_touched"),
    ("  mentioned in its text", "delivered_mentioned"),
    ("turns touching a delivered item", "turns_touching_delivered"),
    ("turns mentioning a delivered item", "turns_mentioning_delivered"),
    ("other same-kind docs touched", "touched_not_delivered"),
    ("turns touching other docs", "turns_touching_other_docs"),
    ("agent retrieve calls, CLI", "agent_retrieve_cli"),
    ("agent retrieve calls, MCP", "agent_retrieve_mcp"),
    ("agent context.py calls", "agent_context_cli"),
)


def format_realized(report: Dict[str, Any]) -> str:
    hosts = ("claude", "codex", "total")
    since = f" since {report['since']}" if report.get("since") else ""
    lines = [f"Realized use of delivered memory: {report['project']}{since}"]
    skipped = report["claude"].get("folders_skipped")
    if skipped:
        lines.append(f"({skipped} Claude Code folder(s) with the same name prefix belong to another project; skipped)")
    width = max(len(title) for title, _ in REALIZED_ROWS) + 2
    lines.append(" " * width + "".join(host.rjust(9) for host in hosts))
    for title, key in REALIZED_ROWS:
        lines.append(title.ljust(width) + "".join(str(report[host].get(key, 0)).rjust(9) for host in hosts))
    lines.append("")
    lines.append("by kind, all hosts: delivered / touched / mentioned")
    for kind in KINDS:
        counts = report["total"]["by_kind"][kind]
        if counts["delivered"]:
            lines.append(f"  {kind:<10} {counts['delivered']:>6} {counts['touched']:>6} {counts['mentioned']:>6}")
    if "paths" in report:
        lines.append("")
        lines.append("by path: delivered / touched / mentioned")
        for path, counts in report["paths"].items():
            lines.append(
                f"  {counts.get('delivered', 0):>5} {counts.get('touched', 0):>5}"
                f" {counts.get('mentioned', 0):>5}  {path}"
            )
    return "\n".join(lines)


# --------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="memory_eval.py",
        description="Measure the memory capsule on each project as it was when the prompt was written.",
    )
    commands = parser.add_subparsers(dest="command", required=True)

    run = commands.add_parser("run", help="rebuild each project as of its prompt, refresh and score")
    run.add_argument("--set", type=Path, required=True, help="JSON list of {id, project, ts, prompt}")
    run.add_argument("--judgments", type=Path, required=True, help="JSON {prompt id: {path: grade 0|1|2}}")
    run.add_argument("--passages", type=Path, help='JSON {prompt id: {path: {"useful": bool, "passages": [...]}}}')
    run.add_argument("--projects-root", type=Path, help="directory holding the projects named in the set")
    run.add_argument(
        "--edition",
        default="auto",
        help='edition under test: "auto" (per project, the default), a name or an edition directory of this clone',
    )
    run.add_argument("--out", type=Path, required=True, help="result JSON (ids, paths and counts only)")
    run.add_argument("--host", choices=HOSTS, default="claude")
    run.add_argument("--limit", type=int, help="evaluate the first N selected prompts")
    run.add_argument("--ids", help="comma-separated prompt ids to evaluate")
    run.add_argument(
        "--as-of",
        dest="as_of",
        choices=("prompt", "now"),
        default="prompt",
        help="prompt: each project as it was at the prompt (default); now: the current tree, the old leaky measurement",
    )
    run.add_argument(
        "--clock",
        choices=("as-of", "real"),
        default="as-of",
        help="as-of: the runtime's clock reads the prompt's instant (default); real: today's",
    )
    run.add_argument(
        "--updated-after",
        choices=("drop", "keep"),
        default="drop",
        help=(
            "a working-tree memory document created before the prompt but edited after it: "
            "drop it (default, the strict as-of corpus) or keep today's body and mark the turn contaminated"
        ),
    )
    run.add_argument("--cache", type=Path, help=f"work and cache directory (default {default_cache()})")
    run.add_argument("--timeout", type=float, default=DEFAULT_TIMEOUT, help="seconds per runtime call")
    run.add_argument("--keep", action="store_true", help="keep each prompt's corpus for debugging")
    run.set_defaults(handler=command_run)

    report = commands.add_parser("report", help="aggregate a result, optionally against another")
    report.add_argument("result", type=Path)
    report.add_argument("--compare", type=Path, help="a second result: side by side, plus a paired per-prompt diff")
    report.add_argument(
        "--show-unjudged", action="store_true", help="list delivered (id, path) pairs without a judgment"
    )
    report.set_defaults(handler=command_report)

    realized = commands.add_parser("realized", help="do agents use what the prompt hook delivered?")
    realized.add_argument("--project", required=True, help="the project directory")
    realized.add_argument("--claude-root", default="~/.claude/projects")
    realized.add_argument("--codex-root", default="~/.codex/sessions")
    realized.add_argument("--since", help="only turns from this UTC date on (YYYY-MM-DD)")
    realized.add_argument("--json", action="store_true")
    realized.add_argument("--paths", action="store_true", help="also list delivered document paths")
    realized.set_defaults(handler=command_realized)
    return parser


def main(argv: Optional[Sequence[str]] = None) -> int:
    arguments = build_parser().parse_args(argv)
    try:
        return int(arguments.handler(arguments))
    except EvalError as error:
        print(f"memory_eval: {error}", file=sys.stderr)
        return 2
    except installer.InventoryError as error:
        print(f"memory_eval: {error}", file=sys.stderr)
        return 2
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    sys.exit(main())
