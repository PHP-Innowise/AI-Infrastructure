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
        [--codex-root DIR] [--state DIR] [--since YYYY-MM-DD] [--json] [--paths]

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
import importlib.util
import json
import math
import os
import platform
import re
import statistics
import shutil
import sqlite3
import stat
import subprocess
import sys
import tempfile
import time
from collections import Counter
from datetime import date, datetime, timedelta, timezone
from pathlib import Path, PurePosixPath
from typing import Any, Callable, Dict, Iterator, List, Mapping, Optional, Pattern, Sequence, Set, Tuple

ROOT = Path(__file__).resolve().parent.parent
if str(ROOT / "scripts") not in sys.path:
    sys.path.insert(0, str(ROOT / "scripts"))

import accelerator_attach  # noqa: E402  - edition detection from framework markers
import install_accelerator as installer  # noqa: E402  - inventories and install rules
from memory_diagnosis import normalize_funnel_diagnosis, normalize_ordinary_receipt  # noqa: E402

UTC = timezone.utc
HOSTS = ("claude", "codex", "cursor")
LAYERS = ("semantic", "episodic", "procedural")
# What a refresh reports per layer; anything else is recorded as "other".
LAYER_STATES = ("updated", "failed")
GRADES = (0, 1, 2)
# The one key of a prompt's judgments that is not a path: the reviewer's note
# on why the grades are what they are, when it is text.
NOTE_KEY = "_note"
CLASSES = ("useful", "noise-only", "unjudged-only", "silent")
CANON_SKILLS = ".agents/skills/"
TOOL_SKILLS = (".claude/skills/", ".cursor/skills/", ".codex/skills/")
RECORDS = "project-brain/dynamic"
ARCHIVE = "project-brain/archive"
ARCHIVE_COUNTS = ("archive_from_git", "archive_from_worktree", "archive_dropped_future", "archive_undetermined",
                  "archive_unreconstructable_updated", "archive_updated_after", "archive_still_dynamic")
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
REFRESH_RETRIEVAL_LIMIT = 3
IDENTITY = ("memory-eval", "memory-eval@localhost")
# The refresh runs with these variables of the caller's environment and no
# others, so neither an attached-mode session, a task override nor a Git
# redirection of the caller can reach the corpus.
ENV_PASSTHROUGH = (
    "PATH", "LANG", "LC_ALL", "LC_CTYPE", "TMPDIR", "TEMP", "TMP", "SYSTEMROOT",
    "SYSTEMDRIVE", "COMSPEC", "PATHEXT", "WINDIR", "USER", "LOGNAME", "USERNAME",
    # Explicit maintainer pilot; absent by default.
    "CONTEXT_EVENT_SERIES",
)
STDERR_LINES = 6
STDERR_CHARACTERS = 600
PROMPT_WINDOW = 20
MISS_AUDIT_STAGES = frozenset({
    "absent-source", "not-indexed", "no-fts-candidate", "window-limit",
    "relevance-drop", "floor-drop", "runtime-exclusion", "rank-loss",
    "budget-loss", "capsule-loss", "selected", "selected-answer-not-in-text",
    "unknown", "unknown-history", "unknown-query-withheld",
    "history-event-limit", "history-admission-drop", "automatic-link-limit",
})
MISS_AUDIT_ANSWERS = frozenset({
    "not-applicable", "unknown", "not-in-source", "not-in-index", "not-in-delivery",
    "partial-index", "partial-delivery", "delivered",
})

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
        if moment.tzinfo is None:
            moment = moment.replace(tzinfo=UTC)
        return moment.astimezone(UTC)
    except (ValueError, OverflowError):
        return None


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


def _finite_duration(value: Any, *, positive: bool = False) -> bool:
    """Duration fields exclude bool and conversions that overflow float."""
    if type(value) not in (int, float):
        return False
    try:
        number = float(value)
    except (OverflowError, TypeError, ValueError):
        return False
    return math.isfinite(number) and (number > 0 if positive else number >= 0)


def _duration(value: Any, label: str) -> Optional[float]:
    if value is None:
        return None  # Missing legacy measurements do not become zero.
    if not _finite_duration(value):
        raise EvalError(f"{label} must be a finite nonnegative number")
    return float(value)


def _refresh_timing(item: Dict[str, Any]) -> Dict[str, Any]:
    refresh = item.get("refresh")
    if refresh is None:
        return {}
    if not isinstance(refresh, dict):
        raise EvalError("refresh timing metadata must be an object")
    return refresh


def _duration_samples(items: Sequence[Dict[str, Any]], field: str) -> List[float]:
    samples = []
    for item in items:
        number = _duration(_refresh_timing(item).get(field), "refresh." + field)
        if number is not None:
            samples.append(number)
    return samples


def _phase_durations(value: Any) -> Dict[str, Any]:
    # Runtime phases are optional diagnostics, not authority for success.
    if not isinstance(value, dict):
        return {}
    return {key: number for key, number in value.items() if _finite_duration(number)}


def write_json(path: Path, document: Any) -> None:
    """Results name client paths and prompt ids: owner-only, whatever the umask."""
    try:
        encoded = (json.dumps(document, indent=1, ensure_ascii=False, allow_nan=False) + "\n").encode("utf-8")
    except (ValueError, TypeError, OverflowError, RecursionError, UnicodeError) as error:
        raise EvalError("result is not serializable as finite UTF-8 JSON") from error
    descriptor = None
    temporary = None
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        descriptor, name = tempfile.mkstemp(prefix="." + path.name + ".tmp-", dir=path.parent)
        temporary = Path(name)
        handle = os.fdopen(descriptor, "wb")
        descriptor = None
        with handle:
            handle.write(encoded)
        os.replace(temporary, path)
        temporary = None  # The renamed pathname is no longer ours to clean up.
    except OSError as error:
        raise EvalError("could not write result JSON") from error
    finally:
        if descriptor is not None:
            try:
                os.close(descriptor)
            except OSError:
                pass
        if temporary is not None:
            try:
                temporary.unlink()
            except FileNotFoundError:
                pass
            except OSError as error:
                raise EvalError("could not clean up result temporary") from error


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



_SNAPSHOT_POLICY = None


def runtime_policy() -> Any:
    """The runtime-under-test's shared source and secret policy module."""
    global _SNAPSHOT_POLICY
    if _SNAPSHOT_POLICY is None:
        path = ROOT / "PHP Core/memory-bank/scripts/automatic_query.py"
        spec = importlib.util.spec_from_file_location("memory_eval_snapshot_policy", path)
        if spec is None or spec.loader is None:
            raise EvalError("Snapshot source policy is unavailable")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        _SNAPSHOT_POLICY = module
    return _SNAPSHOT_POLICY


def snapshot_path_allowed(relative: str) -> bool:
    """Never read environment/credential/dependency files into an evaluation."""
    runtime_policy()
    parts = PurePosixPath(relative).parts
    return not (
        _SNAPSHOT_POLICY.SOURCE_PATH_DENYLIST.search(relative)
        or any(part.casefold() in _SNAPSHOT_POLICY.BLOCKED_SOURCE_PARTS
               or part.casefold().startswith(".env") for part in parts)
        or PurePosixPath(relative).suffix.casefold() in _SNAPSHOT_POLICY.PRIVATE_SOURCE_SUFFIXES
        or (parts and parts[-1].casefold() == "credentials.json")
    )

def materialize(project: Path, tree: str, cache: Path) -> Path:
    """Exact public blobs of `tree`; private paths are withheld before reading."""
    # A new namespace prevents reuse of snapshots predating the source policy.
    target = cache / "public-trees-v1" / tree
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
            if not snapshot_path_allowed(path.as_posix()):
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
            if not snapshot_path_allowed(relative):
                continue
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


def archived_records(root: Path) -> Iterator[str]:
    """Relative paths of archived Brain records (never archived handoffs).

    The runtime never indexes or renders them; they exist so that a chunk
    citing the record it was promoted from validates, as it does in the
    project itself."""
    top = root / ARCHIVE
    if not top.is_dir() or top.is_symlink():
        return
    for directory, folders, files in os.walk(top):
        here = Path(directory)
        folders[:] = sorted(
            name for name in folders
            if not (here / name).is_symlink() and not (here == top and name == "handoffs")
        )
        for name in sorted(files):
            if name.startswith(".") or not name.endswith(".md"):
                continue
            path = here / name
            if path.is_symlink() or not path.is_file():
                continue
            relative = path.relative_to(root).as_posix()
            if snapshot_path_allowed(relative):
                yield relative


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
    counts.update({key: 0 for key in ARCHIVE_COUNTS})
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
        outcome = restore_from_worktree(project, corpus, relative, kind, moment, promotions, updated_after)
        if outcome == "from_worktree+updated_after":
            counts["from_worktree"] += 1
            counts["updated_after"] += 1
        elif outcome is not None:
            counts[outcome] += 1
    # Archived records: the runtime never indexes or renders them, but a
    # promoted chunk cites the record it came from and fails validation when
    # that file is absent. No project commits its archive, so the working
    # tree's copy is taken under the very rule used for active records.
    archived: Set[str] = set()
    for relative in list(archived_records(corpus)):
        archived.add(relative)
        window = creation_window(corpus / relative, "record", promotions)
        if window is not None and window[0] > moment:
            (corpus / relative).unlink()
            counts["archive_dropped_future"] += 1
        else:
            counts["archive_from_git"] += 1
    active = {path.name for path in (corpus / RECORDS).rglob("*.md")} if (corpus / RECORDS).is_dir() else set()
    for relative in list(archived_records(project)):
        if relative in archived:
            continue
        if PurePosixPath(relative).name in active:
            # Still active at the prompt: archived later, and already present.
            counts["archive_still_dynamic"] += 1
            continue
        outcome = restore_from_worktree(project, corpus, relative, "record", moment, promotions, updated_after)
        if outcome == "from_worktree+updated_after":
            counts["archive_from_worktree"] += 1
            counts["archive_updated_after"] += 1
        elif outcome is not None:
            counts["archive_" + outcome] += 1
    return counts


def restore_from_worktree(
    project: Path, corpus: Path, relative: str, kind: str, moment: datetime,
    promotions: Dict[str, Window], updated_after: str,
) -> Optional[str]:
    """Copy one working-tree memory document into the corpus if it existed at
    `moment`; return which count it falls under (None: unreadable)."""
    source = project / relative
    window = creation_window(source, kind, promotions)
    if window is not None and window[0] > moment:
        return "dropped_future"
    try:
        modified = modified_at(source)
    except OSError:
        return None
    # A file unmodified since before the prompt existed then, as it is now.
    if not ((window is not None and window[1] <= moment) or modified <= moment):
        return "undetermined"
    # The document's own edit time decides: a copy tool can preserve
    # or restore an older mtime over a body written after the prompt.
    edited = changed_after(source, kind, moment)
    if edited and updated_after == "drop":
        return "unreconstructable_updated"
    target = corpus / relative
    prepare_parent(corpus, target)
    shutil.copy2(source, target)
    return "from_worktree+updated_after" if edited else "from_worktree"


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
            # The stand measures document retrieval, which an explicit request
            # and the `full` profile still use, whatever profile a project's
            # hooks push. A runtime older than the switch ignores the name.
            "CONTEXT_CAPSULE_PROFILE": "full",
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


def run_runtime(corpus: Path, arguments: Sequence[str], env: Dict[str, str], timeout: float,
                pilot: Optional[Sequence[str]] = None) -> Dict[str, Any]:
    command = [sys.executable, str(corpus / "memory-bank" / "scripts" / "context.py"), *arguments]
    if pilot:
        command = [sys.executable, str(ROOT / "scripts" / "memory_onnx_pilot.py"),
                   "--context", str(corpus / "memory-bank" / "scripts" / "context.py"),
                   *pilot, "--", *arguments]
    started = time.monotonic()
    if pilot:
        # Both pilot preparation and scoring share the parent's remaining
        # budget. Leave bounded cooperative cleanup time before escalation.
        deadline = started + timeout
        grace = min(0.2, timeout * 0.2)
        command[4:4] = ["--invocation-deadline", str(deadline - grace),
                        "--invocation-timeout", str(timeout)]
        process = subprocess.Popen(command, cwd=str(corpus), env=env,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        timed_out = False
        try:
            try:
                stdout, stderr = process.communicate(timeout=max(0.0, deadline - grace - time.monotonic()))
            except subprocess.TimeoutExpired:
                timed_out = True
                process.terminate()
                try:
                    stdout, stderr = process.communicate(timeout=max(0.0, deadline - grace / 4 - time.monotonic()))
                except subprocess.TimeoutExpired:
                    process.kill()
                    try:
                        stdout, stderr = process.communicate(timeout=max(0.0, deadline - time.monotonic()))
                    except subprocess.TimeoutExpired as error:
                        stdout, stderr = error.stdout or b"", error.stderr or b""
            return {"exit": None if timed_out else process.returncode,
                    "seconds": round(time.monotonic() - started, 4),
                    "stdout": stdout, "stderr": stderr}
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=max(0.0, deadline - time.monotonic()))
                except subprocess.TimeoutExpired:
                    process.kill()
                    try:
                        process.wait(timeout=max(0.0, deadline - time.monotonic()))
                    except subprocess.TimeoutExpired:
                        pass
            for stream in (process.stdout, process.stderr):
                if stream is not None:
                    stream.close()
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


def layer_states(result: Optional[Dict[str, Any]]) -> Dict[str, str]:
    """Each memory layer's state as the refresh reported it: one of
    LAYER_STATES, or "other" - never text the runtime may have put there."""
    if not isinstance(result, dict):
        return {}
    return {
        layer: result[layer] if result[layer] in LAYER_STATES else "other"
        for layer in LAYERS
        if layer in result
    }


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
    """{prompt id: {path: grade}}, every grade the integer 0, 1 or 2.

    A prompt may carry its reviewer's note as text under `_note`; it is read
    past, never taken for a path.

    Anything else stops the run before it starts: a grade of 3 or -1 counted
    as useful or as noise, and one that is not a number left its document
    unjudged - either way the measures moved without a word.
    """
    raw = read_json(path)
    if not isinstance(raw, dict):
        raise EvalError(f"{path}: judgments must be a JSON object of {{prompt id: {{path: grade}}}}")
    judgments: Dict[str, Dict[str, int]] = {}
    for prompt_id, grades in raw.items():
        if not isinstance(grades, dict):
            raise EvalError(f"{path}: the judgments of prompt {prompt_id!r} must be an object of {{path: grade}}")
        clean: Dict[str, int] = {}
        for document, grade in grades.items():
            if document == NOTE_KEY and isinstance(grade, str):
                continue
            if isinstance(grade, bool) or not isinstance(grade, int) or grade not in GRADES:
                shown = json.dumps(grade)
                shown = shown if len(shown) <= 40 else shown[:37] + "..."
                raise EvalError(
                    f"{path}: prompt {prompt_id!r}, {document!r}: a grade is the integer 0, 1 or 2, not {shown}"
                )
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
            raise EvalError(f"{path}: prompt {prompt_id!r}: passage documents must be a JSON object")
        clean: Dict[str, Dict[str, Any]] = {}
        for document, entry in documents.items():
            if not isinstance(document, str) or not isinstance(entry, dict):
                raise EvalError(f"{path}: prompt {prompt_id!r}: each passage document must have an object entry")
            values = entry.get("passages", [])
            if not isinstance(values, list) or any(not isinstance(value, str) for value in values):
                raise EvalError(f"{path}: prompt {prompt_id!r}, {document!r}: passages must be a list of strings")
            if "useful" in entry and not isinstance(entry["useful"], bool):
                raise EvalError(f"{path}: prompt {prompt_id!r}, {document!r}: useful must be a JSON Boolean")
            key = normalise_path(document)
            if key in clean and clean[key] != entry:
                raise EvalError(f"{path}: prompt {prompt_id!r}, {key!r}: conflicting passage aliases")
            clean[key] = entry
        passages[str(prompt_id)] = clean
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


def passage_body(value: object) -> str:
    """A passage's non-heading text; headings alone are not an answer body."""
    return "\n".join(
        line for line in str(value or "").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    )


def answer_flatten(value: object) -> str:
    """Flatten an answer passage without Markdown heading lines.

    A labelled changelog passage includes `# Changelog` and its section
    heading; the capsule quotes the entry body. The answer is present even
    when the presentation heading is not, so the heading must not decide
    answer-passage recall.
    """
    body = "\n".join(
        line for line in str(value or "").splitlines()
        if not line.lstrip().startswith("#")
    )
    return flatten(body or value)


def answer_in_text(useful: Sequence[str], passages: Dict[str, Dict[str, Any]], capsule_text: str) -> bool:
    """A labelled passage of a useful delivered document is in the text the model reads."""
    haystack = answer_flatten(capsule_text)
    if not haystack:
        return False
    for path in useful:
        entry = passages.get(path)
        if not isinstance(entry, dict) or entry.get("useful") is False:
            continue
        for passage in entry.get("passages") or []:
            if isinstance(passage, str) and passage_body(passage):
                needle = answer_flatten(passage)
                if needle and needle in haystack:
                    return True
    return False


def labelled_passages(paths: Sequence[str], passages: Dict[str, Dict[str, Any]]) -> Set[str]:
    """Unique normalized answers, not duplicated labels or source pointers."""
    return {
        answer_flatten(value) for path in paths
        if isinstance(passages.get(path), dict) and passages[path].get("useful") is not False
        for value in (passages[path].get("passages")
                      if isinstance(passages[path].get("passages"), list) else [])
        if isinstance(value, str) and passage_body(value) and answer_flatten(value)
    }


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
    *,
    expected_passages: Optional[Set[str]] = None,
    deliverable_passages: Optional[Set[str]] = None,
    secret_patterns: Optional[Mapping[str, Pattern[str]]] = None,
) -> Dict[str, Any]:
    """Score one refresh result. Paths and counts only: no capsule text."""
    result = result if isinstance(result, dict) else {}
    withheld = bool(result.get("query_withheld"))
    delivered = delivered_paths(result.get("capsule"))
    useful = [path for path in delivered if path in grades and grades[path] >= 1]
    noise = [path for path in delivered if path in grades and grades[path] < 1]
    unjudged = [path for path in delivered if path not in grades]
    text = result.get("capsule_text") if isinstance(result.get("capsule_text"), str) else ""
    expected = (expected_passages if expected_passages is not None
                else labelled_passages(answer_existed, passages))
    delivered_answers = labelled_passages(useful, passages) & expected
    # What the runtime could have quoted: in the index's text of the document.
    deliverable = expected if deliverable_passages is None else deliverable_passages & expected
    haystack = answer_flatten(text)
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
        "answer_passages_existed": len(expected),
        "answer_passages_heading_only": sum(
            1 for path in answer_existed
            for value in (passages.get(path, {}).get("passages") or [])
            if isinstance(value, str) and not passage_body(value)
        ),
        "answer_passages_delivered": sum(value in haystack for value in delivered_answers),
        "answer_passages_deliverable": len(deliverable),
        "answer_passages_delivered_deliverable": sum(value in haystack for value in delivered_answers & deliverable),
        "query_withheld": withheld,
        "capsule_chars": len(text),
        **delivery_breakdown(result.get("capsule")),
        "capsule_secret_matches": (
            sum(1 for pattern in secret_patterns.values() if pattern.search(text))
            if secret_patterns is not None else None
        ),
    }


ITEM_KIND = re.compile(r"[a-z][a-z0-9-]{0,31}")
SELECTIONS = ("path-link", "source-link", "prompt-link", "touch-link")


def delivery_breakdown(capsule: object) -> Dict[str, Any]:
    """Which mechanism delivered what: counts and fixed words only.

    Per layer the item kinds (an unrecognised kind is `other`), how many items
    each link mechanism selected, how many carried a changed citation, and how
    many skill sub-files held the procedural slot."""
    kinds: Dict[str, Dict[str, int]] = {layer: {} for layer in LAYERS}
    selections = {name: 0 for name in SELECTIONS}
    changed = 0
    subfiles = 0
    if isinstance(capsule, dict):
        for layer in LAYERS:
            items = capsule.get(layer)
            for item in items if isinstance(items, list) else []:
                if not isinstance(item, dict):
                    continue
                kind = item.get("kind")
                kind = kind if isinstance(kind, str) and ITEM_KIND.fullmatch(kind) else "other"
                kinds[layer][kind] = kinds[layer].get(kind, 0) + 1
                if item.get("selection") in selections:
                    selections[item["selection"]] += 1
                if item.get("source_changed"):
                    changed += 1
                path = item.get("path")
                if (layer == "procedural" and isinstance(path, str) and is_skill(normalise_path(path))
                        and PurePosixPath(path).name != "SKILL.md"):
                    subfiles += 1
    return {
        "layer_kinds": kinds,
        "selections": selections,
        "source_changed_delivered": changed,
        "skill_subfiles_delivered": subfiles,
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


# The runtime marks and quotes a document longer than this only in the stretch
# its query's words point at (MARKED_DOCUMENT_CHARACTERS in context_retrieval).
RUNTIME_MARKED_CHARACTERS = 65536


def indexed_answer_passages(corpus: Path, paths: Sequence[str], expected: Set[str]) -> Tuple[Set[str], Set[str]]:
    """The expected answers the runtime can reach, and those in long documents.

    `existing_answer_passages` asks the file on disk, but the runtime quotes
    the text its index holds for the document: a document the index left out,
    or text it does not store, cannot reach the capsule whatever retrieval
    does, so such an answer is not a retrieval miss. An answer in a document
    longer than the runtime marks whole is still reachable, but only from the
    stretch the query points at; it is counted apart. Read-only.
    """
    database = corpus / "memory-bank/local/context.db"
    if not expected or not database.is_file() or database.is_symlink():
        return set(), set()
    reachable: Set[str] = set()
    long_documents: Set[str] = set()
    connection = None
    try:
        connection = sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True)
        for path in paths:
            names = [path] + ([tool + path[len(CANON_SKILLS):] for tool in TOOL_SKILLS]
                              if path.startswith(CANON_SKILLS) else [])
            for name in names:
                for title, summary, content in connection.execute(
                    "SELECT title, summary, content FROM documents WHERE path = ?", (name,)
                ).fetchall():
                    text = flatten(" ".join(str(part or "") for part in (title, summary, content)))
                    found = {value for value in expected if value in text}
                    reachable |= found
                    if len(str(content or "")) > RUNTIME_MARKED_CHARACTERS:
                        long_documents |= found
    except sqlite3.Error:
        return set(), set()
    finally:
        if connection is not None:
            connection.close()
    return reachable, long_documents


def existing_answer_passages(corpus: Path, paths: Sequence[str], passages: Dict[str, Dict[str, Any]]) -> Set[str]:
    found: Set[str] = set()
    for path in paths:
        target = corpus_file(corpus, path)
        try:
            text = flatten(target.read_text(encoding="utf-8", errors="replace")) if target else ""
        except OSError:
            text = ""
        found.update(value for value in labelled_passages([path], passages) if value in text)
    return found


def source_link_stats(corpus: Path, result: Dict[str, Any]) -> Optional[Dict[str, Any]]:
    """Keep only safe counters before the evaluator removes the local manifest."""
    capsule = result.get("capsule")
    reference = capsule.get("manifest") if isinstance(capsule, dict) else None
    if not isinstance(reference, str) or not re.fullmatch(
        r"(?:memory-bank/local|project-brain/control)/retrieval-manifests/[0-9a-f-]{36}\.json", reference,
    ):
        return None
    target = corpus_file(corpus, reference)
    if target is None or not target.resolve().is_relative_to(corpus.resolve()):
        return None
    try:
        if target.stat().st_size > 512 * 1024:
            return None
        document = read_json(target)
    except (OSError, EvalError):
        return None
    stats = document.get("source_links") if isinstance(document, dict) else None
    if not isinstance(stats, dict):
        return None
    clean = {}
    for key, maximum in (("anchors", 2), ("candidates", 3), ("delivered", 3)):
        value = stats.get(key)
        if type(value) is not int or not 0 <= value <= maximum:
            return None
        clean[key] = value
    return clean


def miss_audit(
    corpus: Path, result: Dict[str, Any], paths: Sequence[str], answer_passages: Dict[str, List[str]],
    env: Dict[str, str], timeout: float, history_text: Optional[str] = None,
    seed_git: bool = False,
) -> Dict[str, Any]:
    """Trace known judged paths in a pinned-clock subprocess, never exposing query/text.

    The helper reads the manifest written by the exact refresh. Its subprocess
    is necessary because importing the runtime here would calculate freshness
    against this evaluator's real clock instead of the clock the refresh used.
    """
    capsule = result.get("capsule") if isinstance(result, dict) else None
    manifest = capsule.get("manifest") if isinstance(capsule, dict) else None
    payload = json.dumps({
        "answer_passages": answer_passages,
        "capsule_text": result.get("capsule_text") if isinstance(result.get("capsule_text"), str) else "",
        "history_text": history_text,
    }).encode("utf-8")
    if seed_git:
        # The archive may carry a `.git` copied from the project. It is either
        # stale or not a valid repository for the synthetic worktree, and it
        # prevented the diagnostic seed from ever running. The corpus belongs
        # to this evaluation run, so strip it before the audit subprocess.
        git_path = corpus / ".git"
        try:
            if git_path.is_dir() and not git_path.is_symlink():
                shutil.rmtree(git_path)
            elif git_path.exists():
                git_path.unlink()
        except OSError:
            pass
    command = [
        sys.executable, str(ROOT / "scripts" / "memory_miss_audit.py"),
        "--corpus", str(corpus), "--manifest", str(manifest or ""),
        "--paths-json", json.dumps(list(paths)),
    ]
    if result.get("query_withheld"):
        command.append("--query-withheld")
    if seed_git:
        command.append("--audit-git")
    try:
        completed = subprocess.run(
            command, cwd=str(corpus), env=env, input=payload, capture_output=True, timeout=timeout,
        )
        decoded = json.loads(completed.stdout.decode("utf-8", "replace")) if completed.returncode == 0 else None
    except (OSError, subprocess.SubprocessError, ValueError, json.JSONDecodeError):
        decoded = None
    expected = set(paths)
    entries = decoded.get("items") if isinstance(decoded, dict) else None
    clean: List[Dict[str, str]] = []
    if isinstance(entries, list):
        for entry in entries:
            if not isinstance(entry, dict):
                continue
            path, stage, answer, counts = (
                entry.get("path"), entry.get("stage"), entry.get("answer"), entry.get("answer_counts")
            )
            valid_counts = (
                isinstance(counts, dict)
                and all(type(counts.get(key)) is int and counts[key] >= 0 for key in ("eligible", "indexed", "delivered"))
            )
            if path in expected and stage in MISS_AUDIT_STAGES and answer in MISS_AUDIT_ANSWERS and valid_counts:
                clean.append({
                    "path": path, "stage": stage, "answer": answer,
                    "answer_counts": counts, "history": entry.get("history") is True,
                })
    found = {entry["path"] for entry in clean}
    clean.extend(
        {"path": path, "stage": "unknown", "answer": "unknown", "answer_counts": {"eligible": 0, "indexed": 0, "delivered": 0}}
        for path in paths if path not in found
    )
    return {"status": "ok" if isinstance(decoded, dict) and decoded.get("status") == "ok" else "unknown", "items": clean}


def audit_answer_passages(grades: Dict[str, int], passages: Dict[str, Dict[str, Any]]) -> Dict[str, List[str]]:
    """Explicit labels used by both score() and the miss audit, for grades 1 and 2."""
    return {
        path: [
            value for value in entry.get("passages") or []
            if isinstance(value, str) and passage_body(value)
        ]
        for path, grade in grades.items()
        if grade >= 1
        for entry in [passages.get(path)]
        if isinstance(entry, dict) and entry.get("useful") is not False
    }


def index_stats(corpus: Path) -> Optional[Dict[str, Any]]:
    """Counters of the corpus's index, read-only: documents, link rows by kind,
    and documents whose stored text still matches a secret pattern."""
    database = corpus / "memory-bank/local/context.db"
    if not database.is_file() or database.is_symlink():
        return None
    connection = None
    try:
        connection = sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True)
        documents = connection.execute("SELECT count(*) FROM documents").fetchone()[0]
        links: Dict[str, int] = {}
        try:
            for kind, number in connection.execute(
                "SELECT ref_kind, count(*) FROM document_links GROUP BY ref_kind"
            ):
                key = kind if isinstance(kind, str) and ITEM_KIND.fullmatch(kind) else "other"
                links[key] = links.get(key, 0) + int(number)
        except sqlite3.OperationalError:
            links = {}
        patterns = list(runtime_policy().SECRET_PATTERNS.values())
        secret = 0
        for title, summary, content in connection.execute("SELECT title, summary, content FROM documents"):
            text = "\n".join(value for value in (title, summary, content) if isinstance(value, str))
            if any(pattern.search(text) for pattern in patterns):
                secret += 1
        return {"documents": int(documents), "links": links, "secret_documents": secret}
    except (sqlite3.Error, OSError, EvalError):
        return None
    finally:
        if connection is not None:
            connection.close()


def contaminated(provenance: Dict[str, Any]) -> bool:
    """Today's body of a document edited after the prompt is in the corpus."""
    return bool(provenance.get("updated_after") or provenance.get("archive_updated_after"))


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
    passage_measured = bool(evaluated) and all(
        type(item.get(key)) is int and 0 <= item[key] <= 2**53-1 for item in evaluated
        for key in ("answer_passages_existed", "answer_passages_delivered")
    ) and all(item['answer_passages_delivered'] <= item['answer_passages_existed'] for item in evaluated)
    passage_total = sum(item["answer_passages_existed"] for item in evaluated) if passage_measured else None
    passage_delivered = sum(item["answer_passages_delivered"] for item in evaluated) if passage_measured else None
    deliverable_measured = bool(evaluated) and all(
        type(item.get(key)) is int for item in evaluated
        for key in ("answer_passages_deliverable", "answer_passages_delivered_deliverable")
    )
    deliverable_total = (sum(item["answer_passages_deliverable"] for item in evaluated)
                         if deliverable_measured else None)
    deliverable_delivered = (sum(item["answer_passages_delivered_deliverable"] for item in evaluated)
                             if deliverable_measured else None)

    def mean(key: str) -> Optional[float]:
        return round(sum(len(item.get(key) or []) for item in evaluated) / count, 3) if count else None

    seconds = _duration_samples(evaluated, "seconds")
    result = {
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
        "answer_passages_existed": passage_total,
        "answer_passages_delivered": passage_delivered,
        "answer_passage_recall": round(passage_delivered / passage_total, 4) if passage_total else None,
        "answer_passages_deliverable": deliverable_total,
        "answer_passages_long_document": (sum(int(item.get("answer_passages_long_document") or 0)
                                              for item in evaluated) if deliverable_measured else None),
        "answer_passages_delivered_deliverable": deliverable_delivered,
        "deliverable_passage_recall": (round(deliverable_delivered / deliverable_total, 4)
                                       if deliverable_total else None),
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
        "answer_passages_heading_only": sum(
            int(item.get("answer_passages_heading_only") or 0)
            for item in evaluated
        ),
        "latency_p50": percentile(seconds, 0.50),
        "latency_p95": percentile(seconds, 0.95),
        **mechanism_totals(evaluated),
        **knowledge_and_skill_totals(evaluated),
    }
    audits = [item.get("miss_audit") for item in evaluated if isinstance(item.get("miss_audit"), dict)]
    if audits:
        entries = [
            entry for audit in audits for entry in audit.get("items") or []
            if isinstance(entry, dict) and isinstance(entry.get("path"), str)
        ]
        stages = Counter(str(entry.get("stage")) for entry in entries)
        answers = Counter(str(entry.get("answer")) for entry in entries)
        answer_counts = {key: sum(
            entry.get("answer_counts", {}).get(key, 0)
            for entry in entries if isinstance(entry.get("answer_counts"), dict)
        ) for key in ("eligible", "indexed", "delivered")}
        unknown = sum(
            stages.get(name, 0)
            for name in ("unknown", "unknown-history", "unknown-query-withheld")
        )
        # A path that never existed at the prompt, or was never indexed, is
        # not evidence about retrieval. The candidate denominator is therefore
        # the labelled paths that the index could have returned.
        eligible = (
            len(entries) - unknown
            - stages.get("absent-source", 0) - stages.get("not-indexed", 0)
        )
        pre_candidate = stages.get("no-fts-candidate", 0) + stages.get("window-limit", 0)
        selected = sum(
            stages.get(name, 0)
            for name in ("selected", "selected-answer-not-in-text", "budget-loss", "capsule-loss")
        )
        history_stages = {"unknown-history", "history-admission-drop", "history-event-limit"}
        history_entries = [
            entry for entry in entries
            if entry.get("history") is True
            or ("history" not in entry and entry.get("stage") in history_stages)
        ]
        history_delivered = sum(
            1 for entry in history_entries
            if entry.get("answer") in {"delivered", "partial-delivery"}
        )
        result["miss_audit"] = {
            "prompts": len(audits),
            "known_paths": len(entries),
            "unknown_paths": unknown,
            "eligible_paths": eligible,
            "candidate_recall": round((eligible - pre_candidate) / eligible, 4) if eligible > 0 else None,
            "selected_recall": round(selected / eligible, 4) if eligible > 0 else None,
            "history_paths": len(history_entries),
            "history_delivered": history_delivered,
            "history_recall": (
                round(history_delivered / len(history_entries), 4)
                if history_entries else None
            ),
            "stages": dict(sorted(stages.items())),
            "answer_states": dict(sorted(answers.items())),
            # Per-path normalized passages intentionally do not replace the
            # scorer's prompt-level de-duplicated answer denominator.
            "per_path_answer_passages": answer_counts,
        }
        for group, skill in (("knowledge", False), ("skills", True)):
            result["miss_audit"][group] = dict(sorted(Counter(
                str(entry.get("stage")) for entry in entries
                if is_skill(normalise_path(entry["path"])) == skill
            ).items()))
    return result


def knowledge_class(item: Dict[str, Any]) -> str:
    """The turn's class with skills left out: what project knowledge did."""
    def keep(key: str) -> List[str]:
        return [path for path in item.get(key) or [] if not is_skill(normalise_path(path))]
    return classify(keep("delivered"), keep("useful_delivered"), keep("noise_delivered"),
                    bool(item.get("query_withheld")))


def skill_outcome(item: Dict[str, Any]) -> str:
    """How the procedural slot did on this turn.

    With a labelled-useful skill: `hit` (a useful one delivered), `wrong`
    (another skill delivered), `empty` (none). Without one: `silent`,
    `noise` (a skill labelled noise delivered) or `unjudged`.
    """
    def skills(key: str) -> List[str]:
        return [path for path in item.get(key) or [] if is_skill(normalise_path(path))]
    delivered = skills("delivered")
    if skills("existed_useful"):
        if skills("useful_delivered"):
            return "hit"
        return "wrong" if delivered else "empty"
    if not delivered:
        return "silent"
    return "noise" if skills("noise_delivered") else "unjudged"


def knowledge_and_skill_totals(evaluated: Sequence[Dict[str, Any]]) -> Dict[str, Any]:
    """Project knowledge and skill routing, apart.

    Agents in real sessions followed none of the 71 skills a capsule named
    and called accelerator skills on 1.1% of Claude turns, while skills were
    53 of 66 missed useful paths on the core set: a turn counted `useful`
    for a skill is useful on the label, not in what the agent did. The
    knowledge measures leave skills out; skill routing is reported on its
    own. Computed from each item's paths, so older results have them too.
    """
    classes = Counter(knowledge_class(item) for item in evaluated)
    outcomes = Counter(skill_outcome(item) for item in evaluated)

    def knowledge(key: str) -> int:
        return sum(
            1 for item in evaluated for path in item.get(key) or []
            if not is_skill(normalise_path(path))
        )
    return {
        "knowledge_useful": classes.get("useful", 0),
        "knowledge_noise_only": classes.get("noise-only", 0),
        "knowledge_unjudged_only": classes.get("unjudged-only", 0),
        "knowledge_silent": classes.get("silent", 0),
        "knowledge_could_help": sum(
            1 for item in evaluated
            if any(not is_skill(normalise_path(path)) for path in item.get("existed_useful") or [])
        ),
        "knowledge_useful_paths_delivered": knowledge("useful_delivered"),
        "knowledge_useful_paths_existed": knowledge("existed_useful"),
        "knowledge_noise_paths": knowledge("noise_delivered"),
        "skill_with_useful": sum(outcomes.get(name, 0) for name in ("hit", "wrong", "empty")),
        "skill_hit": outcomes.get("hit", 0),
        "skill_wrong": outcomes.get("wrong", 0),
        "skill_empty": outcomes.get("empty", 0),
        "skill_without_useful": sum(outcomes.get(name, 0) for name in ("silent", "noise", "unjudged")),
        "skill_silent": outcomes.get("silent", 0),
        "skill_noise": outcomes.get("noise", 0),
        "skill_unjudged": outcomes.get("unjudged", 0),
    }


def mechanism_totals(evaluated: Sequence[Dict[str, Any]]) -> Dict[str, Any]:
    """Path totals and mechanism counters. A counter is None when any item
    lacks it (a result written before it existed): missing never reads as 0."""
    def total(key: str) -> int:
        return sum(len(item.get(key) or []) for item in evaluated)

    def counted(key: str) -> Optional[int]:
        if not evaluated or any(type(item.get(key)) is not int for item in evaluated):
            return None
        return sum(item[key] for item in evaluated)

    breakdown = bool(evaluated) and all(isinstance(item.get("layer_kinds"), dict) for item in evaluated)
    kinds: Optional[Dict[str, Dict[str, int]]] = None
    selections: Optional[Dict[str, int]] = None
    if breakdown:
        kinds = {layer: {} for layer in LAYERS}
        selections = {name: 0 for name in SELECTIONS}
        for item in evaluated:
            for layer, counts in item["layer_kinds"].items():
                for kind, number in (counts or {}).items():
                    kinds.setdefault(layer, {})[kind] = kinds.setdefault(layer, {}).get(kind, 0) + number
            for name, number in (item.get("selections") or {}).items():
                if name in selections:
                    selections[name] += number
    indexes = [_refresh_timing(item).get("index") for item in evaluated]
    indexed = bool(evaluated) and all(isinstance(entry, dict) for entry in indexes)
    link_rows: Optional[Dict[str, int]] = None
    if indexed:
        link_rows = {}
        for entry in indexes:
            for kind, number in (entry.get("links") or {}).items():
                link_rows[kind] = link_rows.get(kind, 0) + number
    index_seconds = _duration_samples(evaluated, "index_seconds")
    return {
        "useful_paths_delivered": total("useful_delivered"),
        "useful_paths_existed": total("existed_useful"),
        "noise_paths": total("noise_delivered"),
        "unjudged_paths": total("unjudged"),
        "layer_kinds": kinds,
        "selections": selections,
        "episodic_brain_events": (
            sum(item["layer_kinds"].get("episodic", {}).get("brain-event", 0) for item in evaluated)
            if breakdown else None
        ),
        "source_changed_delivered": counted("source_changed_delivered"),
        "skill_subfiles_delivered": counted("skill_subfiles_delivered"),
        "capsule_secret_matches": counted("capsule_secret_matches"),
        "index_link_rows": link_rows,
        "index_snapshots_with_links": (
            sum(1 for entry in indexes if any((entry.get("links") or {}).values())) if indexed else None
        ),
        "secret_documents": sum(entry["secret_documents"] for entry in indexes) if indexed else None,
        "capsule_chars_max": max((item.get("capsule_chars") or 0 for item in evaluated), default=None),
        "index_latency_p50": percentile(index_seconds, 0.50),
        "index_latency_p95": percentile(index_seconds, 0.95),
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


def planned_delivery(document: Dict[str, Any]) -> Dict[str, Any]:
    """Supplementary logical-plan delivery, never a successful-pair delta.

    Recorded bindings/counts establish internal consistency only. Missing source
    opportunity or unfinished execution cannot establish a final denominator.
    """
    unknown = {'profile': 'planned-delivery-v1', 'status': 'unknown',
               'unit_delivery_rate': None, 'passage_recall': None}
    plan, items, meta = (document.get(key) for key in ('planned', 'items', 'meta'))
    if not isinstance(plan, dict) or plan.get('profile') != unknown['profile']:
        return unknown
    ids, units = plan.get('ids'), plan.get('units')
    if (not isinstance(ids, list) or any(not isinstance(key, str) or not key.strip() for key in ids)
            or len(set(ids)) != len(ids) or not isinstance(units, dict) or set(units) != set(ids)
            or not isinstance(items, dict) or not set(items) <= set(ids) or not isinstance(meta, dict)
            or type(meta.get('interrupted', False)) is not bool):
        return unknown
    count = lambda value: type(value) is int and 0 <= value <= 2**53-1
    paths = lambda value: (isinstance(value, list)
                           and all(installer.is_safe_relative_path(path) and normalise_path(path) == path for path in value)
                           and len(set(value)) == len(value))
    result = {**unknown, 'status': 'incomplete', 'planned': len(ids), 'terminal': len(items),
              'without_terminal': len(ids)-len(items), 'unattempted': 0, 'runtime_admitted': 0,
              'source_known': 0, 'source_excluded': 0, 'useful_eligible': 0,
              'useful_units_delivered': 0, 'answer_passages_existed': 0, 'answer_passages_delivered': 0}
    bindings: Set[str] = set()
    for key in ids:
        unit, item = units[key], items.get(key)
        if not isinstance(unit, dict):
            return unknown
        state, binding, admitted = unit.get('state'), unit.get('input_sha256'), unit.get('runtime_admitted')
        if (unit.get('id') != key
                or state not in ('unattempted', 'preparing', 'runtime-admitted', 'finished', 'interrupted')
                or not isinstance(binding, str) or not re.fullmatch(r'[0-9a-f]{64}', binding)
                or binding in bindings
                or type(admitted) is not bool or (state == 'finished') != (key in items)
                or state in ('unattempted', 'preparing') and admitted
                or state == 'runtime-admitted' and not admitted):
            return unknown
        bindings.add(binding)
        result['unattempted'] += state == 'unattempted'
        result['runtime_admitted'] += admitted
        if key in items and (not isinstance(item, dict) or item.get('input_sha256') != binding
                                 or item.get('planned_id') != key
                                 or item.get('status') not in ('ok', 'skipped')):
            return unknown
        opportunity = unit.get('source_opportunity')
        if opportunity is None:
            if admitted or item is not None and item.get('status') == 'ok':
                return unknown
            if item is not None and item.get('reason') in ('bad-timestamp', 'no-project', 'no-history'):
                result['source_excluded'] += 1
            continue
        if (not admitted or not isinstance(opportunity, dict)
                or set(opportunity) != {'useful_paths', 'answer_paths', 'answer_passages'}
                or not all(count(value) for value in opportunity.values())
                or opportunity['answer_paths'] > opportunity['useful_paths']
                or opportunity['answer_passages'] and not opportunity['answer_paths']):
            return unknown
        result['source_known'] += 1
        result['useful_eligible'] += opportunity['useful_paths'] > 0
        result['answer_passages_existed'] += opportunity['answer_passages']
        if item is None or item['status'] != 'ok':
            continue
        useful, existed, answered = (item.get(field) for field in ('useful_delivered', 'existed_useful', 'answer_existed'))
        ceiling, delivered = item.get('answer_passages_existed'), item.get('answer_passages_delivered')
        if (not all(paths(value) for value in (useful, existed, answered))
                or len(existed) != opportunity['useful_paths'] or len(answered) != opportunity['answer_paths']
                or not set(useful) <= set(existed) or not set(answered) <= set(existed)
                or not count(ceiling) or ceiling != opportunity['answer_passages']
                or not count(delivered) or delivered > ceiling or delivered and not useful):
            return unknown
        result['useful_units_delivered'] += bool(useful)
        result['answer_passages_delivered'] += delivered
    if any(not count(result[field]) for field in ('answer_passages_existed', 'answer_passages_delivered')):
        return unknown
    if (parse_instant(meta.get('finished')) is not None and not meta.get('interrupted', False)
            and result['terminal'] == result['source_known'] == len(ids)):
        result['status'] = 'complete'
        if result['useful_eligible']:
            result['unit_delivery_rate'] = result['useful_units_delivered']/result['useful_eligible']
        if result['answer_passages_existed']:
            result['passage_recall'] = result['answer_passages_delivered']/result['answer_passages_existed']
    return result


def planned_input_sha256(prompt: Dict[str, Any]) -> str:
    """Bind a logical unit without retaining the prompt text in results."""
    value = {key: prompt[key] for key in ('id', 'project', 'ts', 'prompt')}
    try:
        encoded = json.dumps(value, sort_keys=True, ensure_ascii=False).encode('utf-8')
    except UnicodeError as error:
        raise EvalError('planned inputs must be valid UTF-8') from error
    return hashlib.sha256(encoded).hexdigest()


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
        self.planned: Optional[Dict[str, Any]] = None
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
        if arguments.diagnose_misses:
            self.meta["diagnose_misses"] = True
        if arguments.audit_git:
            self.meta["audit_git"] = True
        if arguments.onnx_excerpts:
            manifest = arguments.onnx_excerpts / "manifest.json"
            self.meta["onnx_excerpts"] = {
                "assets_manifest_sha256": sha256_file(manifest) if manifest.is_file() else None,
                "worker_python": str(arguments.onnx_python),
                "worker_timeout": arguments.onnx_timeout,
                "scope": "selected-document-excerpts-only",
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
        document = {"meta": self.meta, "items": self.items, "summary": summarize(self.items)}
        if self.planned is not None:
            document['planned'] = self.planned
            document['planned_delivery'] = planned_delivery(document)
        return document

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
                item["contaminated"] = contaminated(item["provenance"])
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
            prompt_passages = self.passages.get(prompt["id"], {})
            answered = answer_existing(corpus, grades, prompt_passages)
            expected_passages = existing_answer_passages(corpus, answered, prompt_passages)
            env = runtime_env(self.cache, moment, moment if self.pinned else None)
            objects = repository.history(self.cache, parent) if parent else None
            branch = f"eval/{safe_name(prompt['id'])}"
            commit_corpus(corpus, env, parent if objects else None, objects, branch)
            item["history_linked"] = objects is not None
            # A fresh corpus starts with no index; the hook's index is warm.
            # Building it first keeps the timed refresh the per-turn cost.
            if self.planned is not None:
                unit = self.planned['units'][prompt['id']]
                unit.update(state='runtime-admitted', runtime_admitted=True, source_opportunity={
                    'useful_paths': len(existed), 'answer_paths': len(answered),
                    'answer_passages': len(expected_passages)})
                # A failed checkpoint prevents runtime admission. This write
                # remains outside scoring; the prior atomic document survives.
                write_json(self.arguments.out, self.document())
            warm = run_runtime(corpus, ["index"], env, self.arguments.timeout)
            pilot = None
            pilot_stats = corpus / ".memory-onnx-stats.json"
            if self.arguments.onnx_excerpts:
                pilot = ["--assets", str(self.arguments.onnx_excerpts.resolve()),
                         # Resolving a venv's python symlink selects the base
                         # interpreter and loses its optional dependencies.
                         "--python", str(self.arguments.onnx_python.expanduser().absolute()),
                         "--worker-timeout", str(self.arguments.onnx_timeout),
                         "--stats", str(pilot_stats)]
            refresh = run_runtime(
                corpus,
                [
                    "refresh", "--host", self.arguments.host, "--query", prompt["prompt"],
                    "--task-id", branch, "--limit", str(REFRESH_RETRIEVAL_LIMIT),
                    "--ephemeral", "--sanitize", "--json",
                ],
                env,
                self.arguments.timeout,
                **({"pilot": pilot} if pilot else {}),
            )
            result = refresh_json(refresh["stdout"])
            item["refresh"] = {
                "exit": refresh["exit"],
                "seconds": _duration(refresh["seconds"], "refresh.seconds"),
                "index_seconds": _duration(warm["seconds"], "refresh.index_seconds"),
                "index_exit": warm["exit"],
                "phases": _phase_durations((result or {}).get("phases")),
                "warnings": len((result or {}).get("warnings") or []),
                "layers": layer_states(result),
                "capsule_missing": bool(result) and result.get("capsule") is None and not result.get("query_withheld"),
                "stderr_tail": stderr_tail(refresh["stderr"], prompt["prompt"]),
            }
            try:
                diagnosis = normalize_funnel_diagnosis((result or {}).get("funnel_diagnosis"))
            except Exception:
                diagnosis = None
            if diagnosis is not None:
                item["refresh"]["funnel_diagnosis"] = diagnosis
            try:
                receipt = normalize_ordinary_receipt((result or {}).get("ordinary_receipt"))
            except Exception:
                receipt = None
            if receipt is not None:
                item["refresh"]["ordinary_receipt"] = receipt
            if pilot:
                try:
                    counters = json.loads(pilot_stats.read_text(encoding="utf-8"))
                    item["refresh"]["onnx_excerpts"] = {
                        key: value for key, value in counters.items()
                        if re.fullmatch(r"[a-z-]+(?::[a-z-]+)?", key)
                        and type(value) is int and 0 <= value <= 10_000
                    }
                except (OSError, ValueError, AttributeError):
                    item["refresh"]["onnx_excerpts"] = {"stats-unavailable": 1}
            # Only a refresh that exited 0 is scored. The runtime prints its
            # JSON and exits 1 when a memory layer failed to update: scored,
            # that turn read as a silent one, or one served from a stale
            # index, and a broken runtime as a quiet one.
            if refresh["exit"] is None:
                raise Skip("refresh-timeout")
            if refresh["exit"] != 0 or result is None:
                raise Skip("refresh-error")
            graph_stats = source_link_stats(corpus, result)
            if graph_stats is not None:
                item["refresh"]["source_links"] = graph_stats
            stats = index_stats(corpus)
            if stats is not None:
                item["refresh"]["index"] = stats
            reachable, long_documents = indexed_answer_passages(corpus, answered, expected_passages)
            item.update(score(
                result, grades, prompt_passages, existed, answered,
                expected_passages=expected_passages, deliverable_passages=reachable,
                secret_patterns=runtime_policy().SECRET_PATTERNS,
            ))
            item["answer_passages_long_document"] = len(long_documents)
            if self.arguments.diagnose_misses:
                known_paths = sorted(path for path, grade in grades.items() if grade >= 1)
                answer_passages = audit_answer_passages(grades, prompt_passages)
                item["miss_audit"] = miss_audit(
                    corpus, result, known_paths, answer_passages, env, self.arguments.timeout,
                    history_text=prompt["prompt"],
                    seed_git=self.arguments.audit_git,
                )
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
        active: Optional[str] = None
        try:
            self.planned = {'profile': 'planned-delivery-v1', 'ids': [prompt['id'] for prompt in selected],
                            'units': {prompt['id']: {'id': prompt['id'], 'input_sha256': planned_input_sha256(prompt),
                                                    'state': 'unattempted', 'runtime_admitted': False}
                                      for prompt in selected}}
            write_json(out, self.document())
            for index, prompt in enumerate(selected, 1):
                active = prompt['id']
                unit = self.planned['units'][active]
                unit['state'] = 'preparing'
                write_json(out, self.document())
                item = self.evaluate(prompt)
                item['input_sha256'] = unit['input_sha256']
                item['planned_id'] = active
                self.items[active] = item
                unit['state'] = 'finished'
                write_json(out, self.document())
                active = None
                if item["status"] == "ok":
                    note = f"{item['class']}  delivered={len(item['delivered'])}  {item['refresh']['seconds']:.2f}s"
                else:
                    note = f"skipped: {item.get('reason')}"
                print(f"[{index}/{len(selected)}] {prompt['id']}  {note}", file=sys.stderr)
        except KeyboardInterrupt:
            self.meta["interrupted"] = True
            if active is not None and active not in self.items:
                self.planned['units'][active]['state'] = 'interrupted'
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
    if not _finite_duration(arguments.timeout, positive=True):
        raise EvalError("--timeout must be finite and positive")
    if bool(arguments.onnx_excerpts) != bool(arguments.onnx_python):
        raise EvalError("--onnx-excerpts and --onnx-python must be supplied together")
    if not _finite_duration(arguments.onnx_timeout, positive=True):
        raise EvalError("--onnx-timeout must be finite and positive")
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
    for item in document["items"].values():
        if not isinstance(item, dict):
            raise EvalError("result items must be objects")
        refresh = _refresh_timing(item)
        for field in ("seconds", "index_seconds"):
            _duration(refresh.get(field), "refresh." + field)
        phases = refresh.get("phases")
        if phases is not None:
            if not isinstance(phases, dict):
                raise EvalError("refresh.phases must be an object")
            for value in phases.values():
                _duration(value, "refresh.phases")
    meta = document.get("meta")
    if isinstance(meta, dict):
        budgets = [("meta.timeout", meta.get("timeout"))]
        onnx = meta.get("onnx_excerpts")
        if isinstance(onnx, dict):
            budgets.append(("meta.onnx_excerpts.worker_timeout", onnx.get("worker_timeout")))
        for label, value in budgets:
            if value is not None and not _finite_duration(value, positive=True):
                raise EvalError(f"{label} must be finite and positive")
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
    ("answer passages existed", "answer_passages_existed"),
    ("answer passages heading-only", "answer_passages_heading_only"),
    ("answer passages delivered", "answer_passages_delivered"),
    ("answer passage recall", "answer_passage_recall"),
    ("answer passages deliverable (in the index)", "answer_passages_deliverable"),
    ("  of them in documents over 64 KB", "answer_passages_long_document"),
    ("deliverable passages delivered", "answer_passages_delivered_deliverable"),
    ("deliverable passage recall", "deliverable_passage_recall"),
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
    ("knowledge: useful turns", "knowledge_useful"),
    ("knowledge: could help", "knowledge_could_help"),
    ("knowledge: noise-only turns", "knowledge_noise_only"),
    ("knowledge: unjudged-only turns", "knowledge_unjudged_only"),
    ("knowledge: silent turns", "knowledge_silent"),
    ("knowledge: useful paths delivered", "knowledge_useful_paths_delivered"),
    ("knowledge: noise paths", "knowledge_noise_paths"),
    ("skills: a useful one delivered", "skill_hit"),
    ("skills: another one delivered", "skill_wrong"),
    ("skills: none delivered", "skill_empty"),
    ("skills: noise where none was useful", "skill_noise"),
    ("skills: silent where none was useful", "skill_silent"),
    ("useful paths delivered", "useful_paths_delivered"),
    ("noise paths", "noise_paths"),
    ("unjudged paths", "unjudged_paths"),
    ("skill sub-files in procedural", "skill_subfiles_delivered"),
    ("Brain events in episodic", "episodic_brain_events"),
    ("changed-citation items delivered", "source_changed_delivered"),
    ("snapshots with link rows", "index_snapshots_with_links"),
    ("indexed documents matching a secret", "secret_documents"),
    ("capsules matching a secret", "capsule_secret_matches"),
    ("max capsule chars", "capsule_chars_max"),
    ("index latency p50 (s)", "index_latency_p50"),
    ("index latency p95 (s)", "index_latency_p95"),
)
SHARED_ROWS = {"useful", "answer_in_text", "noise_only", "unjudged_only", "silent", "could_help",
               "answer_could_help", "contaminated", "knowledge_useful", "knowledge_could_help",
               "knowledge_noise_only", "knowledge_unjudged_only", "knowledge_silent"}
# Rows about a run's coverage; every other row measures its evaluated prompts.
COVERAGE_ROWS = {"prompts", "evaluated", "skipped_total"}


def row_value(summary: Dict[str, Any], key: str) -> Optional[float]:
    if key == "skipped_total":
        return sum(summary.get("skipped", {}).values())
    value: Any = summary
    for part in key.split("/"):
        value = value.get(part) if isinstance(value, dict) else None
    if type(value) is int or (type(value) is float and math.isfinite(value)):
        return value
    return None


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
    if key in (
        "answer_passage_recall", "deliverable_passage_recall",
        "miss_audit/candidate_recall", "miss_audit/selected_recall",
        "miss_audit/history_recall",
    ):
        return f"{value:.1%}"
    if key == "useful_paths_delivered":
        return f"{int(value)}/{int(summary.get('useful_paths_existed') or 0)}"
    if key == "knowledge_useful_paths_delivered":
        return f"{int(value)}/{int(summary.get('knowledge_useful_paths_existed') or 0)}"
    if key in ("skill_hit", "skill_wrong", "skill_empty"):
        return f"{int(value)}/{int(summary.get('skill_with_useful') or 0)}"
    if key in ("skill_noise", "skill_silent"):
        return f"{int(value)}/{int(summary.get('skill_without_useful') or 0)}"
    if key.startswith(("latency", "index_latency")):
        return f"{value:.3f}"
    if key.startswith("mean"):
        return f"{value:.2f}"
    return str(int(value))


def describe_meta(meta: Dict[str, Any], items: Dict[str, Dict[str, Any]]) -> str:
    meta = meta if isinstance(meta, dict) else {}
    edition = meta.get('edition')
    edition = edition if isinstance(edition, str) and edition in (*installer.EDITIONS, 'auto') else 'unknown'
    if edition == "auto":
        used = Counter(item['edition'] for item in items.values()
                       if isinstance(item.get('edition'), str) and item['edition'] in installer.EDITIONS)
        edition += " (" + ", ".join(f"{name} {count}" for name, count in sorted(used.items())) + ")"
    commit = meta.get('clone_commit')
    commit = commit[:10] if isinstance(commit, str) and re.fullmatch(r'(?:[0-9a-f]{40}|[0-9a-f]{64})', commit) else '?'
    editions = meta.get('editions')
    dirty = isinstance(editions, dict) and any(isinstance(info, dict) and info.get('dirty') is True
                                              for info in editions.values())
    as_of = meta.get('as_of') if isinstance(meta.get('as_of'), str) and meta['as_of'] in ('prompt', 'now') else 'unknown'
    clock = meta.get('clock') if isinstance(meta.get('clock'), str) and meta['clock'] in ('pinned', 'real') else 'unknown'
    host = meta.get('host') if isinstance(meta.get('host'), str) and meta['host'] in HOSTS else 'unknown'
    return (
        f"edition={edition} as_of={as_of} clock={clock} "
        f"host={host} clone={commit}{' (edition dirty)' if dirty else ''}"
    )


def evaluated_ids(items: Dict[str, Dict[str, Any]]) -> Set[str]:
    return {prompt_id for prompt_id, item in items.items() if item.get("status") == "ok"}


def comparison_compatibility(first: Dict[str, Any], second: Dict[str, Any]) -> Dict[str, Any]:
    """Compare recorded conditions, never infer historical corpus-body identity.

    Runtime version/assets are the treatment. Missing evidence is unknown;
    a known condition mismatch still dominates an unrelated missing field.
    Reasons name fixed fields only, never supplied metadata values or ids.
    """
    missing = object()
    mismatch: Set[str] = set()
    unknown: Set[str] = set()
    meta = [document.get('meta') if isinstance(document.get('meta'), dict) else {}
            for document in (first, second)]

    def compare(field, left, right, valid):
        if valid(left) and valid(right):
            if left != right:
                mismatch.add(field)
        else:
            unknown.add(field)

    digest = lambda value: isinstance(value, str) and re.fullmatch(r'[0-9a-f]{64}', value) is not None
    choice = lambda value, allowed: isinstance(value, str) and value in allowed
    for key in ('set_sha256', 'judgments_sha256'):
        compare('meta.'+key, *(item.get(key, missing) for item in meta), digest)
    compare('meta.passages_sha256', *(item.get('passages_sha256', missing) for item in meta),
            lambda value: value is None or digest(value))
    for key, allowed in (('host', HOSTS), ('clock', ('pinned', 'real')), ('as_of', ('prompt', 'now')),
                         ('updated_after', ('drop', 'keep'))):
        compare('meta.'+key, *(item.get(key, missing) for item in meta), lambda value: choice(value, allowed))
    compare('meta.timeout', *(item.get('timeout', missing) for item in meta),
            lambda value: _finite_duration(value, positive=True))
    if any('onnx_excerpts' in item for item in meta):
        profiles = [item.get('onnx_excerpts') for item in meta]
        compare('meta.onnx_excerpts.worker_timeout',
                *(profile.get('worker_timeout', missing) if isinstance(profile, dict) else missing
                  for profile in profiles), lambda value: _finite_duration(value, positive=True))
    runtimes = [item.get('runtime_under_test') for item in meta]
    if any('runtime_under_test' in item for item in meta):
        compare('meta.runtime_under_test.scorer_sha256',
                *(profile.get('scorer_sha256', missing) if isinstance(profile, dict) else missing
                  for profile in runtimes), digest)

    runs = [document['items'] for document in (first, second)]
    both = evaluated_ids(runs[0]) & evaluated_ids(runs[1])
    if not both:
        unknown.add('items.shared_sources')
    for identifier in both:
        units = [items[identifier] for items in runs]
        compare('items.project', *(unit.get('project', missing) for unit in units),
                lambda value: isinstance(value, str) and bool(value.strip()))
        compare('items.ts', *(parse_instant(unit.get('ts')) for unit in units), lambda value: value is not None)
        compare('items.commit', *(unit.get('commit', missing) for unit in units),
                lambda value: isinstance(value, str) and re.fullmatch(r'(?:[0-9a-f]{40}|[0-9a-f]{64})', value) is not None)
        compare('items.edition', *(unit.get('edition', missing) for unit in units),
                lambda value: choice(value, installer.EDITIONS))
        for key in ('history_linked', 'contaminated'):
            compare('items.'+key, *(unit.get(key, missing) for unit in units), lambda value: type(value) is bool)
        if any(unit.get('contaminated') is True for unit in units):
            unknown.add('items.contaminated')
        provenance = [unit.get('provenance') for unit in units]
        counter = lambda value: type(value) is int and value >= 0
        for profile, settings in zip(provenance, meta):
            required = ({'from_git', 'from_worktree', 'dropped_future', 'undetermined', 'updated_after',
                         'unreconstructable_updated', *ARCHIVE_COUNTS} if settings.get('as_of') == 'prompt'
                        else {'from_git', 'from_worktree', 'dropped_future', 'future_present', 'undetermined'})
            if (not isinstance(profile, dict) or not required <= profile.keys()
                    or not profile.keys() <= required | {'config_from_worktree'}
                    or any(not counter(value) for value in profile.values())):
                unknown.add('items.provenance')
        if all(isinstance(profile, dict) for profile in provenance):
            if provenance[0].keys() != provenance[1].keys():
                unknown.add('items.provenance')
            for key in provenance[0].keys() & provenance[1].keys():
                if counter(provenance[0][key]) and counter(provenance[1][key]) and provenance[0][key] != provenance[1][key]:
                    mismatch.add('items.provenance')
    return {'profile': 'recorded-comparison-v1',
            'status': 'incompatible' if mismatch else 'unknown' if unknown else 'compatible',
            'mismatched_fields': sorted(mismatch), 'unknown_fields': sorted(unknown)}


def format_report(results: Sequence[Tuple[str, Path, Dict[str, Any]]]) -> str:
    summaries = [summarize(document["items"]) for _, _, document in results]
    lines = []
    for (label, path, document), _summary in zip(results, summaries):
        lines.append(f"{label}: {path}  {describe_meta(document.get('meta') or {}, document['items'])}")
        whole = planned_delivery(document)
        line = f"whole-planned {label}: {whole['status']}"
        if whole['status'] != 'unknown':
            percent = lambda value: '-' if value is None else f'{value*100:.1f}%'
            line += (f"; terminal={whole['terminal']}/{whole['planned']}"
                     f"; source-known={whole['source_known']}/{whole['planned']}"
                     f"; source-excluded={whole['source_excluded']}"
                     f"; unattempted={whole['unattempted']}; without-terminal={whole['without_terminal']}"
                     f"; unit delivery={percent(whole['unit_delivery_rate'])}"
                     f"; passage recall={percent(whole['passage_recall'])}")
        lines.append(line+'; supplementary per-run delivery only')
    paired: List[Dict[str, Any]] = []
    delta_title = "delta"
    if len(results) == 2:
        comparison = comparison_compatibility(results[0][2], results[1][2])
        status = comparison['status']
        lines.append('comparison: '+status+' recorded metadata'
                     + ('; quality/latency deltas withheld' if status == 'incompatible'
                        else '; ID-overlap diagnostics only' if status == 'unknown'
                        else '; not a corpus-body identity proof'))
        for name in ('mismatched_fields', 'unknown_fields'):
            if comparison[name]:
                lines.append(name.replace('_', ' ')+': '+', '.join(comparison[name]))
        # A measure's delta is over the prompts both runs evaluated: a prompt
        # one of them skipped would otherwise read as a loss or a gain.
        runs = [document["items"] for _, _, document in results]
        both = evaluated_ids(runs[0]) & evaluated_ids(runs[1])
        if both and status != 'incompatible':
            paired = [summarize({prompt_id: items[prompt_id] for prompt_id in both}) for items in runs]
        only = [len(evaluated_ids(items) - both) for items in runs]
        if any(only):
            delta_title = "paired delta"
            lines.append(
                f"note: {len(both)} evaluated in both, {only[0]} only in A, {only[1]} only in B: each measure's "
                "delta is over the prompts evaluated in both; prompts, evaluated and skipped compare the whole runs"
            )
        if status == 'unknown':
            delta_title = 'diagnostic paired' if any(only) else 'diagnostic delta'
            if any(only):
                lines.append('note: diagnostic paired delta does not establish compatible comparison conditions')
        elif status == 'incompatible':
            delta_title = 'no paired delta'
    for (label, _path, _document), summary in zip(results, summaries):
        audit = summary.get("miss_audit") or {}
        candidate = audit.get("candidate_recall")
        selected = audit.get("selected_recall")
        if not (isinstance(candidate, (int, float)) and isinstance(selected, (int, float))):
            continue
        candidate_loss = max(0.0, 1.0 - float(candidate))
        selection_loss = max(0.0, float(candidate) - float(selected))
        if candidate_loss == 0.0 and selection_loss == 0.0:
            bound = "none measured"
        elif candidate_loss > selection_loss:
            bound = "candidate-bound"
        elif selection_loss > candidate_loss:
            bound = "selection-bound"
        else:
            bound = "mixed"
        lines.append(
            f"{label}: retrieval loss candidate {candidate_loss * 100:.1f}pp, "
            f"selection {selection_loss * 100:.1f}pp ({bound})"
        )
    rows = list(ROWS)
    if any("miss_audit" in summary for summary in summaries):
        rows += [("audit: prompts", "miss_audit/prompts"),
                 ("audit: unknown judged paths", "miss_audit/unknown_paths"),
                 ("audit: candidate recall", "miss_audit/candidate_recall"),
                 ("audit: selected recall", "miss_audit/selected_recall"),
                 ("audit: history paths", "miss_audit/history_paths"),
                 ("audit: history delivered", "miss_audit/history_delivered"),
                 ("audit: history recall", "miss_audit/history_recall")]
        for group in ("knowledge", "skills"):
            stages = sorted(set().union(*(set((summary.get("miss_audit") or {}).get(group, {}))
                                          for summary in summaries)))
            rows += [(f"audit {group}: {stage}", f"miss_audit/{group}/{stage}") for stage in stages]
    width = max(len(title) for title, _ in rows) + 2
    cell = 18
    header = " " * width + "".join(label.rjust(cell) for label, _, _ in results)
    if len(results) == 2:
        header += delta_title.rjust(cell)
    lines += ["", header]
    for title, key in rows:
        row = title.ljust(width) + "".join(row_cell(summary, key).rjust(cell) for summary in summaries)
        if len(results) == 2:
            values = [row_value(summary, key) for summary in (summaries if key in COVERAGE_ROWS else paired)]
            if len(values) == 2 and values[0] is not None and values[1] is not None:
                change = values[1] - values[0]
                delta = (f"{change * 100:+.1f}pp" if key in (
                    "answer_passage_recall", "deliverable_passage_recall",
                    "miss_audit/candidate_recall", "miss_audit/selected_recall",
                    "miss_audit/history_recall",
                )
                         else f"{change:+.3f}" if key.startswith(("mean", "latency", "index_latency"))
                         else f"{int(change):+d}")
                row += delta.rjust(cell)
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
    """ID-only successful-intersection diagnostic; caller checks compatibility."""
    ids = [
        prompt_id
        for prompt_id, item in first.items()
        if item.get("status") == "ok" and (second.get(prompt_id) or {}).get("status") == "ok"
    ]
    tests: Dict[str, Callable[[Dict[str, Any]], bool]] = {
        "useful": lambda item: item.get("class") == "useful",
        "noise-only": lambda item: item.get("class") == "noise-only",
        "answer": lambda item: bool(item.get("answer_in_text")),
        "knowledge useful": lambda item: knowledge_class(item) == "useful",
        "knowledge noise-only": lambda item: knowledge_class(item) == "noise-only",
        "skill hit": lambda item: skill_outcome(item) == "hit",
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
        status = comparison_compatibility(first, second)['status']
        count, changes = paired_diff(first["items"], second["items"])
        print("")
        if status == 'compatible':
            print(f"paired over {count} prompt(s) evaluated in both (B against A)")
        elif status == 'unknown':
            print(f"diagnostic ID-overlap over {count} prompt(s) evaluated in both (B against A; compatibility unknown)")
        else:
            print(f"ID overlap: {count} prompt(s) evaluated in both; paired gains withheld")
        for label, items, other in (("A", first["items"], second["items"]), ("B", second["items"], first["items"])):
            only = sorted(evaluated_ids(items) - evaluated_ids(other))
            if only:
                print(f"  evaluated only in {label}: {', '.join(only)}")
        for name, (gained, lost) in (changes.items() if status != 'incompatible' else ()):
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


NATIVE_JOURNAL = "memory-bank/local/native-results.ndjson"
HEALTH_SERIES = "memory-bank/local/refresh-health.ndjson"
NATIVE_HOSTS = ("claude", "codex", "cursor")
# A journal code is a fixed word; anything else in the field is counted as
# "other", never printed.
NATIVE_CODE = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.:-]{0,79}")
NATIVE_COUNTERS = (
    "prompt_hook_runs", "armed", "armed_after_incomplete", "continuations",
    "stop_hook_runs", "stop_hook_timeouts", "stop_hook_failures", "requested",
    "saved", "held", "omitted", "failed", "refused", "errors",
    "learnings_written", "chunks_promoted",
)


def native_step(counts: Counter, reasons: Dict[str, Counter], record: Dict[str, Any]) -> None:
    """Count one journal line: a step of native_results, or the Stop hook's exit."""
    stage, event = record.get("stage"), record.get("event")
    if not isinstance(stage, str) or not isinstance(event, str):
        return
    code = record.get("code")
    if stage == "stop-hook" and event == "exited":
        status = record.get("status")
        code = str(status) if type(status) is int else None
    elif code is not None:
        code = code if isinstance(code, str) and NATIVE_CODE.fullmatch(code) else "other"
    if stage == "prompt":
        counts["prompt_hook_runs"] += 1
        counts["armed"] += event == "armed"
        counts["armed_after_incomplete"] += event == "armed" and code is not None
        counts["continuations"] += event == "continuation"
    elif stage == "stop-hook":
        counts["stop_hook_runs"] += 1
        counts["stop_hook_timeouts"] += code == "124"
        counts["stop_hook_failures"] += event == "exited" and code not in (None, "0", "124")
    elif stage == "stop":
        counts["requested"] += event == "requested"
        counts["failed"] += event == "failed"
        counts["held"] += event == "held"
    elif stage == "save":
        # A repeat of a finished save is that save again, not another one.
        if event in ("saved", "held", "omitted") and code != "repeat":
            counts[event] += 1
        if event in ("saved", "held"):
            for field, counter in (("learnings", "learnings_written"), ("promoted", "chunks_promoted")):
                value = record.get(field)
                counts[counter] += value if type(value) is int and value > 0 else 0
        counts["refused"] += event == "refused"
    counts["errors"] += event == "error"
    if code is not None:
        reasons.setdefault(stage + " " + event, Counter())[code] += 1


def native_summary(counts: Counter, reasons: Dict[str, Counter]) -> Dict[str, Any]:
    return {**{name: counts.get(name, 0) for name in NATIVE_COUNTERS},
            "reasons": {step: dict(sorted(reasons[step].items())) for step in sorted(reasons)}}


def native_funnel(state: Path, since: Optional[datetime]) -> Dict[str, Any]:
    """How far the automatic native saves got, per host, from disk.

    Reads the journal native_results.py and the Stop hook append - one
    metadata line per prompt, Stop, save and Stop-hook exit - and, for the
    prompt hook runs of every host together, the hook's lines in the
    refresh-health series, which names no host. Counts and fixed codes only.
    """
    hosts: Dict[str, Tuple[Counter, Dict[str, Counter]]] = {
        host: (Counter(), {}) for host in (*NATIVE_HOSTS, "unknown")}

    def within(record: Dict[str, Any]) -> bool:
        at = parse_instant(record.get("at"))
        return since is None or (at is not None and at >= since)

    lines = 0
    for record in read_records(state / NATIVE_JOURNAL):
        if within(record):
            lines += 1
            host = record.get("host") if record.get("host") in NATIVE_HOSTS else "unknown"
            native_step(*hosts[host], record)
    total: Counter = Counter()
    reasons: Dict[str, Counter] = {}
    for counts, steps in hosts.values():
        total.update(counts)
        for step, codes in steps.items():
            reasons.setdefault(step, Counter()).update(codes)
    return {
        "journal": (state / NATIVE_JOURNAL).is_file(),
        "lines": lines,
        "prompt_hook_health_lines": sum(record.get("source") == "hook" and within(record)
                                        for record in read_records(state / HEALTH_SERIES)),
        "hosts": {host: native_summary(*hosts[host]) for host in hosts},
        "total": native_summary(total, reasons),
    }


# The Stop hook's save request opens with this marker and its challenge's token.
NATIVE_CONTINUATION = re.compile(r"INTERNAL_NATIVE_MEMORY_RESULT [0-9a-f]{32}")


def continuation_costs(path: Path, host: str) -> List[Dict[str, Any]]:
    """What each native save continuation in one transcript cost.

    In governed mode the Stop hook asks the agent, after every answer, to save
    its result: one more model call. The journal counts the requests; only
    the transcript says what they cost. For each request this reads the time
    and the output tokens from it to the next human prompt (the save, and the
    answer the agent repeats), beside the same for the answer before it, from
    its prompt to the request. Claude Code's output tokens are each response's
    `usage` (one response spans several records); Codex's, the cumulative
    `token_count`. A request is found by its marker wherever the host records
    it, never in the agent's own messages.
    """
    costs: List[Dict[str, Any]] = []
    prompt: Optional[Tuple[datetime, int]] = None
    pending: Optional[Dict[str, Any]] = None
    last: Optional[datetime] = None
    output = 0
    seen: set = set()

    def close() -> None:
        if pending is not None and last is not None:
            pending["save_seconds"] = max(0.0, (last - pending["at"]).total_seconds())
            pending["save_output_tokens"] = output - pending["output_at"]
            costs.append(pending)

    for record in read_records(path):
        at = parse_instant(record.get("timestamp"))
        payload = record.get("payload") if isinstance(record.get("payload"), dict) else {}
        if host == "codex":
            if payload.get("type") == "token_count" and isinstance(payload.get("info"), dict):
                total = payload["info"].get("total_token_usage")
                if isinstance(total, dict) and isinstance(total.get("output_tokens"), int):
                    output = total["output_tokens"]
            role = payload.get("role") if payload.get("type") == "message" else None
            text = json.dumps(payload.get("content")) if role else ""
            human = role == "user" and not NATIVE_CONTINUATION.search(text) and not \
                "".join(part.get("text", "") for part in payload.get("content") or []
                        if isinstance(part, dict)).lstrip().startswith("<")
            request = role == "user" and bool(NATIVE_CONTINUATION.search(text))
        else:
            message = record.get("message") if isinstance(record.get("message"), dict) else {}
            if record.get("type") == "assistant":
                usage = message.get("usage") if isinstance(message.get("usage"), dict) else {}
                identifier = message.get("id")
                if identifier not in seen and isinstance(usage.get("output_tokens"), int):
                    seen.add(identifier)
                    output += usage["output_tokens"]
            human = (record.get("type") == "user" and not record.get("toolUseResult")
                     and not record.get("isMeta") and isinstance(message.get("content"), str)
                     and not NATIVE_CONTINUATION.search(message["content"]))
            request = record.get("type") != "assistant" and bool(NATIVE_CONTINUATION.search(json.dumps(record)))
        if at is None:
            continue
        if human:
            close()
            pending, prompt = None, (at, output)
        elif request and pending is None and prompt is not None:
            pending = {"at": at, "output_at": output,
                       "answer_seconds": max(0.0, (at - prompt[0]).total_seconds()),
                       "answer_output_tokens": output - prompt[1]}
        last = at
    close()
    return [{key: value for key, value in cost.items() if key not in ("at", "output_at")} for cost in costs]


def continuation_summary(costs: List[Dict[str, Any]]) -> Dict[str, Any]:
    if not costs:
        return {"requests": 0}

    def median(key: str) -> float:
        return round(statistics.median(cost[key] for cost in costs), 1)

    shares = [cost["save_seconds"] / cost["answer_seconds"] for cost in costs if cost["answer_seconds"] > 0]
    return {"requests": len(costs), "save_seconds": median("save_seconds"),
            "save_output_tokens": median("save_output_tokens"),
            "answer_seconds": median("answer_seconds"), "answer_output_tokens": median("answer_output_tokens"),
            "time_share": round(statistics.median(shares), 2) if shares else None}


def command_realized(arguments: argparse.Namespace) -> int:
    project = Path(arguments.project).expanduser().resolve()
    if not project.is_dir():
        raise EvalError(f"--project {arguments.project} is not a directory")
    state = Path(arguments.state).expanduser().resolve() if arguments.state else project
    if not state.is_dir():
        raise EvalError(f"--state {arguments.state} is not a directory")
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
        "native": native_funnel(state, since),
        "continuations": {
            "claude": continuation_summary([cost for path in claude_used for cost in continuation_costs(path, "claude")]),
            "codex": continuation_summary([cost for path in codex_used for cost in continuation_costs(path, "codex")]),
        },
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
    lines.extend(format_native(report["native"]))
    lines.extend(format_continuations(report["continuations"]))
    if "paths" in report:
        lines.append("")
        lines.append("by path: delivered / touched / mentioned")
        for path, counts in report["paths"].items():
            lines.append(
                f"  {counts.get('delivered', 0):>5} {counts.get('touched', 0):>5}"
                f" {counts.get('mentioned', 0):>5}  {path}"
            )
    return "\n".join(lines)


NATIVE_ROWS = (
    ("prompt hook runs", "prompt_hook_runs"),
    ("  armed a save request", "armed"),
    ("    after an unsaved one", "armed_after_incomplete"),
    ("  continuation prompts", "continuations"),
    ("stop hook runs", "stop_hook_runs"),
    ("  timed out", "stop_hook_timeouts"),
    ("  exited non-zero", "stop_hook_failures"),
    ("save requested", "requested"),
    ("saved", "saved"),
    ("held", "held"),
    ("omitted", "omitted"),
    ("failed", "failed"),
    ("refused saves", "refused"),
    ("errors", "errors"),
    ("learnings written", "learnings_written"),
    ("chunks promoted", "chunks_promoted"),
)


def format_continuations(continuations: Dict[str, Any]) -> List[str]:
    lines = ["", "Native save requests in the transcripts (median per request):"]
    for host, summary in continuations.items():
        if not summary["requests"]:
            lines.append(f"  {host:<7} none")
            continue
        share = f", {summary['time_share']:.0%} of the answer's time" if summary["time_share"] is not None else ""
        lines.append(f"  {host:<7} {summary['requests']} request(s): +{summary['save_seconds']} s and"
                     f" +{summary['save_output_tokens']:.0f} output tokens after an answer of"
                     f" {summary['answer_seconds']} s and {summary['answer_output_tokens']:.0f} tokens{share}")
    return lines


def format_native(native: Dict[str, Any]) -> List[str]:
    lines = [""]
    if not native["journal"]:
        lines.append("Native result funnel: no native-results.ndjson in memory-bank/local"
                     " (a runtime before the journal, or no native step ran)")
    else:
        lines.append(f"Native result funnel: {native['lines']} journal line(s)")
        unknown = any(native["hosts"]["unknown"][key] for key in NATIVE_COUNTERS)
        columns = [*NATIVE_HOSTS, *(("unknown",) if unknown else ()), "total"]
        width = max(len(title) for title, _ in NATIVE_ROWS) + 2
        lines.append(" " * width + "".join(column.rjust(9) for column in columns))
        sources = {**native["hosts"], "total": native["total"]}
        for title, key in NATIVE_ROWS:
            lines.append(title.ljust(width) + "".join(str(sources[column][key]).rjust(9) for column in columns))
        if native["total"]["reasons"]:
            lines.append("")
            lines.append("codes, all hosts:")
            for step, codes in native["total"]["reasons"].items():
                for code, count in codes.items():
                    lines.append(f"  {step:<18} {code:<46} {count:>6}")
    lines.append(f"prompt hook runs, all hosts (refresh-health.ndjson): {native['prompt_hook_health_lines']}")
    return lines


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
    run.add_argument(
        "--diagnose-misses", action="store_true",
        help="opt in to pinned-clock, judged-path retrieval-stage diagnostics",
    )
    run.add_argument(
        "--audit-git", action="store_true",
        help=(
            "diagnostic only: seed a temporary Git snapshot in each evaluation "
            "corpus so history admission can be audited without the project's real .git"
        ),
    )
    run.add_argument("--onnx-excerpts", type=Path,
                     help="maintainer-only pilot: local hash-locked asset directory; off by default")
    run.add_argument("--onnx-python", type=Path, help="explicit isolated Python for the optional ONNX worker")
    run.add_argument("--onnx-timeout", type=float, default=5.0, help="hard worker timeout; failure keeps lexical excerpts")
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
    realized.add_argument("--state", help="the accelerator state directory of an attached project"
                          " (default: the project)")
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
