#!/usr/bin/env python3
"""Governed Project Brain storage primitives (Python 3.9+, standard library only)."""

from __future__ import annotations

import contextlib
import errno
import hashlib
import json
import marshal
import os
import re
import stat
import sys
import tempfile
import threading
import time
import uuid
from datetime import date, datetime, timedelta, timezone
from pathlib import Path
from typing import Any, Iterable, Iterator, Optional

import workspace_roots
from validate import (
    FILENAME_PATTERN as BANK_FILENAME_PATTERN,
    ValidationError as BankValidationError,
    display_path as bank_display_path,
    parse_frontmatter as parse_bank_frontmatter,
    SECRET_PATTERNS as BANK_SECRET_PATTERNS,
    sensitive_label,
    validate_bank,
    validate_metadata as validate_bank_metadata,
    validate_secret_patterns as validate_bank_secrets,
)


def _file_lock(handle, unlock=False):
    if os.name != "nt":
        import fcntl
        fcntl.flock(handle.fileno(), fcntl.LOCK_UN if unlock else fcntl.LOCK_EX)
        return
    import msvcrt
    while True:
        handle.seek(0)
        try:
            msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK if unlock else msvcrt.LK_NBLCK, 1)
            return
        except OSError as error:
            if unlock or error.errno not in (errno.EACCES, errno.EAGAIN, errno.EDEADLK):
                raise
            time.sleep(.05)


SCHEMA_VERSION = 1
UUID4_PATTERN = re.compile(
    r"^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"
)
RECORD_TYPES = ("task", "finding", "bug", "incident", "decision", "event")
# Stored phases use one governed vocabulary. Compatibility aliases are accepted
# only at mutation boundaries and are normalized before validation/persistence.
TASK_PHASES = (
    "understanding",
    "planning",
    "implementation",
    "verification",
    "finalization",
)
TASK_PHASE_ALIASES = {
    "implementing": "implementation",
    "execution": "implementation",
    "review": "verification",
    # SKILL FLOW.md's Phase Map names the reviewing/testing row "Quality", so
    # an agent following it reaches for this word.
    "quality": "verification",
}
TASK_PHASE_INPUTS = (*TASK_PHASES, *TASK_PHASE_ALIASES)
# The agent message channel: an append-only JSONL journal per task under
# control/messages. `finding`/`question`/`handoff` carry agent-to-agent
# content; `dispatch`/`completion` are the orchestration log (who was spawned
# with which capsule, who finished). Actors are roster agent slugs plus the
# reserved `main` (the orchestrating conversation); `*` addresses everyone
# and is legal only as a recipient.
MESSAGE_TYPES = ("finding", "question", "handoff", "dispatch", "completion")
MESSAGE_BODY_LIMIT = 8000
ACTOR_PATTERN = re.compile(r"^[a-z0-9][a-z0-9_-]{0,63}$")
PRIVACY = ("public", "team", "restricted", "private")
AUTHORITIES = ("inferred", "observed", "verified")
# Authority may only harden, along a single edge: an `observed` claim that a
# verification re-checked becomes `verified`. Nothing downgrades and `inferred`
# never skips ahead, because automatic promotion trusts `verified` to mean a
# verification actually happened. Authority values never collide with any
# lifecycle status below, which is what lets both kinds of transition share
# one append-only ledger and stay distinguishable forever.
AUTHORITY_TRANSITIONS: dict[str, set[str]] = {"observed": {"verified"}}
LIFECYCLES: dict[str, dict[str, Any]] = {
    "task": {
        "initial": "active",
        "transitions": {
            "active": {"blocked", "verifying", "completed", "cancelled"},
            "blocked": {"active", "cancelled"},
            "verifying": {"active", "completed", "cancelled"},
            "completed": set(),
            "cancelled": set(),
        },
        "terminal": {"completed", "cancelled"},
    },
    "finding": {
        "initial": "open",
        "transitions": {
            "open": {"investigating", "resolved", "superseded"},
            "investigating": {"open", "resolved", "superseded"},
            "resolved": set(),
            "superseded": set(),
        },
        "terminal": {"resolved", "superseded"},
    },
    "bug": {
        "initial": "reported",
        "transitions": {
            "reported": {"triaged", "cancelled"},
            "triaged": {"fixing", "cancelled"},
            "fixing": {"verifying", "triaged", "cancelled"},
            "verifying": {"fixing", "resolved", "cancelled"},
            "resolved": set(),
            "cancelled": set(),
        },
        "terminal": {"resolved", "cancelled"},
    },
    "incident": {
        "initial": "open",
        "transitions": {
            "open": {"contained", "cancelled"},
            "contained": {"recovering", "resolved"},
            "recovering": {"contained", "resolved"},
            "resolved": {"closed"},
            "closed": set(),
            "cancelled": set(),
        },
        "terminal": {"closed", "cancelled"},
    },
    "decision": {
        "initial": "proposed",
        "transitions": {
            "proposed": {"accepted", "rejected"},
            "accepted": {"superseded"},
            "rejected": set(),
            "superseded": set(),
        },
        "terminal": {"rejected", "superseded"},
    },
    "event": {
        "initial": "recorded",
        "transitions": {"recorded": {"superseded"}, "superseded": set()},
        "terminal": {"superseded"},
    },
}
TERMINAL_STATES = {
    status
    for lifecycle in LIFECYCLES.values()
    for status in lifecycle["terminal"]
}
_LOCKS: dict[str, threading.RLock] = {}
_LOCKS_GUARD = threading.Lock()
_LOCK_STATE = threading.local()


class BrainError(Exception):
    """A safe, user-facing Project Brain error."""


def linked_reparse(image: os.stat_result) -> bool:
    """Whether a Windows reparse point redirects its path - a symlink or a
    junction - or cannot be told apart from one. Other reparse points (a
    OneDrive placeholder, a deduplicated file) are the file itself with its
    storage elsewhere; refusing them switched the Brain off on such volumes."""
    if not getattr(image, 'st_file_attributes', 0) & 0x400:
        return False
    tag = getattr(image, 'st_reparse_tag', 0)
    return not tag or tag in workspace_roots.LINK_REPARSE_TAGS


def _require_utf8_record_path(path: Path) -> Path:
    """Brain paths become record keys and index entries, so they are UTF-8 -
    from the project-brain directory down. The directories above the project
    are the machine's own and are never stored: one Latin-1 name in a home
    directory used to switch the whole Brain off."""
    parts = path.parts
    try:
        start = len(parts) - 1 - parts[::-1].index("project-brain")
    except ValueError:
        start = max(len(parts) - 1, 0)
    try:
        os.sep.join(parts[start:]).encode('utf-8')
    except UnicodeError as error:
        raise BrainError('Project Brain source invalid-utf8-path') from error
    return path


class BrainRecordNotFound(BrainError):
    """Positive absence after exhausting a readable native record inventory."""
    def __init__(self, identifier: str, record_type: Optional[str] = None):
        self.identifier = identifier
        self.record_type = record_type
        super().__init__(f"Project Brain {record_type or 'record'} not found: {identifier}")


class BrainRecordReadError(BrainError):
    """A fixed read diagnostic; never the decoder's raw byte/error prose."""
    def __init__(self, path: Path, reason: str):
        self.path = path
        self.reason = reason
        description = 'invalid UTF-8' if reason == 'invalid-utf8' else 'record unavailable'
        super().__init__(f"{path}: {description}")


def normalize_phase(value: str) -> str:
    """Return the canonical stored phase for a CLI/API input value."""
    normalized = TASK_PHASE_ALIASES.get(value, value)
    if normalized not in TASK_PHASES:
        raise BrainError(
            "phase must be one of: "
            + ", ".join(TASK_PHASES)
            + " (aliases: "
            + ", ".join(
                f"{alias}->{canonical}"
                for alias, canonical in TASK_PHASE_ALIASES.items()
            )
            + ")"
        )
    return normalized


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def new_uuid() -> str:
    return str(uuid.uuid4())


def is_uuid4(value: object) -> bool:
    return isinstance(value, str) and UUID4_PATTERN.fullmatch(value.lower()) is not None


def brain_root(repository: Path) -> Path:
    return repository / "project-brain"


# Shared evidence of memory-result saves (memory_results.py), relative to the
# Brain root: a folder per task of attempts and completion proofs, and the
# image of each learning an agent last wrote.
RESULT_ATTEMPTS = ("control", "result-attempts")
RESULT_LEARNING_IMAGES = ("control", "result-learning-images")
_EVIDENCE_NAME = re.compile(r"[0-9a-f]{64}\.json")


def _admit_brain_anchor(repository: Path) -> bool:
    """Reject an observed linked/reparse namespace before reads or lock writes."""
    anchor = _require_utf8_record_path(brain_root(repository))
    try:
        image = anchor.lstat()
    except FileNotFoundError:
        return False
    except OSError as error:
        raise BrainError(f"Project Brain inventory unavailable: {anchor}") from error
    if (not stat.S_ISDIR(image.st_mode)
            or linked_reparse(image)):
        raise BrainError(f"Project Brain inventory unavailable: {anchor}")
    return True


# What an installed accelerator's state looks like, for a state directory an
# attached project starts without. Tooling - schemas, templates, scripts,
# PROTOCOL.md - is never copied: it is read from the clone, so `git pull`
# updates it everywhere at once.
ATTACHED_STATE_DIRECTORIES = (
    "project-brain/archive",
    "project-brain/control/handoffs",
    "project-brain/control/messages",
    "project-brain/control/promotions",
    "project-brain/control/retrieval-manifests",
    "project-brain/dynamic/bugs",
    "project-brain/dynamic/decisions",
    "project-brain/dynamic/events",
    "project-brain/dynamic/findings",
    "project-brain/dynamic/incidents",
    "project-brain/dynamic/tasks",
    "project-brain/indexes",
    "project-brain/local",
    "memory-bank/chunks",
    "memory-bank/local",
)
# The state file and where the accelerator keeps its starting content. The
# configuration is copied because it is per-project policy (mode, owners,
# retention) that a team may change; the rest is the installer's own seed.
ATTACHED_STATE_SEEDS = (
    ("project-brain/config/runtime.json", "project-brain/config/runtime.json"),
    ("project-brain/config/telemetry.json", "project-brain/config/telemetry.json"),
    ("project-brain/config/providers.json", "project-brain/config/providers.json"),
    ("project-brain/indexes/active.json", "project-brain/.install/active.json"),
    ("project-brain/indexes/archive.json", "project-brain/.install/archive.json"),
    ("memory-bank/INDEX.md", "memory-bank/.install/INDEX.md"),
)
# Everything an attached state directory holds: the runtime's layout and the
# launcher's record of the project (scripts/accelerator_attach.py).
ATTACHED_STATE_ENTRIES = ("project-brain", "memory-bank", "launch", "accelerator-attach.json")
# An attached state is a project's memory kept outside the project, so it is
# its owner's alone: directories 0700 and files 0600, whatever the umask of
# the shell or the tool that started the runtime. Installed state lives in the
# project and keeps the project's permissions.
PRIVATE_DIRECTORY_MODE = 0o700


def make_private_directory(path: Path) -> bool:
    """Create `path` and every parent it lacks owner-only, whatever the umask.

    A directory that exists already is left as it is - above the state it is
    not the accelerator's; inside, `secure_attached_state` decides. Returns
    whether `path` itself was created here.
    """
    missing: list[Path] = []
    current = path
    while not os.path.lexists(current) and current.parent != current:
        missing.append(current)
        current = current.parent
    created = False
    for directory in reversed(missing):
        try:
            os.mkdir(directory, PRIVATE_DIRECTORY_MODE)
        except FileExistsError:
            # A concurrent first run made it.
            created = False
            continue
        created = True
        if os.name != "nt":
            # mkdir's mode passes through the umask; this does not.
            os.chmod(directory, PRIVATE_DIRECTORY_MODE)
    return created


def _open_to_others(path: Path) -> bool:
    """Whether a group or other user may use this entry, which the current
    user owns; a link and another user's file are never the runtime's to
    change, and Windows permissions are not mode bits."""
    if os.name == "nt":
        return False
    try:
        status = os.lstat(path)
    except OSError:
        return False
    return (
        not stat.S_ISLNK(status.st_mode)
        and status.st_uid == os.geteuid()
        and bool(status.st_mode & 0o077)
    )


def _owner_only(path: Path) -> None:
    """Take group and other access away from one entry; best effort."""
    if not _open_to_others(path):
        return
    try:
        os.chmod(path, stat.S_IMODE(os.lstat(path).st_mode) & 0o700)
    except OSError:
        return


def secure_attached_state(repository: Path) -> None:
    """Keep an attached state directory owner-only.

    A missing state directory is created owner-only. One that exists - made
    before this rule, or touched by a shell with a looser umask - is tightened
    where it is the accelerator's: each accelerator entry that is open to
    others, with everything under it, and the directory itself when it holds
    nothing but accelerator state (a variable can name any directory). Links
    are never followed, nothing above the state is changed, and another user's
    file is left alone.
    """
    make_private_directory(repository)
    if os.name == "nt":
        return
    try:
        names = set(os.listdir(repository))
    except OSError:
        return
    for name in ATTACHED_STATE_ENTRIES:
        entry = repository / name
        if name not in names or not _open_to_others(entry):
            continue
        if entry.is_dir():
            # os.walk does not descend through a link to a directory.
            for directory, folders, files in os.walk(entry):
                for child in folders + files:
                    _owner_only(Path(directory) / child)
        # Last: a pass that was cut short is taken up again on the next run.
        _owner_only(entry)
    if names <= set(ATTACHED_STATE_ENTRIES):
        _owner_only(repository)


def _missing_attached_state(repository: Path, tooling: Path) -> bool:
    return any(not (repository / directory).is_dir() for directory in ATTACHED_STATE_DIRECTORIES) or any(
        not (repository / target).exists() and (tooling / source).is_file()
        for target, source in ATTACHED_STATE_SEEDS
    )


def ensure_attached_state(repository: Path) -> list[str]:
    """Give an attached project's state directory an installed layout.

    Creates only what is missing and never overwrites, so it is safe on every
    run; returns the state paths it created. Installed, it does nothing.

    A session's first runs start together - Claude Code runs the hooks of one
    event in parallel - and one must never take a seed another is still
    writing for a finished one: a half-written runtime.json is an invalid
    configuration. So a seed appears whole or not at all (written beside its
    place, then renamed into it), and a layout that lacks anything is laid out
    under the mutation lock, checked again once the lock is held. The lock's
    own directory is made first, so the lock works in an empty state. All of
    it is owner-only (`secure_attached_state`).
    """
    if not workspace_roots.is_attached(repository):
        return []
    tooling = workspace_roots.tooling_root(repository)
    secure_attached_state(repository)
    if not _missing_attached_state(repository, tooling):
        return []
    created: list[str] = []
    if make_private_directory(brain_root(repository) / "local"):
        created.append("project-brain/local/")
    with mutation_lock(repository):
        for directory in ATTACHED_STATE_DIRECTORIES:
            if not (repository / directory).is_dir():
                make_private_directory(repository / directory)
                created.append(directory + "/")
        for target, source in ATTACHED_STATE_SEEDS:
            path = repository / target
            origin = tooling / source
            if path.exists() or not origin.is_file():
                continue
            make_private_directory(path.parent)
            atomic_write_bytes(path, origin.read_bytes())
            created.append(target)
    return created


def _runtime_config_present(path: Path) -> bool:
    """Distinguish a missing layout from a dangling or unavailable entry."""
    probe = path
    missing = False
    while True:
        try:
            image = probe.lstat()
        except FileNotFoundError:
            missing = True
            parent = probe.parent
            if parent == probe:
                raise BrainError(f"Invalid Project Brain configuration: {path}")
            probe = parent
            continue
        if (stat.S_ISLNK(image.st_mode)
                or getattr(image, "st_reparse_tag", None) in workspace_roots.LINK_REPARSE_TAGS):
            image = probe.stat()
        if missing:
            if not stat.S_ISDIR(image.st_mode):
                raise BrainError(f"Invalid Project Brain configuration: {path}")
            return False
        if not stat.S_ISREG(image.st_mode):
            raise BrainError(f"Invalid Project Brain configuration: {path}")
        return True


def read_runtime_config_text(repository: Path) -> Optional[str]:
    """Strict existing text view; None only on observed ordinary absence.

    Readable links retain their existing behavior. POSIX nonblocking open
    rejects a raced FIFO before reading. Windows retains regular-file
    observations without that POSIX race guarantee. Regular filesystem I/O
    has no universal wall deadline, and separate calls are not one snapshot.
    """
    path = brain_root(repository) / "config" / "runtime.json"
    try:
        if not _runtime_config_present(path):
            return None
        flags = os.O_RDONLY | getattr(os, "O_BINARY", 0)
        nonblocking = getattr(os, "O_NONBLOCK", None)
        if nonblocking is None:
            if os.name != "nt":
                raise BrainError(f"Invalid Project Brain configuration: {path}")
        else:
            flags |= nonblocking
        flags |= getattr(os, "O_CLOEXEC", 0)
        descriptor = os.open(path, flags)
        try:
            if not stat.S_ISREG(os.fstat(descriptor).st_mode):
                raise BrainError(f"Invalid Project Brain configuration: {path}")
            raw = bytearray()
            while True:
                block = os.read(descriptor, 65536)
                if not block:
                    break
                raw.extend(block)
            text = raw.decode("utf-8").replace("\r\n", "\n").replace("\r", "\n")
        except BaseException:
            # One close owner; an ordinary cleanup failure cannot hide the
            # original error/cancellation. A close cancellation propagates.
            try:
                os.close(descriptor)
            except OSError:
                pass
            raise
        else:
            # A failed successful-path close refuses the result. Never retry
            # a descriptor whose closure may already have taken effect.
            os.close(descriptor)
        return text
    except (OSError, UnicodeError) as error:
        raise BrainError(f"Invalid Project Brain configuration: {path}") from error


def load_config(repository: Path) -> dict[str, Any]:
    path = brain_root(repository) / "config" / "runtime.json"
    defaults: dict[str, Any] = {
        "mode": "governed",
        "framework": "generic",
        "canonical_edition": ".agents",
        "allowed_privacy": ["public", "team"],
        "allowed_authority": ["observed", "verified"],
        "owners": ["*"],
        "telemetry_enabled": False,
        "provider": "sqlite-fts5",
        "automatic_promotion": False,
        "automatic_completion": False,
        "automatic_consolidation": False,
        "automatic_consolidation_verification": False,
        "automatic_compaction": False,
        "compaction_threshold": 5,
        "codebase_map_max_drift": 25,
    }
    text = read_runtime_config_text(repository)
    if text is None:
        return defaults
    try:
        value = json.loads(text)
    except (OSError, json.JSONDecodeError) as error:
        raise BrainError(f"Invalid Project Brain configuration: {path}") from error
    if not isinstance(value, dict):
        raise BrainError("Project Brain configuration must be a JSON object")
    return {**defaults, **value}


def configured_mode(repository: Path, requested: Optional[str] = None) -> str:
    mode = requested or os.environ.get("PROJECT_BRAIN_MODE") or load_config(repository)["mode"]
    if mode not in {"governed", "lightweight"}:
        raise BrainError("Mode must be governed or lightweight")
    return mode


CAPSULE_PROFILES = ("full", "navigation")


def configured_capsule_profile(repository: Path, requested: Optional[str] = None) -> str:
    """What the automatic capsule carries: flag, then env, then config, then `full`.

    `full` is the established capsule: working state, navigation and the
    retrieved semantic and episodic documents. `navigation` keeps the
    working state and the recent files and searches nothing. It exists as an
    experiment switch: the task-outcome measurement of 2026-10-21 showed no
    difference between the full capsule and none, and a capsule without the
    document layers has to be measured before it could become the default.
    An explicit `retrieve` is never affected; only the automatic entry points
    read this. A caller of `refresh` that is itself an explicit request - the
    MCP `memory_retrieve` tool, the evaluation stand - passes
    `--capsule-profile full`. A bad value is rejected here, like a bad
    retrieval gate.
    """
    profile = (
        requested
        or os.environ.get("CONTEXT_CAPSULE_PROFILE")
        or load_config(repository).get("capsule_profile")
        or "full"
    )
    if profile not in CAPSULE_PROFILES:
        raise BrainError("Capsule profile must be full or navigation")
    return profile


def configured_retrieval_gate(
    repository: Path, requested: Optional[str] = None
) -> str:
    """Which gate mode this run uses: flag, then env, then config, then shadow.

    Same ladder as `configured_mode`, and validated here rather than only by
    argparse: `hook-context` never sees the flag, so a bad value in config or
    in the environment has to be rejected where every caller passes through.
    """
    gate = (
        requested
        or os.environ.get("CONTEXT_RETRIEVAL_GATE")
        or load_config(repository).get("retrieval_gate")
        or "shadow"
    )
    if gate not in {"off", "shadow", "enforce"}:
        raise BrainError("Retrieval gate must be off, shadow or enforce")
    return gate


@contextlib.contextmanager
def mutation_lock(repository: Path) -> Iterator[None]:
    """Serialize every repository mutation through one global lock file."""
    _admit_brain_anchor(repository)
    lock_path = brain_root(repository) / "local" / "runtime.lock"
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    key = str(lock_path.resolve())
    with _LOCKS_GUARD:
        process_lock = _LOCKS.setdefault(key, threading.RLock())
    depths = getattr(_LOCK_STATE, "depths", {})
    _LOCK_STATE.depths = depths
    with process_lock:
        if depths.get(key, 0):
            depths[key] += 1
            try:
                yield
            finally:
                depths[key] -= 1
            return
        with lock_path.open("a+", encoding="utf-8") as handle:
            _file_lock(handle)
            depths[key] = 1
            try:
                yield
            finally:
                depths.pop(key, None)
                _file_lock(handle, unlock=True)


def atomic_write_bytes(path: Path, content: bytes) -> None:
    """Replace `path` whole: a reader sees the old content or the new, never
    a part. The temporary file beside it is owner-only, as mkstemp makes it,
    and the rename keeps that mode."""
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=str(path.parent))
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(str(temporary), str(path))
    finally:
        if temporary.exists():
            temporary.unlink()


def atomic_write(path: Path, content: str) -> None:
    # Encoded as written, newlines untranslated: Git-tracked records keep LF
    # on Windows too.
    atomic_write_bytes(path, content.encode("utf-8"))


def atomic_json(path: Path, value: object) -> None:
    atomic_write(path, json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n")


FileSnapshot = dict[Path, Optional[bytes]]


def snapshot_files(paths: Iterator[Path] | list[Path] | tuple[Path, ...]) -> FileSnapshot:
    return {
        path: path.read_bytes() if path.is_file() else None
        for path in dict.fromkeys(paths)
    }


def restore_files(snapshot: FileSnapshot) -> None:
    for path, content in snapshot.items():
        if content is None:
            if path.exists():
                path.unlink()
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            descriptor, temporary_name = tempfile.mkstemp(
                prefix=f".{path.name}.restore.", dir=str(path.parent)
            )
            temporary = Path(temporary_name)
            try:
                with os.fdopen(descriptor, "wb") as handle:
                    handle.write(content)
                    handle.flush()
                    os.fsync(handle.fileno())
                os.replace(str(temporary), str(path))
            finally:
                if temporary.exists():
                    temporary.unlink()


def index_paths(repository: Path) -> tuple[Path, Path]:
    root = brain_root(repository) / "indexes"
    return root / "active.json", root / "archive.json"


def read_record_text(path: Path) -> str:
    """Capture one native record image or retain an explicit unknown outcome."""
    _require_utf8_record_path(path)
    try:
        return path.read_text(encoding="utf-8")
    except UnicodeDecodeError as error:
        raise BrainRecordReadError(path, 'invalid-utf8') from error
    except OSError as error:
        raise BrainRecordReadError(path, 'unavailable') from error


def parse_markdown_record(
    path: Path, *, text: Optional[str] = None
) -> tuple[dict[str, Any], str]:
    """Parse one image; callers that also hash it can supply the same text."""
    _require_utf8_record_path(path)
    if text is None:
        text = read_record_text(path)
    if not text.startswith("---\n"):
        raise BrainError(f"{path}: missing JSON frontmatter")
    try:
        raw, body = text[4:].split("\n---\n", 1)
        metadata = json.loads(raw)
    except (ValueError, json.JSONDecodeError, RecursionError) as error:
        raise BrainError(f"{path}: invalid JSON frontmatter") from error
    if not isinstance(metadata, dict):
        raise BrainError(f"{path}: frontmatter must be an object")
    # Text read as UTF-8 holds no lone surrogate of its own; only a JSON
    # `\u` escape can decode to one, so frontmatter without one needs no
    # walk. validate_record checks the record again in any case.
    if "\\u" in raw:
        try:
            _validate_metadata_text(metadata)
        except BrainError as error:
            raise BrainError(f"{path}: invalid Unicode metadata") from error
    # Records persisted before the canonical phase vocabulary used
    # `execution`. Normalize that legacy value on read; the next supported
    # mutation rewrites the record and handoff with `implementation`.
    if metadata.get("phase") == "execution":
        metadata = dict(metadata)
        metadata["phase"] = "implementation"
    return metadata, body.lstrip("\n")


def render_markdown_record(metadata: dict[str, Any], body: str) -> str:
    return (
        "---\n"
        + json.dumps(metadata, ensure_ascii=False, indent=2, sort_keys=True)
        + "\n---\n\n"
        + body.rstrip()
        + "\n"
    )


def _schema_target(root: dict[str, Any], reference: str) -> dict[str, Any]:
    if not reference.startswith("#/"):
        raise BrainError(f"unsupported external schema reference: {reference}")
    target: Any = root
    for part in reference[2:].split("/"):
        target = target[part.replace("~1", "/").replace("~0", "~")]
    if not isinstance(target, dict):
        raise BrainError(f"invalid schema reference: {reference}")
    return target


def validate_schema_value(
    value: Any,
    schema: dict[str, Any],
    *,
    root: Optional[dict[str, Any]] = None,
    location: str = "$",
) -> None:
    root = root or schema
    if "$ref" in schema:
        validate_schema_value(
            value,
            _schema_target(root, schema["$ref"]),
            root=root,
            location=location,
        )
        return
    if "const" in schema and value != schema["const"]:
        raise BrainError(f"{location}: value must equal {schema['const']!r}")
    if "enum" in schema and value not in schema["enum"]:
        raise BrainError(f"{location}: value is outside schema enum")
    declared_type = schema.get("type")
    accepted = declared_type if isinstance(declared_type, list) else [declared_type]
    type_checks = {
        "object": lambda item: isinstance(item, dict),
        "array": lambda item: isinstance(item, list),
        "string": lambda item: isinstance(item, str),
        "integer": lambda item: isinstance(item, int) and not isinstance(item, bool),
        "number": lambda item: isinstance(item, (int, float)) and not isinstance(item, bool),
        "boolean": lambda item: isinstance(item, bool),
        "null": lambda item: item is None,
    }
    if declared_type is not None and not any(
        type_checks[item](value) for item in accepted
    ):
        raise BrainError(f"{location}: value has invalid schema type")
    if isinstance(value, dict):
        required = set(schema.get("required", []))
        missing = required - value.keys()
        if missing:
            raise BrainError(f"{location}: missing keys {', '.join(sorted(missing))}")
        properties = schema.get("properties", {})
        if schema.get("additionalProperties") is False:
            extra = value.keys() - properties.keys()
            if extra:
                raise BrainError(
                    f"{location}: unexpected keys {', '.join(sorted(extra))}"
                )
        for key, child in properties.items():
            if key in value:
                validate_schema_value(
                    value[key], child, root=root, location=f"{location}.{key}"
                )
    if isinstance(value, list):
        if len(value) < schema.get("minItems", 0):
            raise BrainError(f"{location}: too few items")
        if schema.get("uniqueItems"):
            serialized = [json.dumps(item, sort_keys=True) for item in value]
            if len(serialized) != len(set(serialized)):
                raise BrainError(f"{location}: items must be unique")
        if isinstance(schema.get("items"), dict):
            for index, item in enumerate(value):
                validate_schema_value(
                    item,
                    schema["items"],
                    root=root,
                    location=f"{location}[{index}]",
                )
    if isinstance(value, str):
        if len(value) < schema.get("minLength", 0):
            raise BrainError(f"{location}: string is too short")
        if "pattern" in schema and re.fullmatch(schema["pattern"], value) is None:
            raise BrainError(f"{location}: string does not match schema pattern")
        if schema.get("format") == "uuid" and not is_uuid4(value):
            raise BrainError(f"{location}: value must be UUIDv4")
        if schema.get("format") == "date-time":
            try:
                datetime.fromisoformat(value.replace("Z", "+00:00"))
            except ValueError as error:
                raise BrainError(f"{location}: invalid date-time") from error
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        if "minimum" in schema and value < schema["minimum"]:
            raise BrainError(f"{location}: value is below minimum")
        if "maximum" in schema and value > schema["maximum"]:
            raise BrainError(f"{location}: value is above maximum")


def validate_schema_file(repository: Path, name: str, value: Any) -> None:
    path = brain_root(repository) / "schemas" / name
    if not path.is_file():
        # Attached, schemas ship with the accelerator rather than the state.
        path = brain_root(workspace_roots.tooling_root(repository)) / "schemas" / name
    if not path.is_file():
        return
    try:
        schema = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise BrainError(f"Cannot load Project Brain schema: {path}") from error
    validate_schema_value(value, schema)


# Per thread: a pass in one thread never answers from another's.
_CITED_FILES = threading.local()


@contextlib.contextmanager
def cited_file_memo() -> Iterator[None]:
    """Answer each cited path once within a read-only pass over the Brain.

    A prompt's index rebuild asked the filesystem again for every record
    citing the same file - three path resolutions each, most of the
    rebuild's eligibility cost. The answers live only inside the `with`,
    where nothing creates, moves or removes a file.
    """
    previous = getattr(_CITED_FILES, "memo", None)
    _CITED_FILES.memo = {}
    try:
        yield
    finally:
        _CITED_FILES.memo = previous


def _cited_file(repository: Path, source: str) -> Path:
    """The regular file inside the repository a citation names."""
    relative = source.split("#", 1)[0]
    memo = getattr(_CITED_FILES, "memo", None)
    key = (os.fspath(repository), relative)
    if memo is not None and key in memo:
        answer = memo[key]
        if isinstance(answer, Path):
            return answer
        raise BrainError(answer)
    try:
        path = workspace_roots.resolve(repository, relative).resolve()
        if not workspace_roots.contains(repository, path):
            raise BrainError(f"Source escapes repository: {relative}")
        if not path.is_file() or path.is_symlink():
            raise BrainError(f"Source does not exist or is not a regular file: {relative}")
    except BrainError as error:
        if memo is not None:
            memo[key] = str(error)
        raise
    if memo is not None:
        memo[key] = path
    return path


class _CitationDigestCache:
    """One-call CPU cache; every fingerprint still checks and reads its source."""
    def __init__(self):
        self._digests: dict[bytes, str] = {}
        self._retained_bytes = 0
        self.hits = self.misses = self.bypasses = 0

    def digest(self, data: bytes) -> str:
        cached = self._digests.get(data)
        if cached is not None:
            self.hits += 1
            return cached
        self.misses += 1
        digest = hashlib.sha256(_line_ending_neutral(data)).hexdigest()
        if (len(data) <= 1048576 and len(self._digests) < 64
                and self._retained_bytes + len(data) <= 8388608):
            self._digests[data] = digest
            self._retained_bytes += len(data)
        else:
            self.bypasses += 1
        return digest


def fingerprint(repository: Path, source: str, *,
                _normalization_cache: Optional[_CitationDigestCache] = None) -> dict[str, str]:
    path = _cited_file(repository, source)
    data = path.read_bytes()
    digest = (_normalization_cache.digest(data) if _normalization_cache is not None
              else hashlib.sha256(_line_ending_neutral(data)).hexdigest())
    return {"path": source.split("#", 1)[0], "sha256": digest}


def _line_ending_neutral(data: bytes) -> bytes:
    """Text bytes with CRLF folded to LF; binary data unchanged.

    A checkout with `core.autocrlf=true` holds the same commit as one without
    it, yet a raw-byte digest differed, so every record citing a text file was
    `stale` on a colleague's Windows clone while Git saw no change. Folding
    leaves an LF file's digest exactly what it was, so stored fingerprints need
    no migration. A NUL byte marks binary data, as Git's own heuristic does.
    """
    if b"\0" in data[:8192]:
        return data
    return data.replace(b"\r\n", b"\n")


def source_fingerprints(repository: Path, sources: list[str]) -> list[dict[str, str]]:
    fingerprints = []
    for source in sorted(set(sources)):
        relative = source.split("#", 1)[0]
        path = workspace_roots.resolve(repository, relative).resolve()
        if not workspace_roots.contains(repository, path):
            raise BrainError(f"Source escapes repository: {relative}")
        if not path.is_file() or path.is_symlink():
            raise BrainError(
                f"Source does not exist or is not a regular file: {relative}"
            )
        fingerprints.append(fingerprint(repository, source))
    return fingerprints


def digestible_sources(sources: Any) -> list[str]:
    """The cited paths a digest can be taken of.

    A URL is a legitimate source and is deliberately not fingerprinted: the
    runtime has no network, so the only honest thing it can say about a remote
    citation is nothing.
    """
    if not isinstance(sources, list):
        return []
    return [
        source
        for source in sources
        if isinstance(source, str)
        and source
        and not source.startswith(("https://", "http://"))
    ]


def chunk_source_digests(repository: Path, sources: Any) -> list[dict[str, str]]:
    """Digest every local file a chunk cites, skipping what cannot be read."""
    digests: list[dict[str, str]] = []
    for source in sorted(set(digestible_sources(sources))):
        try:
            digests.append(fingerprint(repository, source))
        except BrainError:
            # A citation that does not resolve is validate.py's problem to
            # report, not a reason to refuse to digest the rest.
            continue
    return digests


def sources_are_fresh(repository: Path, record: dict[str, Any]) -> bool:
    fingerprints = record.get("source_fingerprints", [])
    if not isinstance(fingerprints, list):
        return False
    expected_paths = {source.split("#", 1)[0] for source in record.get("sources", [])}
    fingerprint_paths = {
        item.get("path") for item in fingerprints if isinstance(item, dict)
    }
    if expected_paths != fingerprint_paths:
        return False
    for item in fingerprints:
        if not isinstance(item, dict):
            return False
        source = item.get("path")
        expected = item.get("sha256")
        if not isinstance(source, str) or not isinstance(expected, str):
            return False
        try:
            if fingerprint(repository, source)["sha256"] != expected:
                return False
        except BrainError:
            return False
    return True


def source_changes(
    repository: Path, fingerprints: Any, *,
    _normalization_cache: Optional[_CitationDigestCache] = None
) -> tuple[list[str], list[str]]:
    """Cited paths whose digest moved on, and cited paths that are gone.

    `sources_are_fresh` answers yes or no; a reader deciding what to do with
    knowledge needs to tell an edited citation (the knowledge may still hold;
    check it) from a deleted one (nothing is left to check it against).
    """
    changed: list[str] = []
    missing: list[str] = []
    for item in fingerprints if isinstance(fingerprints, list) else []:
        path = item.get("path") if isinstance(item, dict) else None
        expected = item.get("sha256") if isinstance(item, dict) else None
        if not isinstance(path, str) or not isinstance(expected, str):
            missing.append(str(path))
            continue
        try:
            current = (fingerprint(repository, path) if _normalization_cache is None
                       else fingerprint(repository, path, _normalization_cache=_normalization_cache))["sha256"]
        except BrainError:
            missing.append(path)
            continue
        if current != expected:
            changed.append(path)
    return changed, missing


def citation_problem(repository: Path, record: dict[str, Any]) -> Optional[str]:
    """Why a record's citations cannot be checked at all, or None.

    `source-undigested`: a cited local path has no stored digest, or the
    stored digests are malformed - there is nothing to compare an edit with.
    `source-missing`: a digested path no longer resolves to a regular file in
    the repository - there is nothing left to check the knowledge against.
    An EDITED citation is not a problem here: retrieval serves the record
    marked `source_changed` and ranked at SOURCE_CHANGED_WEIGHT. Existence
    only, no file is read, so the per-turn rebuild of Brain rows does not
    grow with the size of the files records cite.
    """
    fingerprints = record.get("source_fingerprints")
    if not isinstance(fingerprints, list):
        return "source-undigested"
    digested: set[str] = set()
    for item in fingerprints:
        if (
            not isinstance(item, dict)
            or not isinstance(item.get("path"), str)
            or not isinstance(item.get("sha256"), str)
        ):
            return "source-undigested"
        digested.add(item["path"])
    expected = {source.split("#", 1)[0] for source in digestible_sources(record.get("sources"))}
    if not expected <= digested:
        return "source-undigested"
    for path in sorted(digested):
        try:
            _cited_file(repository, path)
        except (BrainError, OSError, ValueError):
            return "source-missing"
    return None


def _require_string(record: dict[str, Any], key: str) -> None:
    if not isinstance(record.get(key), str) or not record[key].strip():
        raise BrainError(f"{key} must be a non-empty string")


def _validate_metadata_text(value: Any) -> None:
    """JSON escapes must also produce text accepted by native UTF-8 storage."""
    try:
        # One pass in C for a JSON-shaped record: a lone surrogate anywhere,
        # key or value, fails the encode. What JSON cannot hold is walked.
        json.dumps(value, ensure_ascii=False).encode("utf-8")
        return
    except UnicodeEncodeError as error:
        raise BrainError("invalid Unicode metadata") from error
    except (TypeError, ValueError, RecursionError):
        pass
    pending = [value]
    visited: set[int] = set()
    while pending:
        current = pending.pop()
        if isinstance(current, str):
            try:
                current.encode("utf-8")
            except UnicodeEncodeError as error:
                raise BrainError("invalid Unicode metadata") from error
        elif isinstance(current, (dict, list)) and id(current) not in visited:
            visited.add(id(current))
            if isinstance(current, dict):
                pending.extend(current.keys())
                pending.extend(current.values())
            else:
                pending.extend(current)


def _require_string_list(record: dict[str, Any], key: str) -> None:
    value = record.get(key)
    if not isinstance(value, list) or any(not isinstance(item, str) or not item for item in value):
        raise BrainError(f"{key} must be a list of non-empty strings")


def _require_datetime(record: dict[str, Any], key: str) -> None:
    _require_string(record, key)
    try:
        datetime.fromisoformat(record[key].replace("Z", "+00:00"))
    except ValueError as error:
        raise BrainError(f"{key} must be an RFC3339 timestamp") from error


def validate_record(record: dict[str, Any], *, archived: bool = False) -> None:
    if not isinstance(record, dict):
        raise BrainError('record must be an object')
    _validate_metadata_text(record)
    required = {
        "schema_version", "id", "type", "external_id", "title", "status", "revision",
        "owner", "authorized_owners", "privacy", "authority", "confidence", "created_at",
        "updated_at", "transitions", "source_fingerprints", "conflicts", "supersedes",
        "superseded_by", "goal", "progress", "next_steps", "files", "sources",
    }
    missing = required - record.keys()
    # `phase` and `auto_checkpoint` are optional: adding either to the required
    # set would invalidate every record written before it existed.
    extra = record.keys() - required - {"phase", "auto_checkpoint"}
    if missing:
        raise BrainError(f"record missing keys: {', '.join(sorted(missing))}")
    if extra:
        raise BrainError(f"record has unexpected keys: {', '.join(sorted(extra))}")
    if "phase" in record:
        if record["type"] != "task":
            raise BrainError("only a task may declare a phase")
        if record["phase"] not in TASK_PHASES:
            raise BrainError(f"phase must be one of: {', '.join(TASK_PHASES)}")
    if "auto_checkpoint" in record:
        # The automatic turn flush writes here so it never overwrites the
        # operator's own `progress`. An empty value carries no information and
        # therefore may not exist: the writer either has a checkpoint to report
        # or leaves the key absent.
        if record["type"] != "task":
            raise BrainError("only a task may record an auto checkpoint")
        _require_string(record, "auto_checkpoint")
    if type(record["schema_version"]) is not int or record["schema_version"] != SCHEMA_VERSION:
        raise BrainError("unsupported schema_version")
    if not is_uuid4(record["id"]):
        raise BrainError("record id must be UUIDv4")
    if record["type"] not in RECORD_TYPES:
        raise BrainError(f"type must be one of: {', '.join(RECORD_TYPES)}")
    for key in ("external_id", "title", "owner", "goal", "status", "privacy", "authority"):
        _require_string(record, key)
    if not isinstance(record['progress'], str):
        raise BrainError('progress must be text')
    for key in ("created_at", "updated_at"):
        _require_datetime(record, key)
    for key in (
        "authorized_owners", "conflicts", "supersedes", "next_steps", "files", "sources"
    ):
        _require_string_list(record, key)
        if len(record[key]) != len(set(record[key])):
            raise BrainError(f"{key} must contain unique values")
    if record["owner"] not in record["authorized_owners"]:
        raise BrainError("owner must be included in authorized_owners")
    if record["privacy"] not in PRIVACY:
        raise BrainError("invalid privacy")
    if record["authority"] not in AUTHORITIES:
        raise BrainError("invalid authority")
    if type(record["confidence"]) not in (int, float) or not 0 <= record["confidence"] <= 1:
        raise BrainError("confidence must be between 0 and 1")
    if type(record["revision"]) is not int or record["revision"] < 1:
        raise BrainError("revision must be a positive integer")
    lifecycle = LIFECYCLES[record["type"]]
    transitions = lifecycle["transitions"]
    if record["status"] not in transitions:
        raise BrainError(f"invalid {record['type']} status")
    if not isinstance(record["transitions"], list) or not record["transitions"]:
        raise BrainError("transitions must be a non-empty append-only list")
    previous: Optional[str] = None
    for index, transition in enumerate(record["transitions"]):
        if not isinstance(transition, dict):
            raise BrainError("transition must be an object")
        if set(transition) != {"from", "to", "at", "actor", "reason"}:
            raise BrainError("transition does not match strict schema")
        _require_datetime(transition, "at")
        _require_string(transition, "actor")
        _require_string(transition, "reason")
        current_from = transition.get("from")
        current_to = transition.get("to")
        if not isinstance(current_to, str) or (current_from is not None and not isinstance(current_from, str)):
            raise BrainError('transition endpoints must be text, with a null initial source')
        if index == 0:
            if current_from is not None or current_to != lifecycle["initial"]:
                raise BrainError(
                    f"first {record['type']} transition must create "
                    f"{lifecycle['initial']} state"
                )
        elif current_from in AUTHORITIES and current_to in AUTHORITIES:
            # An authority transition shares the ledger but not the status
            # chain: it must follow the one-way authority graph and leaves
            # `previous` untouched, so the surrounding lifecycle history
            # still has to line up end to end.
            if current_to not in AUTHORITY_TRANSITIONS.get(current_from, set()):
                raise BrainError(
                    f"illegal authority transition: {current_from} -> {current_to}"
                )
            continue
        elif current_from != previous or current_to not in transitions.get(current_from, set()):
            raise BrainError(f"illegal lifecycle transition: {current_from} -> {current_to}")
        previous = current_to
    if previous != record["status"]:
        raise BrainError("transition history does not match status")
    if not isinstance(record['source_fingerprints'], list):
        raise BrainError('source_fingerprints must be a list')
    for fingerprint_item in record["source_fingerprints"]:
        if (
            not isinstance(fingerprint_item, dict)
            or set(fingerprint_item) != {"path", "sha256"}
            or not isinstance(fingerprint_item["path"], str)
            or not isinstance(fingerprint_item['sha256'], str)
            or re.fullmatch(r"[0-9a-f]{64}", fingerprint_item["sha256"]) is None
        ):
            raise BrainError("source_fingerprints does not match strict schema")
    for relationship in [*record["conflicts"], *record["supersedes"]]:
        if not is_uuid4(relationship):
            raise BrainError("record relationships must use UUIDv4 IDs")
    if record["superseded_by"] is not None and not is_uuid4(record["superseded_by"]):
        raise BrainError("superseded_by must be null or UUIDv4")
    if archived and record["status"] not in lifecycle["terminal"]:
        raise BrainError("only terminal records may be archived")


def validate_handoff(record: dict[str, Any], task: Optional[dict[str, Any]] = None) -> None:
    if not isinstance(record, dict):
        raise BrainError('handoff must be an object')
    _validate_metadata_text(record)
    required = {
        "schema_version", "id", "type", "task_id", "task_revision", "revision",
        "owner", "privacy", "created_at", "updated_at", "objective", "current_state",
        "next_actions", "files", "sources", "status",
    }
    missing = required - record.keys()
    extra = record.keys() - required - {"phase"}
    if missing:
        raise BrainError(f"handoff missing keys: {', '.join(sorted(missing))}")
    if extra:
        raise BrainError(f"handoff has unexpected keys: {', '.join(sorted(extra))}")
    if type(record['schema_version']) is not int or record['schema_version'] != SCHEMA_VERSION:
        raise BrainError('unsupported handoff schema_version')
    if "phase" in record and record["phase"] not in TASK_PHASES:
        raise BrainError(f"phase must be one of: {', '.join(TASK_PHASES)}")
    if not is_uuid4(record["id"]) or not is_uuid4(record["task_id"]):
        raise BrainError("handoff and task IDs must be UUIDv4")
    _require_string(record, 'status')
    if record["type"] != "handoff" or record["status"] not in {"active", "closed"}:
        raise BrainError("invalid handoff type or status")
    if record["privacy"] not in PRIVACY:
        raise BrainError("invalid handoff privacy")
    for key in ("owner", "objective"):
        _require_string(record, key)
    if not isinstance(record['current_state'], str):
        raise BrainError('current_state must be text')
    for key in ("created_at", "updated_at"):
        _require_datetime(record, key)
    for key in ("task_revision", "revision"):
        if type(record[key]) is not int or record[key] < 1:
            raise BrainError(f"{key} must be a positive integer")
    for key in ("next_actions", "files", "sources"):
        _require_string_list(record, key)
    if task is not None:
        if record["task_id"] != task["id"] or record["task_revision"] != task["revision"]:
            raise BrainError("handoff task binding is stale")


def dynamic_path(repository: Path, record: dict[str, Any]) -> Path:
    plural = {
        "task": "tasks", "finding": "findings", "bug": "bugs", "incident": "incidents",
        "decision": "decisions", "event": "events",
    }[record["type"]]
    return brain_root(repository) / "dynamic" / plural / f"{record['id']}.md"


def handoff_path(repository: Path, task_uuid: str) -> Path:
    return brain_root(repository) / "control" / "handoffs" / f"{task_uuid}.md"


# A directory a writer changed while it was being listed is read again this
# many times before the inventory gives up: an atomic record write adds a
# temporary file beside the record and renames it over it.
_INVENTORY_ATTEMPTS = 6


def _record_inventory(root: Path) -> list[Path]:
    """Enumerate every candidate before yielding; failed traversal is unknown.

    This is an error-aware native inventory, not a filesystem snapshot or proof
    against unsynchronized external writers. No linked/reparse authority path
    is followed. Only a positively absent initial root can mean an empty tree.
    """
    return [path for path, _, _ in _record_inventory_stats(root)]


def _record_inventory_stats(root: Path) -> list[tuple[Path, os.stat_result, str]]:
    """_record_inventory with each record file's lstat, taken as it was
    listed, and its path as a string."""
    _require_utf8_record_path(root)

    def unavailable(candidate: Path) -> BrainError:
        return BrainError(f"Project Brain inventory unavailable: {candidate}")

    def directory(candidate: Path):
        try:
            image = candidate.lstat()
        except OSError as error:
            raise unavailable(candidate) from error
        if (not stat.S_ISDIR(image.st_mode)
                or linked_reparse(image)):
            raise unavailable(candidate)
        return image

    try:
        root.lstat()
    except FileNotFoundError:
        return []
    except OSError as error:
        raise unavailable(root) from error
    pending = [root]
    result: list[tuple[str, os.stat_result]] = []
    while pending:
        current = pending.pop()
        for attempt in range(_INVENTORY_ATTEMPTS):
            files: list[tuple[str, os.stat_result]] = []
            directories: list[Path] = []
            settled = True
            before = directory(current)
            try:
                with os.scandir(current) as scanner:
                    entries = sorted(scanner, key=lambda entry: entry.name)
            except OSError as error:
                raise unavailable(current) from error
            for entry in entries:
                # Strings until an entry is kept: a large Brain is listed many
                # times by one command, and Path objects dominated the cost.
                try:
                    entry.name.encode('utf-8')
                except UnicodeError as error:
                    raise BrainError('Project Brain source invalid-utf8-path') from error
                try:
                    # The entry's own type, without a system call where the
                    # platform reports it: a writer's temporary file may be
                    # renamed away before anything else looks at it.
                    if entry.is_symlink() or (
                            os.name == "nt"
                            and linked_reparse(entry.stat(follow_symlinks=False))):
                        raise unavailable(Path(entry.path))
                    if entry.is_dir(follow_symlinks=False):
                        directories.append(Path(entry.path))
                        continue
                except FileNotFoundError:
                    settled = False
                    break
                except OSError as error:
                    raise unavailable(Path(entry.path)) from error
                # What fnmatch(name, '*.md') decides, without its pattern work.
                if not os.path.normcase(entry.name).endswith('.md'):
                    continue
                try:
                    image = os.lstat(entry.path)
                except FileNotFoundError:
                    settled = False
                    break
                except OSError as error:
                    raise unavailable(Path(entry.path)) from error
                if (stat.S_ISLNK(image.st_mode) or not stat.S_ISREG(image.st_mode)
                        or linked_reparse(image)):
                    raise unavailable(Path(entry.path))
                files.append((entry.path, image))
            after = directory(current)
            if settled and all(getattr(before, key) == getattr(after, key) for key in
                               ('st_dev', 'st_ino', 'st_mode', 'st_mtime_ns', 'st_ctime_ns')):
                break
            # A writer replaced or moved a record while this directory was
            # listed; what was read may miss it. Read the directory again.
            time.sleep(0.002 * (attempt + 1))
        else:
            raise unavailable(current)
        result.extend(files)
        pending.extend(directories)
    # Path order - by normalized components - computed once per string rather
    # than by Path comparisons, which dominated listing a large Brain.
    result.sort(key=lambda item: os.path.normcase(item[0]).split(os.sep))
    return [(Path(text), image, text) for text, image in result]


# Record images this process has already validated: (archived, sha256 of the
# file text). validate_record depends on nothing but the record, and one prompt
# walks the Brain more than once (index, then the task lookup) - validating
# every record each time was a third of a warm prompt on a 500-record Brain.
_VALID_RECORD_IMAGES: set[tuple[bool, str]] = set()
_VALID_RECORD_LIMIT = 50000


def validate_record_image(record: dict[str, Any], text: str, *, archived: bool = False) -> None:
    """validate_record, remembered per exact file text within this process."""
    key = (archived, hashlib.sha256(text.encode("utf-8", "surrogatepass")).hexdigest())
    if key in _VALID_RECORD_IMAGES:
        return
    validate_record(record, archived=archived)
    if len(_VALID_RECORD_IMAGES) < _VALID_RECORD_LIMIT:
        _VALID_RECORD_IMAGES.add(key)


def _claimed_identity(text: Optional[str]) -> Optional[dict[str, Any]]:
    """The type/id/external_id a record that failed to parse still names."""
    if not isinstance(text, str) or not text.startswith("---\n"):
        return None
    try:
        raw = text[4:].split("\n---\n", 1)[0]
        metadata = json.loads(raw)
    except (ValueError, RecursionError):
        return None
    if not isinstance(metadata, dict):
        return None
    return {key: metadata.get(key) for key in ("type", "id", "external_id")}


def _record_problem(repository: Path, path: Path, metadata: Any, error: BrainError, *,
                    schema: bool = True) -> dict[str, Any]:
    """One skipped record: where, why, and the identity it claims if readable."""
    message = str(error)
    # Read errors already lead with the path; the report adds it once.
    if message.startswith(f"{path}: "):
        message = message[len(f"{path}: "):]
    if isinstance(metadata, dict):
        # The schema names the offending field; the lifecycle check alone may
        # not. A record that failed to parse keeps its parse error instead.
        if schema:
            try:
                validate_schema_file(repository, "dynamic-record.schema.json", metadata)
            except BrainError as detail:
                message = str(detail)
            except (OSError, ValueError, TypeError, KeyError, RecursionError):
                pass
        identity = tuple(metadata.get(key) if isinstance(metadata.get(key), str) else None
                         for key in ("type", "id", "external_id"))
    else:
        identity = None
    return {"path": path, "error": message, "identity": identity}


def iter_records(repository: Path, *, include_archive: bool = False,
                 problems: Optional[list[dict[str, Any]]] = None
                 ) -> Iterator[tuple[Path, dict[str, Any], str]]:
    """Every readable, valid governed record.

    A record that cannot be read or does not pass validation is skipped and,
    when `problems` is given, reported there with its cause and the identity it
    claims. One bad file - a half-merged conflict, a teammate's record from a
    newer schema, a hand edit - used to stop every task's memory, `get` of
    unrelated tasks and `validate`'s own report. Lookups by identifier still
    refuse rather than report absence when the one they ask for is the bad one
    (find_record), so a skipped record never turns into a duplicate.
    """
    for path, metadata, body, _ in iter_record_images(repository, include_archive=include_archive,
                                                       problems=problems):
        yield path, metadata, body


# Valid record images this process has read, by path: the file's identity as
# listed (device, inode, size, modification and change times), whether it lay
# in the archive, and the image. A pass over an unchanged Brain then reads no
# file again - one record-result made thirty passes over every record. Every
# write here replaces a record whole, so the new file never has the old one's
# identity, and an edit in place changes its times.
_RECORD_IMAGE_CACHE: dict[str, tuple[tuple[int, ...], bool, bytes, str, str]] = {}
_RECORD_IMAGE_CACHE_LIMIT = 20000


def _remember_record_image(path: str, image: os.stat_result, archived: bool,
                           metadata: dict[str, Any], body: str, text: str) -> None:
    try:
        packed = marshal.dumps(metadata)
    except ValueError:
        return
    if len(_RECORD_IMAGE_CACHE) >= _RECORD_IMAGE_CACHE_LIMIT:
        _RECORD_IMAGE_CACHE.clear()
    _RECORD_IMAGE_CACHE[path] = (_image_key(image), archived, packed, body, text)


def _image_key(image: os.stat_result) -> tuple[int, ...]:
    return (image.st_dev, image.st_ino, image.st_size, image.st_mtime_ns, image.st_ctime_ns)


# The cache above, carried from one process to the next: every prompt is a new
# process, and reading, parsing and validating every record again was most of
# a prompt on a large Brain. It is kept in the ignored project-brain/local/,
# for this project's records only, and tied to this module's file and the
# Python that wrote it - another validator or marshal format starts it over.
# A record whose times are within two seconds of the save stays out of it: a
# file system with a coarse clock could give a second edit in the same tick
# the first one's times. Anything unreadable is a cache miss, never an error.
_PERSISTED_IMAGES = "record-images.cache"
_PERSISTED_LIMIT = 64 * 1024 * 1024
_PERSISTED_SETTLE_NS = 2_000_000_000
_PERSISTED_STATE: dict[str, Any] = {"root": None, "dirty": False, "used": set(), "held": False}


def hold_persisted_images() -> None:
    """Read the carried cache but write none of it for the rest of this
    process: a run that promised to leave the filesystem as it found it."""
    _PERSISTED_STATE["held"] = True


def _persisted_version() -> tuple:
    source = os.stat(__file__)
    return (2, tuple(sys.version_info[:2]), source.st_size, source.st_mtime_ns)


def _persisted_entry(value: Any) -> Optional[tuple]:
    """A stored entry as the in-process cache holds it, or None. The body is
    stored as its length: it is the end of the record's text."""
    if not (isinstance(value, tuple) and len(value) == 5
            and isinstance(value[0], tuple) and len(value[0]) == 5
            and all(type(part) is int for part in value[0])
            and type(value[1]) is bool and type(value[2]) is bytes
            and type(value[3]) is int and type(value[4]) is str
            and 0 <= value[3] <= len(value[4])):
        return None
    text = value[4]
    return (value[0], value[1], value[2], text[len(text) - value[3]:], text)


def _load_persisted_images(repository: Path) -> None:
    root = str(brain_root(repository))
    if _PERSISTED_STATE["root"] == root:
        return
    _PERSISTED_STATE.update(root=root, dirty=False, used=set())
    path = os.path.join(root, "local", _PERSISTED_IMAGES)
    try:
        descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_BINARY", 0))
        with os.fdopen(descriptor, "rb") as handle:
            raw = handle.read(_PERSISTED_LIMIT + 1)
        if len(raw) > _PERSISTED_LIMIT:
            return
        version, entries = marshal.loads(raw)
        if version != _persisted_version() or not isinstance(entries, dict):
            return
        prefix = root + os.sep
        for key, value in entries.items():
            entry = _persisted_entry(value)
            if (entry is not None and isinstance(key, str) and key.startswith(prefix)
                    and key not in _RECORD_IMAGE_CACHE
                    and len(_RECORD_IMAGE_CACHE) < _RECORD_IMAGE_CACHE_LIMIT):
                _RECORD_IMAGE_CACHE[key] = entry
    except (OSError, ValueError, EOFError, TypeError):
        return


def _save_persisted_images() -> None:
    root = _PERSISTED_STATE["root"]
    if root is None or not _PERSISTED_STATE["dirty"] or _PERSISTED_STATE["held"]:
        return
    _PERSISTED_STATE["dirty"] = False
    local = os.path.join(root, "local")
    temporary = None
    try:
        if os.path.islink(local) or not os.path.isdir(local):
            return
        settled = time.time_ns() - _PERSISTED_SETTLE_NS
        prefix = root + os.sep
        entries = {}
        for key, value in list(_RECORD_IMAGE_CACHE.items()):
            if (not key.startswith(prefix) or max(value[0][3], value[0][4]) >= settled
                    or not value[4].endswith(value[3])):
                continue
            if key not in _PERSISTED_STATE["used"]:
                # Loaded but not looked at this time: kept while the file is
                # the one it was read from, dropped once it is gone.
                try:
                    if _image_key(os.lstat(key)) != value[0]:
                        continue
                except OSError:
                    continue
            entries[key] = (value[0], value[1], value[2], len(value[3]), value[4])
        if not entries and not os.path.lexists(os.path.join(local, _PERSISTED_IMAGES)):
            return
        raw = marshal.dumps((_persisted_version(), entries))
        if len(raw) > _PERSISTED_LIMIT:
            return
        descriptor, temporary = tempfile.mkstemp(prefix="." + _PERSISTED_IMAGES + ".", dir=local)
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(raw)
        os.replace(temporary, os.path.join(local, _PERSISTED_IMAGES))
        temporary = None
    except (OSError, ValueError, RuntimeError):
        return
    finally:
        if temporary is not None:
            with contextlib.suppress(OSError):
                os.unlink(temporary)


def iter_record_images(repository: Path, *, include_archive: bool = False,
                       problems: Optional[list[dict[str, Any]]] = None
                       ) -> Iterator[tuple[Path, dict[str, Any], str, str]]:
    """iter_records with the exact text each record was read and validated
    from, for a caller that hashes or indexes it: one read, one image."""
    if not _admit_brain_anchor(repository):
        return
    _load_persisted_images(repository)
    used = _PERSISTED_STATE["used"]
    roots = [(brain_root(repository) / "dynamic", False)]
    if include_archive:
        roots.append((brain_root(repository) / "archive", True))
    # Finish all active/archive directory scans before the first record is
    # consumed by lookup, duplicate admission or a mutation caller.
    paths = [(path, image, text, archived) for root, archived in roots
             for path, image, text in _record_inventory_stats(root)]
    archived_handoffs = brain_root(repository) / 'archive' / 'handoffs'

    def skip(path: Path, metadata: Any, error: BrainError, *, schema: bool = True) -> None:
        if problems is not None:
            problems.append(_record_problem(repository, path, metadata, error, schema=schema))

    for path, image, key_text, archived in paths:
        cached = _RECORD_IMAGE_CACHE.get(key_text)
        if cached is not None and cached[0] == _image_key(image) and cached[1] == archived:
            used.add(key_text)
            # A fresh copy each time: callers edit the record they are handed.
            yield path, marshal.loads(cached[2]), cached[3], cached[4]
            continue
        text = None
        try:
            text = read_record_text(path)
            metadata, body = parse_markdown_record(path, text=text)
        except BrainError as error:
            # Keep the identity it claims when the frontmatter still parses
            # (invalid Unicode in a title, say): a lookup for that task then
            # says the record is invalid instead of "not found".
            claimed = _claimed_identity(text)
            if claimed is None and is_uuid4(path.stem):
                # Unreadable bytes name nothing, but the file is named by the
                # record's UUID: a lookup must say "invalid", not "not found".
                claimed = {"type": None, "id": path.stem, "external_id": None}
            skip(path, claimed, error, schema=False)
            continue
        if metadata.get("type") == "handoff":
            # Whatever identity the file carries stays claimed, so no lookup
            # reads its absence as room for a new record.
            claimed = {"type": None, "id": metadata.get("id"),
                       "external_id": metadata.get("external_id") or metadata.get("task_id")}
            if path.parent == archived_handoffs and path.stem == metadata.get('task_id'):
                try:
                    validate_handoff(metadata)
                    if metadata['status'] != 'closed':
                        raise BrainError('archived handoff must be closed')
                except (BrainError, KeyError, TypeError) as error:
                    skip(path, claimed, BrainError(f'archived handoff invalid: {error}'), schema=False)
                continue
            skip(path, claimed, BrainError("unexpected handoff in authority path"), schema=False)
            continue
        if (metadata.get('type') not in RECORD_TYPES
                or not is_uuid4(metadata.get('id'))
                or not isinstance(metadata.get('external_id'), str)
                or not metadata['external_id'].strip()):
            skip(path, metadata, BrainError("record identity unavailable"))
            continue
        try:
            validate_record_image(metadata, text, archived=archived)
        except BrainError as error:
            skip(path, metadata, error)
            continue
        _remember_record_image(key_text, image, archived, metadata, body, text)
        used.add(key_text)
        _PERSISTED_STATE["dirty"] = True
        yield path, metadata, body, text
    # A whole pass that read anything leaves the cache for the next process.
    _save_persisted_images()


def find_record(
    repository: Path,
    identifier: str,
    *,
    record_type: Optional[str] = None,
    include_archive: bool = False,
) -> tuple[Path, dict[str, Any], str]:
    matches = []
    problems: list[dict[str, Any]] = []
    for path, record, body in iter_records(repository, include_archive=include_archive,
                                           problems=problems):
        if (
            (record_type is None or record.get("type") == record_type)
            and identifier in {record.get("id"), record.get("external_id")}
        ):
            matches.append((path, record, body))
    # A skipped record that claims this identity is not absence: answering
    # "not found" would let the caller create a duplicate beside it.
    claimants = [problem for problem in problems
                 if problem["identity"] is not None and identifier in problem["identity"][1:]
                 and (record_type is None or problem["identity"][0] in (None, record_type))]
    if len(matches) + len(claimants) > 1:
        raise BrainError(f"Project Brain record identifier is ambiguous: {identifier}")
    if claimants:
        # Repository-relative: the message reaches prompts and logs, and the
        # absolute path names the machine's own directories.
        try:
            shown = claimants[0]['path'].relative_to(repository).as_posix()
        except ValueError:
            shown = claimants[0]['path'].name
        raise BrainError(f"Project Brain record invalid: {shown}: {claimants[0]['error']}")
    if not matches:
        raise BrainRecordNotFound(identifier, record_type)
    return matches[0]


def find_task(repository: Path, identifier: str) -> tuple[Path, dict[str, Any], str]:
    return find_record(repository, identifier, record_type="task")


def _record_body(record: dict[str, Any]) -> str:
    """Render the human-readable half of a record.

    ``## Sources`` is rendered for the same reason ``## Files`` is: the index
    reads bodies, not frontmatter, so a finding created with
    ``--source app/Billing/InvoiceTotal.php`` was previously unreachable by
    that path — the one string a reader is most likely to search for. Nothing
    verifies an on-disk body against this function (``validate_repository``
    checks frontmatter only) and ``document_metadata.source_hash`` is
    recomputed from the file at index time, so existing records keep their
    current body until their next mutation rewrites them.
    """
    next_steps = "\n".join(f"- {item}" for item in record["next_steps"]) or "- None"
    files = "\n".join(f"- `{item}`" for item in record["files"]) or "- None"
    sources = "\n".join(f"- `{item}`" for item in record.get("sources") or []) or "- None"
    return (
        f"# {record['title']}\n\n"
        f"## Goal\n{record['goal']}\n\n"
        f"## Progress\n{record['progress'] or 'Not started.'}\n\n"
        f"## Next Steps\n{next_steps}\n\n"
        f"## Files\n{files}\n\n"
        f"## Sources\n{sources}\n"
    )


def render_current_state(
    progress: Optional[str], auto_checkpoint: Optional[str]
) -> str:
    """Combine manual progress with the automatic turn checkpoint for display.

    Manual progress always leads: it is the operator's own account and
    automation must never displace it. The checkpoint follows under an explicit
    ``Since checkpoint:`` label so a reader can tell testimony from telemetry.
    Without manual progress the checkpoint stands alone — its own
    ``Auto-checkpoint:`` prefix already states its provenance, so it is not
    dressed up as handwritten progress. Without a checkpoint the rendering is
    byte-identical to what it was before the field existed.
    """
    progress = progress or ""
    auto_checkpoint = auto_checkpoint or ""
    if progress and auto_checkpoint:
        return f"{progress}\nSince checkpoint: {auto_checkpoint}"
    return progress or auto_checkpoint


def _handoff_for(task: dict[str, Any], existing: Optional[dict[str, Any]] = None) -> dict[str, Any]:
    timestamp = utc_now()
    return {
        "schema_version": SCHEMA_VERSION,
        "id": existing["id"] if existing else new_uuid(),
        "type": "handoff",
        "task_id": task["id"],
        "task_revision": task["revision"],
        "revision": existing["revision"] + 1 if existing else 1,
        "owner": task["owner"],
        "privacy": task["privacy"],
        "created_at": existing["created_at"] if existing else timestamp,
        "updated_at": timestamp,
        "objective": task["goal"],
        "current_state": render_current_state(
            task["progress"], task.get("auto_checkpoint")
        ),
        "next_actions": task["next_steps"],
        "files": task["files"],
        "sources": task["sources"],
        "status": "closed" if task["status"] in TERMINAL_STATES else "active",
        **({"phase": task["phase"]} if "phase" in task else {}),
    }


def _handoff_body(handoff: dict[str, Any]) -> str:
    actions = "\n".join(f"- {item}" for item in handoff["next_actions"]) or "- None"
    phase = handoff.get("phase")
    return (
        f"# Handoff for {handoff['task_id']}\n\n"
        f"## Phase\n{phase or 'Not declared.'}\n\n"
        f"## Objective\n{handoff['objective']}\n\n"
        f"## Current State\n{handoff['current_state'] or 'Not started.'}\n\n"
        f"## Next Actions\n{actions}\n"
    )


def guard_shared_text(label: str, values: Any) -> None:
    """Refuse a Project Brain write that would store a secret or personal data.

    Records, handoffs and agent messages are Git-tracked and repeated into
    Task Capsules, so the governed path holds the line the lightweight path
    and the capsule gate already hold. Only the text the call introduces is
    checked: a record written before this guard existed stays updatable. The
    message names the kind of data, never the value.
    """
    text = "\n".join(value for value in values if isinstance(value, str) and value)
    found = sensitive_label(text)
    if found:
        raise BrainError(
            f"{label} contains {found}; replace it with a sanitized summary "
            "- nothing was stored"
        )


def stored_secret_label(record: dict[str, Any]) -> Optional[str]:
    """The kind of credential a stored record's free text carries, or None.

    Validation reports secrets only. A record written before the write guard
    may hold personal data, which promotion and the capsule already refuse to
    spread; failing every validation over it would leave the operator a
    repository they cannot get green without rewriting history.
    """
    fields = [
        record.get("title"), record.get("goal"), record.get("progress"),
        record.get("auto_checkpoint"), *record.get("next_steps", []),
        *(item.get("reason") for item in record.get("transitions", []) if isinstance(item, dict)),
    ]
    text = "\n".join(value for value in fields if isinstance(value, str) and value)
    for label, pattern in BANK_SECRET_PATTERNS.items():
        if pattern.search(text):
            return f"a possible {label}"
    return None


def _write_record(repository: Path, record: dict[str, Any], path: Path) -> None:
    validate_record(record)
    affected = [path, *index_paths(repository)]
    current_handoff_path = handoff_path(repository, record["id"])
    handoff: Optional[dict[str, Any]] = None
    if record["type"] == "task":
        affected.append(current_handoff_path)
        existing = None
        if current_handoff_path.is_file():
            existing, _ = parse_markdown_record(current_handoff_path)
        handoff = _handoff_for(record, existing)
        validate_handoff(handoff, record)
    snapshot = snapshot_files(affected)
    try:
        atomic_write(path, render_markdown_record(record, _record_body(record)))
        if handoff is not None:
            atomic_write(
                current_handoff_path,
                render_markdown_record(handoff, _handoff_body(handoff)),
            )
        rebuild_indexes(repository)
    except Exception:
        restore_files(snapshot)
        raise


def create_record(
    repository: Path,
    record_type: str,
    external_id: str,
    title: str,
    files: list[str],
    sources: list[str],
    *,
    owner: str,
    privacy: str = "team",
    authority: str = "observed",
    confidence: float = 1.0,
    goal: Optional[str] = None,
    conflicts: Optional[list[str]] = None,
) -> dict[str, Any]:
    if record_type not in RECORD_TYPES:
        raise BrainError(f"type must be one of: {', '.join(RECORD_TYPES)}")
    guard_shared_text(
        f"{record_type.capitalize()} record", [title, goal, *(conflicts or [])]
    )
    with mutation_lock(repository):
        # Alias lookup and creation must share the same repository lock.
        try:
            find_record(repository, external_id)
        except BrainRecordNotFound as error:
            if error.identifier != external_id or error.record_type is not None:
                raise
        else:
            raise BrainError(f"Project Brain record already exists: {external_id}")
        timestamp = utc_now()
        initial = LIFECYCLES[record_type]["initial"]
        record: dict[str, Any] = {
            "schema_version": SCHEMA_VERSION,
            "id": new_uuid(),
            "type": record_type,
            "external_id": external_id,
            "title": title,
            "status": initial,
            "revision": 1,
            "owner": owner,
            "authorized_owners": sorted(set([owner])),
            "privacy": privacy,
            "authority": authority,
            "confidence": confidence,
            "created_at": timestamp,
            "updated_at": timestamp,
            "transitions": [
                {
                    "from": None,
                    "to": initial,
                    "at": timestamp,
                    "actor": owner,
                    "reason": f"{record_type.capitalize()} created",
                }
            ],
            "source_fingerprints": source_fingerprints(repository, sources),
            "conflicts": sorted(set(conflicts or [])),
            "supersedes": [],
            "superseded_by": None,
            "goal": goal or title,
            "progress": "",
            "next_steps": [],
            "files": sorted(set(files)),
            "sources": sorted(set(sources)),
        }
        path = dynamic_path(repository, record)
        if path.exists():
            raise BrainError(f"Project Brain UUID collision: {record['id']}")
        _write_record(repository, record, path)
    return record


def create_task(
    repository: Path,
    external_id: str,
    goal: str,
    files: list[str],
    sources: list[str],
    *,
    owner: str,
    privacy: str = "team",
) -> dict[str, Any]:
    return create_record(
        repository,
        "task",
        external_id,
        goal,
        files,
        sources,
        owner=owner,
        privacy=privacy,
        goal=goal,
    )


# Who checked a verified claim: the agent that wrote it, or a person.
ATTESTATIONS = ("agent", "person")
ATTESTATION_MARK = re.compile(r"\[attestation:(agent|person)\]")


def record_attestation(record: dict[str, Any]) -> str:
    """Who checked the claim the record's latest verification hardened.

    'agent' or 'person' from the verification's ledger entry; '' when it
    does not say. Unattended saves made before the mark existed said
    "agent-attested" in their reason, and still count as the agent's.
    """
    for transition in reversed(record.get("transitions") or []):
        if transition.get("to") != "verified" or transition.get("from") not in AUTHORITY_TRANSITIONS:
            continue
        reason = str(transition.get("reason") or "")
        marks = ATTESTATION_MARK.findall(reason)
        if marks:
            # Supported writes append the explicit mark after free-form
            # prose, which may itself quote an older attestation marker.
            return marks[-1]
        return "agent" if "agent-attested" in reason else ""
    return ""

def update_record(
    repository: Path,
    identifier: str,
    *,
    expected_revision: Optional[int],
    progress: Optional[str],
    next_steps: list[str],
    files: list[str],
    sources: list[str],
    actor: str,
    conflicts: Optional[list[str]] = None,
    transition_to: Optional[str] = None,
    phase: Optional[str] = None,
    authority: Optional[str] = None,
    auto_checkpoint: Optional[str] = None,
    reason: str = "Task updated",
    attestation: Optional[str] = None,
    reverify: bool = False,
    allow_phase_regression: bool = False,
    replace_next_steps: bool = False,
) -> dict[str, Any]:
    """Mutate a record under the caller's compare-and-swap revision.

    ``authority`` promotes the record's evidence level. Only the
    ``observed`` -> ``verified`` edge is legal — verification is the one act
    that hardens a claim, and nothing may downgrade or fabricate that step —
    so any other requested pair is refused before the record changes. The
    promotion is appended to the same append-only ``transitions`` ledger as a
    lifecycle change (actor, reason, from/to authority), ahead of any status
    transition requested by the same call, and runs under the same
    ``expected_revision`` compare-and-swap as every other field.

    ``auto_checkpoint`` is the automated turn flush's own field. Replacing it
    is the point — each flush supersedes the previous checkpoint — while
    ``progress`` stays reserved for the operator's narrative, so an automated
    caller passes ``progress=None`` and its checkpoint here.

    ``attestation`` says who checked the claim a verification hardens:
    ``agent`` when the agent that wrote it attested it and no person read it,
    ``person`` when someone did. It is written into the verification's own
    ledger entry (`record_attestation` reads it back), not into a new record
    field, so a runtime that predates it still accepts the record; promotion
    carries an agent's attestation into the chunk's tags.

    ``reverify`` says the update restates the claim of a record that is
    already verified - a revised consequence - and ``attestation`` who
    checked the new claim. The ledger gains another ``observed`` ->
    ``verified`` entry: the revision is the observation, the attestation its
    check, and it is the one authority edge every runtime's validator
    accepts. Without it a person's correction kept the agent's mark from the
    first verification, and the next agent was free to overwrite it.

    ``next_steps`` are appended unless ``replace_next_steps`` makes them the
    whole list: without it a step that was done could never leave, and the
    handoff kept presenting finished work as what comes next. ``files`` keep
    the newest touch last, which is the end the capsule projects from.
    """
    guard_shared_text(
        "Record update",
        [progress, auto_checkpoint, reason, *next_steps, *(conflicts or [])],
    )
    with mutation_lock(repository):
        path, record, _ = find_record(repository, identifier)
        validate_record(record)
        if expected_revision is not None and record["revision"] != expected_revision:
            raise BrainError(
                f"Stale {record['type']} revision: expected {expected_revision}, "
                f"current {record['revision']}"
            )
        if actor not in record["authorized_owners"]:
            raise BrainError(f"Owner is not authorized to mutate record: {actor}")
        if record["type"] == "event" and (
            progress is not None or next_steps or files or sources
            or authority is not None or replace_next_steps
        ):
            raise BrainError("Events are immutable; only lifecycle supersession is allowed")
        if auto_checkpoint is not None and record["type"] != "task":
            raise BrainError("only a task may record an auto checkpoint")
        if attestation is not None and attestation not in ATTESTATIONS:
            raise BrainError("attestation must be one of: " + ", ".join(ATTESTATIONS))
        if authority is not None:
            if authority not in AUTHORITY_TRANSITIONS.get(record["authority"], set()):
                raise BrainError(
                    f"Illegal authority transition: {record['authority']} -> "
                    f"{authority}; only observed -> verified is allowed"
                )
            record["transitions"].append(
                {
                    "from": record["authority"], "to": authority, "at": utc_now(),
                    "actor": actor,
                    "reason": reason + (
                        f" [attestation:{attestation}]" if attestation is not None else ""
                    ),
                }
            )
            record["authority"] = authority
        if reverify:
            if authority is not None or record["authority"] != "verified":
                raise BrainError("Only a verified record's revised claim is verified again")
            if attestation is None:
                raise BrainError("Say who checked the revised claim: attestation agent or person")
            record["transitions"].append(
                {
                    "from": "observed", "to": "verified", "at": utc_now(),
                    "actor": actor, "reason": f"{reason} [attestation:{attestation}]",
                }
            )
        if transition_to is not None:
            allowed = LIFECYCLES[record["type"]]["transitions"].get(
                record["status"], set()
            )
            if transition_to not in allowed:
                raise BrainError(
                    f"Illegal {record['type']} transition: "
                    f"{record['status']} -> {transition_to}"
                )
            timestamp = utc_now()
            record["transitions"].append(
                {
                    "from": record["status"], "to": transition_to, "at": timestamp,
                    "actor": actor, "reason": reason,
                }
            )
            record["status"] = transition_to
        if phase is not None:
            if record["type"] != "task":
                raise BrainError("only a task may declare a phase")
            new_phase = normalize_phase(phase)
            current_phase = record.get("phase")
            if (
                not allow_phase_regression
                and current_phase in TASK_PHASES
                and TASK_PHASES.index(new_phase) < TASK_PHASES.index(current_phase)
            ):
                raise BrainError(
                    f"Phase may not move backward: {current_phase} -> {new_phase};"
                    " pass allow_phase_regression to state the regression is"
                    " deliberate"
                )
            record["phase"] = new_phase
        if progress is not None:
            record["progress"] = progress
        if auto_checkpoint is not None:
            record["auto_checkpoint"] = auto_checkpoint
        record["next_steps"] = list(
            dict.fromkeys(
                next_steps if replace_next_steps else [*record["next_steps"], *next_steps]
            )
        )
        touched = list(dict.fromkeys(files))
        retouched = set(touched)
        record["files"] = [
            existing for existing in record["files"] if existing not in retouched
        ] + touched
        record["sources"] = list(dict.fromkeys([*record["sources"], *sources]))
        if conflicts is not None:
            record["conflicts"] = list(
                dict.fromkeys([*record["conflicts"], *conflicts])
            )
        record["source_fingerprints"] = source_fingerprints(repository, record["sources"])
        record["revision"] += 1
        record["updated_at"] = utc_now()
        _write_record(repository, record, path)
        return record


def update_task(
    repository: Path,
    identifier: str,
    **changes: Any,
) -> dict[str, Any]:
    _, record, _ = find_task(repository, identifier)
    if record["type"] != "task":
        raise BrainError("update_task requires a task record")
    return update_record(repository, identifier, **changes)


def get_record(repository: Path, identifier: str) -> dict[str, Any]:
    _, record, _ = find_record(repository, identifier)
    validate_record(record)
    return record


def get_task(repository: Path, identifier: str) -> dict[str, Any]:
    _, record, _ = find_task(repository, identifier)
    validate_record(record)
    return record


def snapshot_record_state(repository: Path, identifier: str) -> FileSnapshot:
    path, record, _ = find_record(repository, identifier)
    paths = [path, *index_paths(repository)]
    if record["type"] == "task":
        paths.append(handoff_path(repository, record["id"]))
    return snapshot_files(paths)


def restore_record_state(repository: Path, snapshot: FileSnapshot) -> None:
    with mutation_lock(repository):
        restore_files(snapshot)


def rollback_created_record(repository: Path, identifier: str) -> None:
    with mutation_lock(repository):
        path, record, _ = find_record(repository, identifier)
        paths = [path]
        if record["type"] == "task":
            paths.append(handoff_path(repository, record["id"]))
        for item in paths:
            if item.exists():
                item.unlink()
        rebuild_indexes(repository)


def close_task(
    repository: Path,
    identifier: str,
    outcome: str,
    verification: list[str],
    *,
    expected_revision: Optional[int],
    actor: str,
) -> dict[str, Any]:
    progress = outcome
    if verification:
        progress += "\nVerification: " + "; ".join(verification)
    return update_task(
        repository,
        identifier,
        expected_revision=expected_revision,
        progress=progress,
        next_steps=[],
        files=[],
        sources=[],
        actor=actor,
        transition_to="completed",
        reason="Task completed after verification",
    )


def cancel_task(repository: Path, identifier: str, *, actor: str) -> dict[str, Any]:
    return update_task(
        repository,
        identifier,
        expected_revision=None,
        progress=None,
        next_steps=[],
        files=[],
        sources=[],
        actor=actor,
        transition_to="cancelled",
        reason="Task abandoned",
    )


def messages_path(repository: Path, task_uuid: str) -> Path:
    return brain_root(repository) / "control" / "messages" / f"{task_uuid}.jsonl"


def validate_actor(value: str, *, allow_broadcast: bool = False) -> str:
    actor = (value or "").strip()
    if allow_broadcast and actor == "*":
        return actor
    if ACTOR_PATTERN.fullmatch(actor) is None:
        raise BrainError(
            "Actor must be a lowercase slug (letters, digits, '-', '_') up to"
            " 64 characters"
        )
    return actor


def validate_message(record: dict[str, Any]) -> None:
    """Validate one channel entry with in-code rules.

    The JSON schema file is defense for the governed monorepo; a copied-out
    edition without ``project-brain/schemas`` still enforces the same
    contract through this function.
    """
    if record.get("schema_version") != SCHEMA_VERSION:
        raise BrainError(f"message schema_version must be {SCHEMA_VERSION}")
    if not is_uuid4(record.get("id")):
        raise BrainError("message id must be UUIDv4")
    if not is_uuid4(record.get("task_id")):
        raise BrainError("message task_id must be UUIDv4")
    for key in ("seq", "task_revision"):
        if not isinstance(record.get(key), int) or record[key] < 1:
            raise BrainError(f"message {key} must be a positive integer")
    _require_datetime(record, "created_at")
    validate_actor(str(record.get("from_actor", "")))
    validate_actor(str(record.get("to_actor", "")), allow_broadcast=True)
    if record.get("type") not in MESSAGE_TYPES:
        raise BrainError(
            "message type must be one of: " + ", ".join(MESSAGE_TYPES)
        )
    body = record.get("body")
    if not isinstance(body, str) or not body.strip():
        raise BrainError("message body must not be empty")
    if len(body) > MESSAGE_BODY_LIMIT:
        raise BrainError(
            f"message body exceeds {MESSAGE_BODY_LIMIT} characters"
        )
    refs = record.get("refs")
    if not isinstance(refs, list) or any(
        not isinstance(item, str) or not item.strip() for item in refs
    ):
        raise BrainError("message refs must be a list of non-empty strings")
    digest = record.get("capsule_digest")
    if digest is not None and (
        not isinstance(digest, str)
        or re.fullmatch(r"[0-9a-f]{64}", digest) is None
    ):
        raise BrainError("message capsule_digest must be a SHA-256 hex digest")


def append_message(
    repository: Path,
    identifier: str,
    *,
    from_actor: str,
    to_actor: str,
    message_type: str,
    body: str,
    refs: Optional[list[str]] = None,
    capsule_digest: Optional[str] = None,
) -> dict[str, Any]:
    """Append one entry to the task's agent channel.

    The journal is append-only under the global mutation lock: entries are
    never rewritten, so the git history of the file is the audit trail. A
    terminal task refuses new messages - the channel exists for active
    coordination, not for post-mortem edits.
    """
    guard_shared_text("Agent message", [body])
    with mutation_lock(repository):
        _, task, _ = find_task(repository, identifier)
        if task["status"] in LIFECYCLES["task"]["terminal"]:
            raise BrainError(
                "Messages require an active task; this task is terminal"
            )
        path = messages_path(repository, task["id"])
        seq = 1
        if path.is_file():
            with path.open("r", encoding="utf-8") as handle:
                seq += sum(1 for line in handle if line.strip())
        record: dict[str, Any] = {
            "schema_version": SCHEMA_VERSION,
            "id": new_uuid(),
            "seq": seq,
            "task_id": task["id"],
            "task_revision": task["revision"],
            "created_at": utc_now(),
            "from_actor": validate_actor(from_actor),
            "to_actor": validate_actor(to_actor, allow_broadcast=True),
            "type": message_type,
            "body": body,
            "refs": list(refs or []),
        }
        if capsule_digest is not None:
            record["capsule_digest"] = capsule_digest
        validate_message(record)
        validate_schema_file(repository, "message.schema.json", record)
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("a", encoding="utf-8", newline="\n") as handle:
            handle.write(json.dumps(record, ensure_ascii=False, sort_keys=True) + "\n")
            handle.flush()
            os.fsync(handle.fileno())
        return record


def read_messages(
    repository: Path,
    identifier: str,
    *,
    for_actor: Optional[str] = None,
    since_seq: int = 0,
    message_type: Optional[str] = None,
) -> list[dict[str, Any]]:
    """Return channel entries, filtered for a recipient.

    ``for_actor`` selects entries addressed to that actor or broadcast
    (``*``); ``since_seq`` skips entries the caller has already consumed -
    the journal keeps no read cursor, consumers track their own position.
    Terminal tasks stay readable: the audit trail outlives the work.
    """
    _, task, _ = find_record(
        repository, identifier, record_type="task", include_archive=True
    )
    path = messages_path(repository, task["id"])
    if not path.is_file():
        return []
    recipient = validate_actor(for_actor) if for_actor is not None else None
    selected: list[dict[str, Any]] = []
    for number, line in enumerate(
        path.read_text(encoding="utf-8").splitlines(), 1
    ):
        if not line.strip():
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError as error:
            raise BrainError(
                f"Corrupt message journal line {number}: {path}"
            ) from error
        validate_message(record)
        if record["seq"] <= since_seq:
            continue
        if message_type is not None and record["type"] != message_type:
            continue
        if recipient is not None and record["to_actor"] not in {recipient, "*"}:
            continue
        selected.append(record)
    return selected


def record_is_eligible(repository: Path, record: dict[str, Any], config: dict[str, Any]) -> tuple[bool, str]:
    """Whether retrieval may index a record, and the reason if not.

    A record whose cited file was edited is eligible (retrieval marks it); one
    whose cited file is gone (`source-missing`) or was never digested
    (`source-undigested`) is not.
    """
    if record.get("privacy") == "private":
        return False, "private"
    if record.get("privacy") not in config["allowed_privacy"]:
        return False, "privacy"
    owners = set(config.get("owners", ["*"]))
    if record.get("privacy") == "restricted" and "*" not in owners and record.get("owner") not in owners:
        return False, "owner"
    if record.get("authority") not in config["allowed_authority"]:
        return False, "authority"
    record_type = record.get("type")
    if (
        record_type not in LIFECYCLES
        or record.get("status") in LIFECYCLES[record_type]["terminal"]
    ):
        return False, "lifecycle"
    if record.get("superseded_by") is not None:
        return False, "superseded"
    # An edited citation no longer evicts the record: retrieval marks it and
    # ranks it down (context_retrieval._runtime_filter), as it does a chunk.
    # Any edit used to evict it - accepted decisions, open findings and
    # events alike - and its links went with it. Only a citation that cannot
    # be checked at all keeps the record out.
    problem = citation_problem(repository, record)
    if problem is not None:
        return False, problem
    return True, "eligible"


def _relative_posix(path: Path, repository: Path) -> str:
    """path.relative_to(repository).as_posix() for a path known to lie under
    it, by slicing the string: pathlib's version was most of an index rebuild
    on a large Brain."""
    text, base = str(path), str(repository)
    if base in ("", os.curdir) and not os.path.isabs(text):
        relative = text
    elif text.startswith(base) and text[len(base):len(base) + 1] == os.sep and not base.endswith(os.sep):
        relative = text[len(base) + 1:]
    else:
        return path.relative_to(repository).as_posix()
    return relative.replace(os.sep, "/") if os.sep != "/" else relative


def build_index_data(
    repository: Path,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    active: list[dict[str, Any]] = []
    archived: list[dict[str, Any]] = []
    for path, record, _ in iter_records(repository, include_archive=True):
        relative = _relative_posix(path, repository)
        item = {
            "id": record.get("id"), "type": record.get("type"), "status": record.get("status"),
            "revision": record.get("revision"), "path": relative,
        }
        if "archive" in relative.split("/"):
            archived.append(item)
        else:
            active.append(item)
    key = lambda item: (str(item["type"]), str(item["id"]), str(item["path"]))
    return sorted(active, key=key), sorted(archived, key=key)


def rebuild_indexes(repository: Path) -> None:
    active, archived = build_index_data(repository)
    active_path, archive_path = index_paths(repository)
    atomic_json(active_path, active)
    atomic_json(archive_path, archived)


def stale_record_states(repository: Path) -> list[tuple[str, str]]:
    """Records whose cited sources are no longer what they were when written,
    each with its state: `changed` (retrieval serves it marked for checking),
    `source-missing` or `source-undigested` (retrieval leaves it out).
    Archived records are listed too, though they are never retrieved."""
    stale: list[tuple[str, str]] = []
    for path, record, _ in iter_records(repository, include_archive=True):
        try:
            fresh = sources_are_fresh(repository, record)
        except (BrainError, OSError):
            continue
        if not fresh:
            stale.append((str(path), citation_problem(repository, record) or "changed"))
    return stale


def stale_records(repository: Path) -> list[str]:
    """Records whose cited source changed after they were written.

    Reported beside validation, not as a failure of it: an edited citation
    leaves the record intact and retrieval serves it marked for checking; a
    deleted or undigested one keeps it out of retrieval.
    """
    return [path for path, _ in stale_record_states(repository)]


def validate_repository(
    repository: Path, *, check_freshness: bool = True
) -> list[str]:
    """Every problem with the governed records, journals, manifests and indexes.

    ``check_freshness=False`` leaves out one rule: a record whose cited source
    changed after it was written. That is the source moving on, not the record
    breaking - retrieval serves such a record marked for checking, and only a
    deleted citation keeps it out - and no write can cause or cure it.
    Compaction passes it: with the rule in, the first edit to any file a record
    cites, such as the living spec behind an accepted decision, refused every
    compaction from then on.
    """
    errors: list[str] = []
    seen: set[str] = set()
    records: dict[str, tuple[Path, dict[str, Any]]] = {}
    problems: list[dict[str, Any]] = []
    for path, record, _ in iter_records(repository, include_archive=True, problems=problems):
        try:
            archived = "archive" in path.parts
            validate_schema_file(
                repository, "dynamic-record.schema.json", record
            )
            validate_record(record, archived=archived)
            if record["id"] in seen:
                raise BrainError(f"duplicate UUID: {record['id']}")
            seen.add(record["id"])
            expected_path = (
                brain_root(repository) / "archive" / record["type"] / f"{record['id']}.md"
                if archived
                else dynamic_path(repository, record)
            )
            if path != expected_path:
                raise BrainError(
                    f"record path must be {expected_path.relative_to(repository)}"
                )
            if check_freshness and not sources_are_fresh(repository, record):
                raise BrainError("source fingerprint is stale")
            secret = stored_secret_label(record)
            if secret is not None:
                raise BrainError(
                    f"record contains {secret}; remove it from the record and "
                    "rotate the credential, since Git history keeps it"
                )
            records[record["id"]] = (path, record)
        except BrainError as error:
            errors.append(f"{path}: {error}")
    # Records the inventory skipped are reported here by name and cause; a
    # skipped one still counts as present for the relationship checks below.
    errors.extend(f"{problem['path']}: {problem['error']}" for problem in problems)
    present = set(records) | {problem["identity"][1] for problem in problems
                              if problem["identity"] is not None and problem["identity"][1]}
    for record_id, (path, record) in records.items():
        for relationship in [*record["conflicts"], *record["supersedes"]]:
            if relationship not in present:
                errors.append(f"{path}: relationship points to missing {relationship}")
        replacement = record["superseded_by"]
        if replacement is not None and replacement not in present:
            errors.append(f"{path}: superseded_by points to missing {replacement}")
        if replacement in records and record_id not in records[replacement][1]["supersedes"]:
            errors.append(f"{path}: supersession backlink is missing from {replacement}")
    handoff_root = _require_utf8_record_path(brain_root(repository) / "control" / "handoffs")
    if handoff_root.is_dir():
        for path in [_require_utf8_record_path(p) for p in sorted(handoff_root.glob("*.md"))]:
            try:
                handoff, _ = parse_markdown_record(path)
                validate_schema_file(repository, "handoff.schema.json", handoff)
                task = get_task(repository, handoff["task_id"])
                validate_handoff(handoff, task)
            except (BrainError, KeyError) as error:
                errors.append(f"{path}: {error}")
    archived_handoffs = _require_utf8_record_path(brain_root(repository) / "archive" / "handoffs")
    if archived_handoffs.is_dir():
        for path in [_require_utf8_record_path(p) for p in sorted(archived_handoffs.glob("*.md"))]:
            try:
                handoff, _ = parse_markdown_record(path)
                validate_schema_file(repository, "handoff.schema.json", handoff)
                _, task, _ = find_record(
                    repository,
                    handoff["task_id"],
                    record_type="task",
                    include_archive=True,
                )
                validate_handoff(handoff, task)
                if handoff["status"] != "closed":
                    raise BrainError("archived handoff must be closed")
            except (BrainError, KeyError) as error:
                errors.append(f"{path}: {error}")
    messages_root = brain_root(repository) / "control" / "messages"
    if messages_root.is_dir():
        for path in sorted(messages_root.glob("*.jsonl")):
            try:
                if not is_uuid4(path.stem):
                    raise BrainError("message journal name must be a task UUID")
                find_record(
                    repository, path.stem, record_type="task", include_archive=True
                )
                previous_seq = 0
                for number, line in enumerate(
                    path.read_text(encoding="utf-8").splitlines(), 1
                ):
                    if not line.strip():
                        raise BrainError(
                            f"line {number}: blank line in message journal"
                        )
                    entry = json.loads(line)
                    validate_message(entry)
                    validate_schema_file(
                        repository, "message.schema.json", entry
                    )
                    if entry["task_id"] != path.stem:
                        raise BrainError(
                            f"line {number}: task_id does not match the journal"
                        )
                    if entry["seq"] != previous_seq + 1:
                        raise BrainError(
                            f"line {number}: seq must increase by exactly one"
                        )
                    previous_seq = entry["seq"]
            except (OSError, json.JSONDecodeError, BrainError, KeyError) as error:
                errors.append(f"{path}: {error}")
    # Keyed off the manifest's own declared version rather than one frozen
    # set: version 2 adds query provenance and the gate verdict, version 3 adds
    # host provenance, and older manifests must keep validating instead of
    # turning a consuming project's Definition of Done red after an upgrade.
    #
    # The gate joined version 2 rather than opening a version 3 because
    # version 2 has not been released: it and the gate land in the same
    # unreleased change, and no manifest anywhere was ever written to the
    # intermediate shape. Requiredness lives here and not in the JSON schema
    # because `validate_schema_value` has no conditional construct, so a
    # schema `required` entry would reject a version 1 manifest too.
    version_1_keys = {
        "schema_version", "id", "created_at", "query", "task_id", "task_revision",
        "filters", "selected", "excluded", "token_estimates", "provider",
        "escalation_reason",
    }
    manifest_keys_by_version = {
        1: version_1_keys,
        2: version_1_keys | {"query_source", "phase_seconds", "gate"},
        3: version_1_keys
        | {
            "query_source", "phase_seconds", "gate", "host", "entry_point",
            "local_episode_count",
        },
    }
    # Version 4 adds the counters of automatic source-linked expansion.
    # Version 3 shipped, so a version 3 manifest that carried them would be
    # rejected by every runtime that validates version 3 exactly.
    manifest_keys_by_version[4] = manifest_keys_by_version[3] | {"source_links"}
    version_1_token_keys = {
        "policy", "handoff", "durable", "dynamic", "evidence", "total",
        "target", "hard",
    }
    manifest_token_keys_by_version = {
        1: version_1_token_keys,
        2: version_1_token_keys,
        3: version_1_token_keys | {"local_episodes"},
        4: version_1_token_keys | {"local_episodes"},
    }
    manifests = brain_root(repository) / "control" / "retrieval-manifests"
    if manifests.is_dir():
        for path in sorted(manifests.glob("*.json")):
            try:
                manifest = json.loads(path.read_text(encoding="utf-8"))
                validate_schema_file(
                    repository, "retrieval-manifest.schema.json", manifest
                )
                expected_keys = manifest_keys_by_version.get(
                    manifest.get("schema_version") if isinstance(manifest, dict) else None
                )
                if (
                    not isinstance(manifest, dict)
                    or expected_keys is None
                    or set(manifest) != expected_keys
                ):
                    raise BrainError("retrieval manifest does not match strict schema")
                if not is_uuid4(manifest["id"]) or not is_uuid4(manifest["task_id"]):
                    raise BrainError("retrieval manifest IDs must be UUIDv4")
                if manifest["id"] != path.stem:
                    raise BrainError("retrieval manifest ID does not match filename")
                estimates = manifest["token_estimates"]
                expected_token_keys = manifest_token_keys_by_version[manifest["schema_version"]]
                if not isinstance(estimates, dict) or set(estimates) != expected_token_keys:
                    raise BrainError(
                        "retrieval manifest token estimates do not match strict schema"
                    )
                if estimates.get("total", 0) > estimates.get("hard", 12000):
                    raise BrainError("retrieval manifest exceeds hard token ceiling")
            except (OSError, json.JSONDecodeError, BrainError, KeyError, TypeError) as error:
                errors.append(f"{path}: {error}")
    promotions = brain_root(repository) / "control" / "promotions"
    if promotions.is_dir():
        for path in sorted(promotions.glob("*.json")):
            try:
                promotion = json.loads(path.read_text(encoding="utf-8"))
                if not isinstance(promotion, dict):
                    raise BrainError("promotion must be an object")
                validate_promotion_record(
                    repository, promotion, expected_id=path.stem
                )
            except (OSError, json.JSONDecodeError, BrainError, KeyError, TypeError) as error:
                errors.append(f"{path}: {error}")
    errors.extend(index_errors(repository))
    return errors


_INDEX_REMEDY = ("It is derived from the records: the next Brain write rebuilds it - a "
                 "checkpoint, `update`, `record-result` or `compact`.")


def index_errors(repository: Path) -> list[str]:
    """What is wrong with the two deterministic indexes, each named by its
    repository path, with the way out: a merge of two branches that both
    added records leaves conflict markers or one side's list in them."""
    def shown(path: Path) -> str:
        # Attached, the Brain lives in the state directory, outside the project.
        try:
            return _relative_posix(path, repository)
        except ValueError:
            return path.as_posix()
    errors: list[str] = []
    try:
        expected = build_index_data(repository)
    except (OSError, BrainError) as error:
        return [f"{shown(brain_root(repository) / 'indexes')}: {error}"]
    for path, wanted in zip(index_paths(repository), expected):
        relative = shown(path)
        try:
            actual = json.loads(path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as error:
            errors.append(f"{relative}: deterministic index is not valid JSON ({error}); a merge "
                          f"can leave conflict markers in it. {_INDEX_REMEDY}")
            continue
        except (OSError, UnicodeError) as error:
            errors.append(f"{relative}: deterministic index unreadable ({error}). {_INDEX_REMEDY}")
            continue
        if actual != wanted:
            errors.append(f"{relative}: deterministic index is stale or non-canonical. {_INDEX_REMEDY}")
    return errors


def bank_write_errors(
    repository: Path,
    before: list[str],
    attested: tuple[Path, ...] = (),
) -> list[str]:
    """What a Memory Bank write broke, as opposed to what the bank already had.

    Every write used to validate the whole bank afterwards and roll back on any
    error at all. A chunk that merely reached its review date therefore failed
    every promotion, compaction, re-attestation and retirement in the
    repository, and two overdue chunks each blocked the one command that would
    have repaired the other. Time alone made the bank unwritable.

    A write is now refused for errors it introduced, compared against the
    bank as it stood before the write (``before``), and for any error in a
    chunk it produced or attested (``attested``): a new chunk, or one an
    operator just re-verified, meets the full contract whatever the rest of
    the bank looks like. A problem that predates the write stays where it
    always surfaces - `validate.py`, `bank-audit`, and retrieval exclusion -
    instead of being charged to the next unrelated write.
    """
    bank = repository / "memory-bank"
    baseline = set(before)
    errors = [error for error in validate_bank(bank) if error not in baseline]
    for path in attested:
        try:
            validate_bank_metadata(
                path,
                parse_bank_frontmatter(path),
                repository,
                # A sink, so an attested chunk that is now terminal gets the
                # same treatment of a deleted citation as the full report.
                [],
                label=bank_display_path(path, bank),
            )
            validate_bank_secrets(path)
        except (OSError, BankValidationError) as error:
            message = f"{bank_display_path(path, bank)}: {error}"
            if message not in errors:
                errors.append(message)
    return errors


def promoted_source_rewrites(
    repository: Path, renames: dict[str, str]
) -> list[tuple[Path, str]]:
    """Plan the Memory Bank edits that keep promoted citations resolvable.

    A chunk cites its source record by path. Archiving a promoted record
    without repointing the citation leaves a dangling reference that fails
    Memory Bank validation, which in turn blocks every later promotion. The
    rewrite is planned separately from applying it so compaction can snapshot
    the affected chunks and roll them back with the moves.
    """
    rewrites: list[tuple[Path, str]] = []
    chunks = repository / "memory-bank" / "chunks"
    if not renames or not chunks.is_dir():
        return rewrites
    for path in sorted(chunks.glob("*.md")):
        try:
            metadata, body = parse_markdown_record(path)
        except BrainError:
            continue
        sources = metadata.get("sources")
        if not isinstance(sources, list):
            continue
        updated: list[Any] = []
        touched = False
        for source in sources:
            if not isinstance(source, str):
                updated.append(source)
                continue
            head, separator, fragment = source.partition("#")
            if head in renames:
                updated.append(renames[head] + separator + fragment)
                touched = True
            else:
                updated.append(source)
        if touched:
            metadata["sources"] = updated
            digests = metadata.get("source_digests")
            if isinstance(digests, list):
                # The file moved; its contents did not. Repointing the digest
                # keeps the path sets equal - leave them diverged and every
                # promoted chunk reads as source-changed forever after the
                # first compaction.
                metadata["source_digests"] = [
                    {**entry, "path": renames[entry["path"]]}
                    if isinstance(entry, dict) and entry.get("path") in renames
                    else entry
                    for entry in digests
                ]
            rewrites.append((path, render_markdown_record(metadata, body)))
    return rewrites


# A ticket, issue or record number: an uppercase prefix, a hyphen and a
# number - `CMS-741`. A space does not join them: "PHP 8.2" and "ISO 8601" are
# a language and a standard with their versions, not work items, and a number
# that runs on into a decimal is a version as well ("PHP-8.2").
TICKET_PATTERN = re.compile(r"\b([A-Z]{2,10})-(\d{1,6})\b(?!\.\d)")
# Prefixes that, with a number, name a standard, an encoding, an algorithm, a
# protocol, a license or a product version rather than a work item. "UTF-8"
# made two unrelated completion events one ticket's series, and "PSR-4" in a
# chunk withheld it as evidence about another ticket. A project that files its
# work under one of these keys loses only the ticket preference for it.
TICKET_STOP_PREFIXES = frozenset(
    """
    AES AGPL ANSI ASCII BCP BSD CET CP CRC CSS CVE CVSS CWE DES DIN DNS DSA
    ECDSA ECMA EN EPL ES FIPS GDPR GMT GPL GPT HMAC HTML HTTP HTTPS IEC IEEE IP
    IPV ISO JSON KOI LATIN LGPL MD MPEG MPL NIST OAUTH OWASP PCI PER PHP PSR
    RFC RSA SHA SOC SSL TCP TLS UCS UDP UTC UTF UUID WCAG WIN XML YAML
    """.split()
)
CONSOLIDATION_MIN_EVENTS = 2
CONSOLIDATION_VERIFICATION = re.compile(r"Verification:\s*(.+?)(?:\s+##|\s*$)", re.S | re.I)
CONSOLIDATED_PREFIX = "CONSOLIDATED-"
# A consolidated finding carries each event it covers on a line of its own -
# what was done, the outcome and the verification the event recorded - ending
# with the event's UUID. The line is the event's content once the event leaves
# retrieval, and the UUID is how a later pass knows the event is covered.
_CONSOLIDATED_ENTRY = re.compile(
    r"^- .*\[event ([0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12})\]$",
    re.M,
)
CONSOLIDATED_ENTRY_LIMIT = 2000


def ticket_ids(text: object, known: Optional[Iterable[str]] = None) -> list[str]:
    """Ticket identifiers ``text`` names, uppercase and in order, each once.

    ``known`` holds the prefixes a project files its work under; when it names
    any, only those count, so `GPT-4` or `COVID-19` is not taken for a ticket
    where the work is `CMS-741`. Without it the shape and the stop list decide.
    """
    allowed = frozenset(known or ())
    found: list[str] = []
    for prefix, number in TICKET_PATTERN.findall(str(text or "")):
        if prefix in TICKET_STOP_PREFIXES or (allowed and prefix not in allowed):
            continue
        value = f"{prefix}-{number}"
        if value not in found:
            found.append(value)
    return found


def completion_subject(title: object) -> Optional[str]:
    """What a completion event says it completed: the `X` of `Completed X: ...`."""
    value = " ".join(str(title or "").split())
    if not value.casefold().startswith("completed "):
        return None
    subject, separator, _ = value[len("completed "):].partition(": ")
    if not separator:
        return None
    return subject.strip() or None


def work_ticket_prefixes(record: dict[str, Any]) -> set[str]:
    """Ticket prefixes a record shows the project filing its work under.

    A task's own ID, and the task a completion event names: both are names
    people gave their work, unlike a ticket an outcome or a document merely
    mentions.
    """
    if record.get("type") == "task":
        text: object = record.get("external_id")
    elif record.get("type") == "event":
        text = completion_subject(record.get("title"))
    else:
        return set()
    return {value.split("-", 1)[0] for value in ticket_ids(text)}


def _event_tickets(record: dict[str, Any]) -> list[str]:
    """Ticket IDs an event names, title first and external ID last.

    A completion event's title leads with the task it completed (`Completed
    CMS-741: ...`), so its first ticket is the subject. An event's own
    external ID may itself look like a ticket (`EV-1`); it is a fallback,
    never the subject the event is about. The goal is not read: an outcome
    that mentions a related ticket does not move the event into that
    ticket's series.
    """
    tickets: list[str] = []
    for key in ("title", "external_id"):
        for value in ticket_ids(record.get(key)):
            if value not in tickets:
                tickets.append(value)
    return tickets


def _first_sentence(text: object, limit: int = 600) -> str:
    value = " ".join(str(text or "").split())
    if not value:
        return ""
    end = re.search(r"(?<=[.!?])\s+", value)
    sentence = value[: end.start()] if end else value
    return sentence[:limit].rstrip()


def _event_subject(record: dict[str, Any]) -> str:
    """The descriptive part of a completed event's title, when it has one.

    A goal is often the literal word "completed" followed by a verification
    block; taking the latest goal as the finding's title produced
    "CMS-744: completed Verification: ..." on the real 27-event series. The
    title after "Completed <ticket>: " names the work instead.
    """
    title = " ".join(str(record.get("title") or "").split())
    if title.casefold().startswith("completed "):
        title = title[len("Completed "):]
    if ": " in title:
        title = title.split(": ", 1)[1]
    title = title.strip(" :-")
    if len(title.split()) < 4:
        return ""
    if title.casefold().startswith(("completed", "verification")):
        return ""
    if "auto-provisioned" in title.casefold():
        return ""
    if re.match(r"^[A-Z]{2,10}-\d+\b", title):
        return ""
    return title


def _event_verification(record: dict[str, Any]) -> str:
    """What an event says verified it, or '' when it says nothing."""
    match = CONSOLIDATION_VERIFICATION.search(str(record.get("goal") or ""))
    return " ".join(match.group(1).split()) if match else ""


def _event_outcome(record: dict[str, Any]) -> str:
    """The outcome an event recorded, without its verification."""
    goal = str(record.get("goal") or "")
    match = CONSOLIDATION_VERIFICATION.search(goal)
    outcome = " ".join((goal[: match.start()] if match else goal).split())
    # A completion without an outcome of its own records the bare word.
    if outcome.rstrip(".").casefold() in ("completed", "complete", "done"):
        return ""
    return outcome


def _clipped(text: str, limit: int) -> str:
    if len(text) <= limit:
        return text
    cut = text[: max(1, limit - 1)]
    space = cut.rfind(" ")
    if space > limit // 2:
        cut = cut[:space]
    return cut.rstrip(" ,;:") + "…"


def _consolidation_entry(record: dict[str, Any]) -> str:
    """The line that carries one event in its ticket's finding: what was done,
    the outcome and the verification, whole up to a generous bound."""
    title = " ".join(str(record.get("title") or "").split())
    subject = title.split(": ", 1)[1] if completion_subject(title) is not None else title
    subject = subject.strip(" :-") or str(record.get("external_id") or "event")
    parts = [_clipped(subject, 160) + ":"]
    outcome = _event_outcome(record)
    if outcome:
        parts.append(outcome)
    verification = _event_verification(record)
    if verification:
        parts.append(f"Verification: {verification}")
    return f"- {_clipped(' '.join(parts), CONSOLIDATED_ENTRY_LIMIT)} [event {record['id']}]"


def _covered_events(record: Optional[dict[str, Any]]) -> dict[str, str]:
    """The events a consolidated finding already carries, by UUID, with their lines."""
    if record is None:
        return {}
    return {
        match.group(1): match.group(0)
        for match in _CONSOLIDATED_ENTRY.finditer(str(record.get("progress") or ""))
    }


def _consolidated_progress(
    ticket: str, previous: str, lines: list[str], related: list[str]
) -> str:
    """A consolidated finding's progress: every event line, old ones first.

    Lines a previous pass wrote are kept as they are, and so is other text a
    finding written before these lines existed carried, so nothing it said is
    lost when an event is added.
    """
    kept: list[str] = []
    entries: list[str] = []
    for line in previous.splitlines():
        if _CONSOLIDATED_ENTRY.fullmatch(line):
            entries.append(line)
        elif line.strip() and not (
            re.fullmatch(r"Consolidated \d+ completed events? for \S+\.", line)
            or line.startswith("Related findings: ")
        ):
            kept.append(line)
    entries.extend(lines)
    head = (
        f"Consolidated {len(entries)} completed event"
        f"{'' if len(entries) == 1 else 's'} for {ticket}."
    )
    tail = ["Related findings: " + ", ".join(related[:8]) + "."] if related else []
    return "\n".join([head, *kept, *entries, *tail])


def _local_sources(repository: Path, records: list[dict[str, Any]]) -> list[str]:
    """The records' sources a finding can fingerprint: local files that exist."""
    sources: list[str] = []
    for record in records:
        for source in record.get("sources") or []:
            if not isinstance(source, str) or not source or source in sources:
                continue
            if source.startswith(("https://", "http://")):
                # The record can name a remote source; a durable finding is
                # fingerprinted locally, and an unfingerprinted URL would make
                # the new record unvalidatable.
                continue
            try:
                path = workspace_roots.resolve(repository, source.split("#", 1)[0])
            except (OSError, ValueError):
                continue
            if path.is_file():
                sources.append(source)
    return sources


def _consolidated_finding(
    repository: Path, ticket: str, events: list[tuple[Path, dict[str, Any], str]],
    related: list[str],
) -> dict[str, Any]:
    ordered = sorted(events, key=lambda item: str(item[1].get("created_at") or ""))
    latest = ordered[-1][1]
    subject = next(
        (
            candidate
            for candidate in (_event_subject(record) for _, record, _ in reversed(ordered))
            if candidate
        ),
        "",
    )
    goal = subject or _first_sentence(latest.get("goal")) or _first_sentence(latest.get("title")) or ticket
    title = f"{ticket}: {goal}"
    if len(title) > 160:
        title = title[:157].rstrip() + "..."
    records = [record for _, record, _ in ordered]
    files: list[str] = []
    for record in records:
        for path in record.get("files") or []:
            if isinstance(path, str) and path and path not in files:
                files.append(path)
    verified = sum(1 for record in records if _event_verification(record))
    return {
        "external_id": f"{CONSOLIDATED_PREFIX}{ticket}",
        "title": title,
        "goal": goal,
        "files": files,
        "sources": _local_sources(repository, records),
        "progress": _consolidated_progress(
            ticket, "", [_consolidation_entry(record) for record in records], related
        ),
        "events": len(ordered),
        "verifications": verified,
        # Every event recorded its own verification. Counting distinct
        # sentences instead left a finding unverified when two events both
        # said "unit tests passed".
        "all_verified": verified == len(ordered),
    }


def _consolidation_findings(
    repository: Path,
) -> tuple[dict[str, list[tuple[dict[str, Any], bool]]], dict[str, list[dict[str, Any]]]]:
    """Every finding by the ticket it is about, in one pass over the Brain and
    its archive: each ticket's consolidated findings - the active one and any
    archived earlier one - and the other findings that name the ticket."""
    archive = brain_root(repository) / "archive"
    own: dict[str, list[tuple[dict[str, Any], bool]]] = {}
    related: dict[str, list[dict[str, Any]]] = {}
    for path, record, _ in iter_records(repository, include_archive=True):
        if record.get("type") != "finding":
            continue
        external = str(record.get("external_id") or "")
        if external.startswith(CONSOLIDATED_PREFIX):
            own.setdefault(external[len(CONSOLIDATED_PREFIX):], []).append(
                (record, archive in path.parents)
            )
            continue
        for value in ticket_ids(f"{external} {record.get('title') or ''}".upper()):
            related.setdefault(value, []).append(record)
    return own, related


def _promoted_chunk(
    repository: Path, record: dict[str, Any], latest: dict[str, dict[str, Any]]
) -> Optional[str]:
    """The active chunk that carries this exact revision of the record, if any."""
    proposal = latest.get(record["id"])
    if proposal is None or proposal["source_records"][0].get("revision") != record["revision"]:
        return None
    memory_id = proposal.get("destination_memory_id")
    return memory_id if _chunk_status(repository, memory_id) == "active" else None


def consolidate(
    repository: Path,
    *,
    apply: bool = False,
    ticket: Optional[str] = None,
    owner: str = "local",
) -> dict[str, Any]:
    """Collapse a ticket's completed events into one finding, optionally applying.

    Read-only without ``apply``. The finding carries every event it covers on
    a line of its own - what was done, the outcome and the verification - and
    cites the events' own sources, not the event files: compaction may archive
    those, and a citation to a moved record would make the durable conclusion
    stale. Events that reach the ticket later are added to its finding while
    the finding is active; once it is archived, a new finding takes them.

    Retrieval serves no resolved finding, so an event leaves the diary only
    once its finding's current revision is an active Memory Bank chunk: until
    then the event is the one place its outcome can be found, on a fresh clone
    as anywhere. It is then marked ``superseded`` by the operator, never in
    another owner's name, and a later ``compact`` moves it.

    Only the operator's own events are folded in; another owner's are left to
    that owner. A ticket that already has a resolved finding of its own is
    left alone: a second conclusion would duplicate it, and superseding the
    events into it would drop their text, which it does not carry.
    """
    config = load_config(repository)
    verify_consolidated = bool(config.get("automatic_consolidation_verification"))
    wanted = ticket.strip().upper() if isinstance(ticket, str) and ticket.strip() else None
    groups: dict[str, list[tuple[Path, dict[str, Any], str]]] = {}
    foreign: dict[str, int] = {}
    active: dict[str, tuple[Path, dict[str, Any]]] = {}
    for path, record, body in iter_records(repository):
        if record.get("type") == "finding":
            external = str(record.get("external_id") or "")
            if external.startswith(CONSOLIDATED_PREFIX):
                active[external[len(CONSOLIDATED_PREFIX):]] = (path, record)
            continue
        if record.get("type") != "event" or record.get("status") != "recorded":
            continue
        if record.get("privacy") not in ("public", "team"):
            # A shared finding must not carry a private or restricted event's
            # text into a team-visible record.
            continue
        if "completed" not in str(record.get("title") or "").casefold():
            continue
        tickets = _event_tickets(record)
        if not tickets:
            continue
        value = tickets[0]
        if wanted is not None and value != wanted:
            continue
        if owner not in (record.get("authorized_owners") or []):
            # Another owner's record is theirs to supersede, and their own
            # consolidation adds it to the ticket's finding.
            foreign[value] = foreign.get(value, 0) + 1
            continue
        groups.setdefault(value, []).append((path, record, body))
    report: dict[str, Any] = {
        "applied": apply,
        "tickets": [],
        "created": 0,
        "updated": 0,
        "existing": 0,
        "verified": 0,
        "superseded": 0,
        "blocked": 0,
    }
    # Read once for the whole pass, and only when a ticket needs them: a
    # ticket whose events its active finding already covers needs neither the
    # archive nor the promotion ledger.
    index: Optional[tuple[dict[str, list[tuple[dict[str, Any], bool]]],
                          dict[str, list[dict[str, Any]]]]] = None
    ledger: Optional[dict[str, dict[str, Any]]] = None
    for value in sorted(set(groups) | set(foreign)):
        events = sorted(
            groups.get(value, []),
            key=lambda item: (str(item[1].get("created_at") or ""), str(item[1]["id"])),
        )
        entry: dict[str, Any] = {
            "ticket": value,
            "events": len(events),
            "ids": [str(record.get("id")) for _, record, _ in events],
        }
        if foreign.get(value):
            entry["foreign"] = foreign[value]
        report["tickets"].append(entry)
        if not events:
            entry.update(
                state="skipped",
                reason="the completed events belong to another owner, who consolidates them",
            )
            continue
        summary_path, summary = active.get(value, (None, None))
        covered = _covered_events(summary)
        uncovered = [item for item in events if item[1]["id"] not in covered]
        if summary is not None and not uncovered and summary.get("authority") != "verified":
            # A ticket's steady state: nothing new, and an observed finding is
            # never promoted, so no event can leave the diary yet.
            entry.update(
                state="unchanged", finding=summary.get("external_id"),
                reason="the events stay recorded until the finding is verified and promoted",
            )
            continue
        archived: list[dict[str, Any]] = []
        related: list[dict[str, Any]] = []
        if summary is None or uncovered:
            if index is None:
                index = _consolidation_findings(repository)
            archived = [record for record, gone in index[0].get(value, []) if gone]
            related = index[1].get(value, [])
        # The events a Memory Bank chunk already carries leave the diary.
        retiring: list[tuple[dict[str, Any], str, list[tuple[Path, dict[str, Any], str]]]] = []
        taken: set[str] = set()
        for volume in ([summary] if summary is not None else []) + archived:
            carried = _covered_events(volume)
            taken.update(carried)
            members = [item for item in events if item[1]["id"] in carried]
            if not members or volume.get("authority") != "verified":
                continue
            if ledger is None:
                ledger = promotion_ledger(repository)[1]
            memory_id = _promoted_chunk(repository, volume, ledger)
            if memory_id is not None:
                retiring.append((volume, memory_id, members))
        if retiring:
            entry["supersede"] = sum(len(members) for _, _, members in retiring)
            if apply:
                superseded = _retire_consolidated_events(repository, retiring, owner, entry)
                if superseded is None:
                    report["blocked"] += 1
                    continue
                entry["superseded"] = superseded
                report["superseded"] += superseded
        remaining = [item for item in events if item[1]["id"] not in taken]
        if not remaining:
            if retiring:
                entry["state"] = "superseded" if apply else "planned-supersede"
            else:
                entry.update(
                    state="unchanged", finding=summary.get("external_id") if summary else None,
                    reason="the events stay recorded until the finding's revision is promoted",
                )
            continue
        names = [f"{record.get('external_id')} ({record.get('id')})" for record in related]
        if summary is not None:
            _extend_consolidated(
                repository, summary_path, summary, value, remaining, names, entry, report,
                apply=apply, owner=owner, verify_consolidated=verify_consolidated,
            )
            continue
        resolved = next(
            (record for record in related if record.get("status") == "resolved"), None
        )
        if resolved is not None:
            # The ticket's own conclusion exists. A second one would duplicate
            # it, and superseding the events into it would drop their text.
            entry.update(
                state="existing", finding=resolved.get("external_id"),
                reason="the ticket has a resolved finding; the events stay recorded",
            )
            report["existing"] += 1
            continue
        if len(remaining) < CONSOLIDATION_MIN_EVENTS:
            entry.update(state="skipped", reason="fewer than two completed events")
            continue
        planned = _consolidated_finding(repository, value, remaining, names)
        entry.update(
            state="planned", finding=planned["external_id"], title=planned["title"],
            sources=len(planned["sources"]), adds=len(remaining),
        )
        if not apply:
            continue
        verify = bool(verify_consolidated and planned["all_verified"] and planned["sources"])
        if not _create_consolidated(repository, planned, entry, owner=owner, verify=verify):
            report["blocked"] += 1
            continue
        report["created"] += 1
        if verify:
            report["verified"] += 1
    return report


def _create_consolidated(
    repository: Path, planned: dict[str, Any], entry: dict[str, Any], *,
    owner: str, verify: bool,
) -> bool:
    """Write a new consolidated finding, resolved - and verified when every
    event recorded its verification; False with a blocked entry otherwise."""
    with mutation_lock(repository):
        snapshot = snapshot_files(list(index_paths(repository)))
        created: Optional[dict[str, Any]] = None
        try:
            created = create_record(
                repository,
                "finding",
                planned["external_id"],
                planned["title"],
                planned["files"],
                planned["sources"],
                owner=owner,
                authority="observed",
                goal=planned["goal"],
            )
            updated = update_record(
                repository,
                created["id"],
                expected_revision=created["revision"],
                progress=planned["progress"],
                next_steps=[],
                files=[],
                sources=[],
                actor=owner,
                transition_to="resolved",
                reason=f"Consolidated {planned['events']} completed events",
            )
            if verify:
                update_record(
                    repository,
                    created["id"],
                    expected_revision=updated["revision"],
                    progress=None,
                    next_steps=[],
                    files=[],
                    sources=[],
                    actor=owner,
                    authority="verified",
                    attestation="agent",
                    reason="Every consolidated event recorded verification",
                )
        except Exception as error:
            if created is not None:
                try:
                    rollback_created_record(repository, created["id"])
                except BrainError:
                    pass
            restore_files(snapshot)
            entry.update(state="blocked", reason=str(error))
            return False
    entry.update(state="created", verified=verify)
    return True


def _extend_consolidated(
    repository: Path, path: Path, summary: dict[str, Any], ticket: str,
    events: list[tuple[Path, dict[str, Any], str]], related: list[str],
    entry: dict[str, Any], report: dict[str, Any], *,
    apply: bool, owner: str, verify_consolidated: bool,
) -> None:
    """Add events that reached a ticket after its active finding was written.

    An observed finding takes them as they are, and becomes verified once
    every line it carries recorded a verification. A verified finding is a
    checked claim: it takes new events only when each recorded its own
    verification, as a revised claim the agent attests; otherwise they stay
    recorded until the finding is archived and a new one takes them.
    """
    entry["finding"] = summary.get("external_id")
    records = [record for _, record, _ in events]
    verified = summary.get("authority") == "verified"
    if owner not in (summary.get("authorized_owners") or []):
        entry.update(
            state="skipped",
            reason=f"the finding belongs to {summary.get('owner')}; the events stay recorded",
        )
        return
    if verified and not (
        verify_consolidated and all(_event_verification(record) for record in records)
    ):
        entry.update(
            state="skipped",
            reason="the verified finding takes only verified events; they stay recorded",
        )
        return
    progress = _consolidated_progress(
        ticket, str(summary.get("progress") or ""),
        [_consolidation_entry(record) for record in records], related,
    )
    sources = _local_sources(repository, records)
    # Every line after the head must be an event line that recorded its
    # verification: text a finding carried from before those lines existed
    # says nothing of whether its earlier events were verified.
    carried = progress.splitlines()[1:]
    if carried and carried[-1].startswith("Related findings: "):
        carried = carried[:-1]
    harden = bool(
        not verified
        and verify_consolidated
        and (summary.get("sources") or sources)
        and carried
        and all(
            _CONSOLIDATED_ENTRY.fullmatch(line) and " Verification: " in line
            for line in carried
        )
    )
    entry.update(state="planned-update", adds=len(records))
    if not apply:
        return
    files = list(dict.fromkeys(
        item for record in records for item in record.get("files") or []
        if isinstance(item, str) and item
    ))
    with mutation_lock(repository):
        snapshot = snapshot_files([path, *index_paths(repository)])
        try:
            update_record(
                repository,
                summary["id"],
                expected_revision=summary["revision"],
                progress=progress,
                next_steps=[],
                files=files,
                sources=sources,
                actor=owner,
                authority="verified" if harden else None,
                attestation="agent" if harden or verified else None,
                reverify=verified,
                reason=f"Added {len(records)} completed event(s) of {ticket}",
            )
        except Exception as error:
            restore_files(snapshot)
            entry.update(state="blocked", reason=str(error))
            report["blocked"] += 1
            return
    entry.update(state="updated", verified=verified or harden)
    report["updated"] += 1
    if harden:
        report["verified"] += 1


def _retire_consolidated_events(
    repository: Path,
    retiring: list[tuple[dict[str, Any], str, list[tuple[Path, dict[str, Any], str]]]],
    owner: str,
    entry: dict[str, Any],
) -> Optional[int]:
    """Supersede the events whose finding a chunk now carries, as the
    operator: how many, or None with a blocked entry."""
    with mutation_lock(repository):
        snapshot = snapshot_files(
            [path for _, _, members in retiring for path, _, _ in members]
            + list(index_paths(repository))
        )
        count = 0
        try:
            for volume, memory_id, members in retiring:
                for _, record, _ in members:
                    update_record(
                        repository,
                        record["id"],
                        expected_revision=record["revision"],
                        progress=None,
                        next_steps=[],
                        files=[],
                        sources=[],
                        actor=owner,
                        transition_to="superseded",
                        reason=(
                            f"Consolidated into {volume.get('external_id')}, "
                            f"carried by {memory_id}"
                        ),
                    )
                    count += 1
        except Exception as error:
            restore_files(snapshot)
            entry.update(state="blocked", reason=str(error))
            return None
    return count


def _retain_consolidated(record: dict[str, Any]) -> bool:
    """Whether a consolidated, still-observed finding must stay in dynamic.

    Resolved is terminal, so the automatic compaction on the same turn
    boundary would archive the finding the moment consolidation wrote it.
    Retrieval serves neither copy - the ticket's events stay recorded for
    that - but the active one is where the ticket's later events are added
    and where a reviewer finds it. It stays until it is verified, when
    promotion carries it into the Memory Bank.
    """
    return (
        record.get("type") == "finding"
        and str(record.get("external_id") or "").startswith(CONSOLIDATED_PREFIX)
        and record.get("authority") != "verified"
    )


def compact(repository: Path, *, only_external_id: Optional[str] = None) -> dict[str, int]:
    """Archive terminal and superseded records; with ``only_external_id``,
    only those of that ID - what a new task of the same name needs moved."""
    with mutation_lock(repository):
        # Without freshness: moving terminal records can neither cause nor
        # cure a stale citation, and the documented contract is that
        # compaction skips stale records, not that one refuses all of it.
        errors = validate_repository(repository, check_freshness=False)
        # The indexes are rebuilt from the records below: a stale or
        # conflicted one is what compaction repairs, not a reason to refuse.
        derived = set(index_errors(repository))
        errors = [error for error in errors if error not in derived]
        if errors:
            raise BrainError(
                "Compaction refused because active records are invalid "
                f"({len(errors)} problem(s); see `context.py validate`)"
            )
        moves: list[tuple[Path, Path]] = []
        for path, record, _ in list(iter_records(repository)):
            if (
                record["status"] not in LIFECYCLES[record["type"]]["terminal"]
                and record.get("superseded_by") is None
            ):
                continue
            if _retain_consolidated(record):
                continue
            if only_external_id is not None and record.get("external_id") != only_external_id:
                continue
            destination = brain_root(repository) / "archive" / record["type"] / path.name
            if destination.exists():
                raise BrainError(f"Archive destination already exists: {destination}")
            moves.append((path, destination))
            handoff = handoff_path(repository, record["id"])
            if handoff.exists():
                archived_handoff = brain_root(repository) / "archive" / "handoffs" / handoff.name
                if archived_handoff.exists():
                    raise BrainError(
                        f"Archive destination already exists: {archived_handoff}"
                    )
                moves.append((handoff, archived_handoff))
        renames = {
            source.relative_to(repository).as_posix(): destination.relative_to(
                repository
            ).as_posix()
            for source, destination in moves
        }
        rewrites = promoted_source_rewrites(repository, renames)
        bank = repository / "memory-bank"
        bank_before = validate_bank(bank) if bank.is_dir() else []
        snapshot = snapshot_files(
            [path for move in moves for path in move]
            + list(index_paths(repository))
            + [path for path, _ in rewrites]
        )
        try:
            for source, destination in moves:
                destination.parent.mkdir(parents=True, exist_ok=True)
                os.replace(str(source), str(destination))
            for path, content in rewrites:
                atomic_write(path, content)
            rebuild_indexes(repository)
            post_errors = validate_repository(repository, check_freshness=False)
            if post_errors:
                raise BrainError(
                    "Archive validation failed after compaction: "
                    + "; ".join(post_errors)
                )
            if bank.is_dir():
                # Repointing a citation is mechanical, not an attestation:
                # only what the move broke may stop it.
                bank_errors = bank_write_errors(repository, bank_before)
                if bank_errors:
                    raise BrainError(
                        "Memory Bank validation failed after compaction: "
                        + "; ".join(bank_errors)
                    )
        except Exception:
            restore_files(snapshot)
            raise
        prune_result_evidence(repository)
    moved_records = sum(
        1 for source, _ in moves if "dynamic" in source.parts
    )
    return {"moved": moved_records}


def _evidence_entries(folder: Path) -> list[os.DirEntry]:
    try:
        if folder.is_symlink() or not folder.is_dir():
            return []
        with os.scandir(folder) as entries:
            return list(entries)
    except OSError:
        return []


def _remove_evidence(entry: os.DirEntry) -> int:
    try:
        if _EVIDENCE_NAME.fullmatch(entry.name) and entry.is_file(follow_symlinks=False):
            os.unlink(entry.path)
            return 1
    except OSError:
        pass
    return 0


def prune_result_evidence(repository: Path) -> int:
    """Remove save evidence that names no active record; return the count.

    An archived task takes no new memory and an archived learning is never
    revised, so what is left of their saves could serve only a replay - and
    the clone that saved proves that from its own receipt. Runs under the
    compaction lock, best effort: evidence left behind is harmless, and the
    next compaction tries again.
    """
    try:
        active = {record["id"] for _, record, _ in iter_records(repository)}
    except (BrainError, OSError):
        return 0
    removed = 0
    for folder in _evidence_entries(brain_root(repository).joinpath(*RESULT_ATTEMPTS)):
        if not folder.is_dir(follow_symlinks=False):
            # A file at the top is the layout before folders per task;
            # nothing reads it any more.
            removed += _remove_evidence(folder)
            continue
        if folder.name in active:
            continue
        for entry in _evidence_entries(Path(folder.path)):
            removed += _remove_evidence(entry)
        try:
            os.rmdir(folder.path)
        except OSError:
            pass
    for entry in _evidence_entries(brain_root(repository).joinpath(*RESULT_LEARNING_IMAGES)):
        try:
            with open(entry.path, "rb") as stream:
                record_id = json.loads(stream.read(4097)).get("record_id")
        except (OSError, ValueError, AttributeError):
            continue
        if isinstance(record_id, str) and record_id not in active:
            removed += _remove_evidence(entry)
    return removed


PROMOTION_KEYS = {
    "schema_version", "id", "status", "revision", "proposer", "reviewer",
    "review_mode", "reviewed_at", "created_at", "updated_at", "source_records",
    "title", "content", "destination_memory_id", "destination_revision",
    "conflicts", "outcome",
}
REVIEW_MODES = ("human", "automatic")
# Only a resolved consequence is durable. Tasks are excluded on purpose: their
# progress is checkpoint bookkeeping, not reusable knowledge, and events record
# that something happened rather than what to do about it.
PROMOTABLE_STATES = {
    "finding": {"resolved"},
    "bug": {"resolved"},
    "incident": {"closed"},
    "decision": {"accepted"},
}

# What a promoted record becomes in the bank. Kept beside PROMOTABLE_STATES
# because the two must cover the same four record types: every promotion wrote
# `type: decision` regardless of origin, so a resolved bug and a closed
# incident both entered the bank claiming to be decisions, and `bank-audit`,
# INDEX.md and every reader keyed on type were reading a value nothing had
# chosen. Each value is in validate.ALLOWED_TYPES.
PROMOTION_TYPE_BY_RECORD = {
    "finding": "domain",
    "bug": "constraint",
    "incident": "operations",
    "decision": "decision",
}


def validate_promotion_record(
    repository: Path, promotion: dict[str, Any], *, expected_id: Optional[str] = None
) -> None:
    validate_schema_file(repository, "promotion.schema.json", promotion)
    if set(promotion) != PROMOTION_KEYS:
        raise BrainError("promotion does not match strict schema")
    if not is_uuid4(promotion["id"]) or (
        expected_id is not None and promotion["id"] != expected_id
    ):
        raise BrainError("promotion UUID must match filename")
    if promotion["status"] not in {"proposed", "reviewed", "rejected", "applied"}:
        raise BrainError("invalid promotion status")
    if promotion["review_mode"] not in REVIEW_MODES:
        raise BrainError("invalid promotion review mode")
    if (
        promotion["status"] in {"reviewed", "applied"}
        and not promotion["reviewer"]
        and promotion["review_mode"] != "automatic"
    ):
        raise BrainError("reviewed promotion requires reviewer")
    if promotion["review_mode"] == "automatic" and promotion["reviewer"]:
        # An automatic promotion must not name a reviewer: the record has to
        # state plainly that no human approved it.
        raise BrainError("automatic promotion must not claim a reviewer")
    expected_outcome = (
        "approved-without-review"
        if promotion["review_mode"] == "automatic"
        else "approved"
    )
    if promotion["status"] == "proposed" and promotion["outcome"] is not None:
        raise BrainError("proposed promotion outcome must be null")
    if promotion["status"] == "rejected" and promotion["outcome"] != "rejected":
        raise BrainError("rejected promotion outcome must be rejected")
    if (
        promotion["status"] in {"reviewed", "applied"}
        and promotion["outcome"] != expected_outcome
        # v1 used `promoted` after apply. Keep those tracked records readable;
        # the reviewed boundary below never accepts or emits that legacy value.
        and not (
            promotion["status"] == "applied"
            and promotion["outcome"] == "promoted"
        )
    ):
        raise BrainError(
            f"{promotion['review_mode']} promotion outcome must be {expected_outcome}"
        )
    for source in promotion["source_records"]:
        if (
            not isinstance(source, dict)
            or set(source) != {"id", "type", "path", "revision"}
            or not is_uuid4(source["id"])
            or source["type"] not in RECORD_TYPES
            or not isinstance(source["path"], str)
            or not source["path"]
            or not isinstance(source["revision"], int)
            or source["revision"] < 1
        ):
            raise BrainError("invalid promotion source binding")


def create_promotion(
    repository: Path,
    source_ids: list[str],
    title: str,
    content: str,
    *,
    proposer: str,
    review_mode: str = "human",
) -> dict[str, Any]:
    if review_mode not in REVIEW_MODES:
        raise BrainError("Promotion review mode must be human or automatic")
    if not source_ids:
        raise BrainError("Promotion requires at least one source record")
    sources = []
    for identifier in source_ids:
        path, record, _ = find_record(repository, identifier, include_archive=True)
        sources.append(
            {
                "id": record["id"],
                "type": record["type"],
                "path": path.relative_to(repository).as_posix(),
                "revision": record["revision"],
            }
        )
    timestamp = utc_now()
    proposal = {
        "schema_version": 1, "id": new_uuid(), "status": "proposed", "revision": 1,
        "proposer": proposer, "reviewer": None, "review_mode": review_mode,
        "reviewed_at": None, "created_at": timestamp,
        "updated_at": timestamp, "source_records": sources, "title": title, "content": content,
        "destination_memory_id": None, "destination_revision": None, "conflicts": [],
        "outcome": None,
    }
    validate_promotion_record(repository, proposal)
    with mutation_lock(repository):
        atomic_json(brain_root(repository) / "control" / "promotions" / f"{proposal['id']}.json", proposal)
    return proposal


def review_promotion(repository: Path, promotion_id: str, reviewer: str, approve: bool) -> dict[str, Any]:
    path = brain_root(repository) / "control" / "promotions" / f"{promotion_id}.json"
    with mutation_lock(repository):
        try:
            proposal = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise BrainError(f"Promotion not found or invalid: {promotion_id}") from error
        validate_promotion_record(
            repository, proposal, expected_id=promotion_id
        )
        if proposal.get("status") != "proposed":
            raise BrainError("Only proposed promotions may be reviewed")
        if proposal.get("review_mode") == "automatic":
            raise BrainError(
                "Automatic promotions are not human-reviewable; "
                "a human review would misrepresent how they were approved"
            )
        if reviewer == proposal.get("proposer"):
            raise BrainError("Promotion reviewer must be independent from proposer")
        proposal["status"] = "reviewed" if approve else "rejected"
        proposal["reviewer"] = reviewer
        proposal["reviewed_at"] = utc_now()
        proposal["updated_at"] = utc_now()
        proposal["revision"] += 1
        proposal["outcome"] = "approved" if approve else "rejected"
        atomic_json(path, proposal)
        return proposal


def auto_review_promotion(repository: Path, promotion_id: str) -> dict[str, Any]:
    """Advance an automatic promotion to `reviewed` without naming a reviewer.

    This is not a stand-in for the human gate; it is the absence of one, made
    explicit. `reviewer` stays null so the record cannot be mistaken later for
    something a person approved.
    """
    path = brain_root(repository) / "control" / "promotions" / f"{promotion_id}.json"
    with mutation_lock(repository):
        try:
            proposal = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise BrainError(f"Promotion not found or invalid: {promotion_id}") from error
        validate_promotion_record(repository, proposal, expected_id=promotion_id)
        if proposal.get("review_mode") != "automatic":
            raise BrainError("Only automatic promotions may be auto-reviewed")
        if proposal.get("status") != "proposed":
            raise BrainError("Only proposed promotions may be reviewed")
        proposal["status"] = "reviewed"
        proposal["reviewed_at"] = utc_now()
        proposal["updated_at"] = utc_now()
        proposal["revision"] += 1
        proposal["outcome"] = "approved-without-review"
        validate_promotion_record(repository, proposal, expected_id=promotion_id)
        atomic_json(path, proposal)
        return proposal


def iter_promotions(repository: Path) -> Iterator[tuple[str, dict[str, Any]]]:
    directory = brain_root(repository) / "control" / "promotions"
    if not directory.is_dir():
        return
    for path in sorted(directory.glob("*.json")):
        try:
            yield path.stem, json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue


def promoted_source_ids(repository: Path) -> set[str]:
    """Return sources that must not be proposed again.

    An applied promotion is done, and one awaiting human review is legitimately
    in flight. An automatic promotion that stalled before applying is neither:
    it is retried in place rather than blocking its source for good.
    """
    promoted: set[str] = set()
    for _, proposal in iter_promotions(repository):
        if proposal.get("status") == "rejected":
            continue
        if (
            proposal.get("status") != "applied"
            and proposal.get("review_mode") == "automatic"
        ):
            continue
        for source in proposal.get("source_records", []):
            if isinstance(source, dict) and isinstance(source.get("id"), str):
                promoted.add(source["id"])
    return promoted


def stalled_automatic_promotions(repository: Path) -> list[tuple[str, dict[str, Any]]]:
    """Return automatic promotions that were created but never applied."""
    return [
        (promotion_id, proposal)
        for promotion_id, proposal in iter_promotions(repository)
        if proposal.get("review_mode") == "automatic"
        and proposal.get("status") in {"proposed", "reviewed"}
    ]


def promotion_ledger(repository: Path) -> tuple[set[str], dict[str, dict[str, Any]]]:
    """What the promotions say about each record: held, or last promoted alone.

    Held: in a person's review, or promoted together with other records - a
    synthesis is a person's to revise. Last promoted alone: the newest
    applied promotion of the record by itself, by the record revision it
    carried, which a revised record is compared with.
    """
    held: set[str] = set()
    latest: dict[str, dict[str, Any]] = {}
    for _, proposal in iter_promotions(repository):
        if not isinstance(proposal, dict) or proposal.get("status") == "rejected":
            continue
        sources = [
            source for source in proposal.get("source_records") or []
            if isinstance(source, dict) and isinstance(source.get("id"), str)
            and type(source.get("revision")) is int
        ]
        if proposal.get("status") != "applied":
            if proposal.get("review_mode") != "automatic":
                held.update(source["id"] for source in sources)
            continue
        if len(sources) != 1:
            held.update(source["id"] for source in sources)
            continue
        previous = latest.get(sources[0]["id"])
        if previous is None or sources[0]["revision"] > previous["source_records"][0]["revision"]:
            latest[sources[0]["id"]] = proposal
    return held, latest


def _chunk_status(repository: Path, memory_id: object) -> Optional[str]:
    if not isinstance(memory_id, str):
        return None
    try:
        return str(_find_chunk(repository / "memory-bank", memory_id)[1].get("status"))
    except BrainError:
        return None


def obsolete_promotion(
    repository: Path, proposal: dict[str, Any], applied: dict[str, dict[str, Any]]
) -> Optional[str]:
    """Why an unapplied automatic promotion can never apply, or None.

    Another promotion already brought its record's revision - or a later one
    - into the bank, or the record moved on: a revision is never taken back
    and a record compaction moved never moves back, so `apply_promotion`
    would refuse it on every retry for good.
    """
    source = proposal["source_records"][0]
    winner = applied.get(source["id"])
    if (
        winner is not None
        and winner.get("id") != proposal.get("id")
        and winner["source_records"][0]["revision"] >= source["revision"]
    ):
        return f"already promoted as {winner.get('destination_memory_id')}"
    try:
        path, record, _ = find_record(repository, source["id"], include_archive=True)
    except BrainRecordNotFound as error:
        if error.identifier == source['id'] and error.record_type is None:
            return "its record no longer exists"
        return None
    except BrainError:
        return None
    if record["revision"] != source["revision"]:
        return f"its record moved on to revision {record['revision']}"
    if path.relative_to(repository).as_posix() != source["path"]:
        return "its record moved to " + path.relative_to(repository).as_posix()
    return None


def withdraw_promotion(repository: Path, promotion_id: str, reason: str) -> dict[str, Any]:
    """Close an automatic promotion that can never apply.

    Left open it was retried, and failed, on every run for good. The schema
    has no withdrawn state, so it takes the closing state an unapplied
    proposal has - `rejected`, by nobody, which every reader already takes as
    "not promoted" - and `conflicts` keeps why, for the audit trail.
    """
    path = brain_root(repository) / "control" / "promotions" / f"{promotion_id}.json"
    with mutation_lock(repository):
        try:
            proposal = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise BrainError(f"Promotion not found or invalid: {promotion_id}") from error
        validate_promotion_record(repository, proposal, expected_id=promotion_id)
        if proposal.get("review_mode") != "automatic" or proposal.get("status") not in {"proposed", "reviewed"}:
            raise BrainError("Only an unapplied automatic promotion is withdrawn")
        proposal["status"] = "rejected"
        proposal["outcome"] = "rejected"
        proposal["updated_at"] = utc_now()
        proposal["revision"] += 1
        proposal["conflicts"] = list(dict.fromkeys([*proposal["conflicts"], f"withdrawn: {reason}"]))
        validate_promotion_record(repository, proposal, expected_id=promotion_id)
        atomic_json(path, proposal)
        return proposal


def promotion_content(record: dict[str, Any]) -> Optional[str]:
    """Build durable content from a record, or None when it says nothing.

    A record's rendered body is mostly lifecycle scaffolding — goal, progress,
    next steps, files — and for a decision the goal merely repeats the title.
    Promoting that verbatim fills durable memory with stubs that restate their
    own heading, so the reusable part is assembled explicitly and a record
    without one is not promoted at all.
    """
    title = str(record["title"]).strip()
    progress = " ".join(str(record.get("progress") or "").split())
    if not progress:
        return None
    if progress.rstrip(".").casefold() == title.rstrip(".").casefold():
        return None
    sections = [progress]
    sources = [
        source
        for source in record.get("sources", [])
        if isinstance(source, str) and source.strip()
    ]
    if sources:
        sections.append("## Sources\n" + "\n".join(f"- {source}" for source in sources))
    return "\n\n".join(sections)


def promotion_eligibility_error(
    repository: Path,
    record: dict[str, Any],
    config: dict[str, Any],
    *,
    allow_changed_sources: bool = False,
) -> Optional[str]:
    promotable = PROMOTABLE_STATES.get(record["type"])
    if promotable is None:
        return f"{record['type']} records are not promotable"
    if record["status"] not in promotable:
        return f"{record['type']} status {record['status']} is not promotable"
    if record["authority"] != "verified":
        return f"authority is {record['authority']}, not verified"
    if record["privacy"] not in config["allowed_privacy"]:
        return f"privacy {record['privacy']} is not allowed for retrieval"
    content = promotion_content(record)
    if content is None:
        return "record carries no content beyond its own title"
    found = sensitive_label(f"{record['title']}\n{content}")
    if found:
        # Durable memory is the widest audience a record can reach; a record
        # that predates the write guard must not carry its data there.
        return f"record content contains {found}"
    if sources_are_fresh(repository, record):
        return None
    if allow_changed_sources and citation_problem(repository, record) is None:
        # The automatic path promotes knowledge whose cited files were edited
        # after it was verified: the chunk keeps the digests taken at
        # verification, so retrieval serves it marked "source changed"
        # instead of the bank never learning it. On a real project 59 of 70
        # resolved findings were blocked here for good, because promotion ran
        # days after the fix that resolved them had edited the cited files.
        # A deleted citation, or one the record never digested, still blocks.
        return None
    stale = [
        item["path"]
        for item in record["source_fingerprints"]
        if not sources_are_fresh(
            repository,
            {"sources": [item["path"]], "source_fingerprints": [item]},
        )
    ]
    return "cited source changed since the record was written: " + ", ".join(
        stale or record["sources"]
    )


# A promoted chunk is durable shared knowledge, so text that tries to steer an
# agent is withheld from the automatic path even when its record is resolved
# and verified. A person can still review the proposal; the gate is about what
# the machine may do without one.
_PROVENANCE_INSTRUCTION_PATTERNS = (
    re.compile(r"\bignore\s+(?:all\s+)?(?:previous|prior|above)\s+instructions?\b", re.I),
    re.compile(r"\bdisregard\s+(?:all\s+)?(?:previous|prior|above|your)\s+instructions?\b", re.I),
    re.compile(r"\bforget\s+(?:all\s+)?(?:previous|prior|above|your)\s+instructions?\b", re.I),
    re.compile(r"\bsystem\s+prompt\b", re.I),
    re.compile(r"<\|(?:im_start|im_end|system|assistant)\|>", re.I),
)


def provenance_policy_fingerprint() -> str:
    """Identity of the instruction patterns. An index that screened a chunk
    under other patterns has not screened it under these."""
    return hashlib.sha256(
        "\n".join(
            f"{pattern.flags}:{pattern.pattern}" for pattern in _PROVENANCE_INSTRUCTION_PATTERNS
        ).encode("utf-8")
    ).hexdigest()


def instruction_like(title: object, content: object) -> bool:
    """Whether a title or text reads like an instruction to an agent."""
    text = "\n".join(part for part in (str(title or ""), str(content or "")) if part)
    return any(pattern.search(text) for pattern in _PROVENANCE_INSTRUCTION_PATTERNS)


def provenance_problems(record: dict[str, Any], content: Optional[str] = None) -> list[str]:
    """Why an automatic promotion may not carry this record, if it may not."""
    problems: list[str] = []
    sources = [source for source in record.get("sources") or [] if isinstance(source, str)]
    external = [source for source in sources if source.startswith(("https://", "http://"))]
    if external:
        problems.append(
            "external source requires review: " + external[0]
        )
    if instruction_like(record.get("title"), content):
        problems.append("instruction-like content requires review")
    return problems


def promotable_records(
    repository: Path,
    config: dict[str, Any],
    *,
    allow_changed_sources: bool = False,
    only: Optional[set[str]] = None,
) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    """Return promotable records, and the ones a rule held back.

    Eligibility is deliberately narrow. Tasks never qualify: auto-checkpoint
    progress describes what happened in a session, not a consequence worth
    carrying into another one. Only `verified` authority qualifies, because an
    automatic pipeline has no reviewer to catch an unverified claim.

    A record of a promotable type that a rule rejected is reported rather than
    dropped in silence. Editing a cited source is enough to hold a decision
    back forever, and an unexplained absence from durable memory is impossible
    to notice.
    """
    already = promoted_source_ids(repository)
    held, applied = promotion_ledger(repository)
    candidates: list[dict[str, Any]] = []
    blocked: list[dict[str, str]] = []
    # Archived records stay promotable. A resolved finding is both promotable
    # and archivable, so compaction can move it before a capped promotion run
    # reaches it; scanning only active records would lose it permanently.
    for path, record, body in iter_records(repository, include_archive=True):
        try:
            validate_record(record)
        except BrainError:
            continue
        if only is not None and record["id"] not in only:
            continue
        if record["status"] not in PROMOTABLE_STATES.get(record["type"], set()):
            continue
        content = promotion_content(record)
        supersedes = None
        if record["id"] in already:
            # Promoted once. A record revised since - a corrected consequence
            # - is promoted again, and its chunk supersedes the one promoted
            # before: excluding the record for good left the bank serving
            # the old conclusion, marked only "source changed", while the
            # corrected one never arrived.
            previous = applied.get(record["id"])
            if (
                record["id"] in held
                or previous is None
                or content is None
                or content == previous.get("content")
                or record["revision"] <= previous["source_records"][0]["revision"]
            ):
                continue
            memory_id = previous.get("destination_memory_id")
            if previous.get("review_mode") != "automatic":
                # What a person reviewed is not replaced without one.
                blocked.append({"record_id": record["id"], "reason": (
                    f"revised since {memory_id} was promoted after a person's review; "
                    "propose the revision for review")})
                continue
            status = _chunk_status(repository, memory_id)
            if status != "active":
                blocked.append({"record_id": record["id"], "reason": (
                    f"revised since it was promoted, and {memory_id} is {status or 'missing'}; "
                    "promote the revision by hand if it still holds")})
                continue
            supersedes = memory_id
        provenance = provenance_problems(record, content)
        if provenance:
            blocked.append({"record_id": record["id"], "reason": "; ".join(provenance)})
            continue
        reason = promotion_eligibility_error(
            repository, record, config, allow_changed_sources=allow_changed_sources
        )
        if reason is not None:
            blocked.append({"record_id": record["id"], "reason": reason})
            continue
        candidates.append({"record": record, "content": content, "supersedes": supersedes})
    return candidates, blocked


# A small, fixed stop list rather than corpus-adaptive terms. The adaptive
# vocabulary in the retrieval layer is derived from `memory-bank/local/`, which
# is git-ignored and disposable, and making the CONTENT of the tracked Memory
# Bank depend on it would let two machines on the same commit promote different
# sets. This list never moves, so the decision is reproducible.
_DUPLICATE_STOPWORDS = frozenset(
    """a an and are as at be but by for from has have if in into is it its of on
    or that the their then there these this to was were will with when where
    which while""".split()
)
BANK_DUPLICATE_RATIO = 0.6


def _duplicate_tokens(text: str) -> list[str]:
    # Digits are kept at any length. They are frequently the only thing that
    # distinguishes two findings - an error code, a table version, an ordinal -
    # and dropping them as "too short" made texts identical that a reader would
    # never confuse.
    return [
        token
        for token in re.findall(r"[^\W_]+", str(text).casefold())
        if (len(token) >= 2 or token.isdigit())
        and token not in _DUPLICATE_STOPWORDS
    ]


def _duplicate_shingles(text: str) -> set[tuple[str, ...]]:
    """Three-gram shingles, so word order counts and short texts still compare.

    Whole-set Jaccard over bare tokens was tried elsewhere in this repository
    and recorded as diluting below any usable threshold; shingles keep two
    texts that say the same thing in the same order close, and two that merely
    share vocabulary apart.
    """
    tokens = _duplicate_tokens(text)
    if not tokens:
        return set()
    if len(tokens) < 3:
        return {tuple(tokens)}
    return {tuple(tokens[index : index + 3]) for index in range(len(tokens) - 2)}


def text_similarity(left: str, right: str) -> float:
    """Jaccard overlap of two texts' shingles, 0.0 when either says nothing."""
    first = _duplicate_shingles(left)
    second = _duplicate_shingles(right)
    if not first or not second:
        return 0.0
    return len(first & second) / len(first | second)


def active_chunk_texts(repository: Path) -> list[tuple[str, str]]:
    """Every active chunk as (memory id, comparable text).

    The comparable text is the title plus the body, because that is where a
    promoted conclusion actually lands. The chunk's frontmatter `sources` is
    not usable for this: promotion writes the SOURCE RECORD's own file path
    there, so two chunks promoted from two records can never share one, and
    compaction rewrites them again when it archives the record.
    """
    texts: list[tuple[str, str]] = []
    chunks = repository / "memory-bank" / "chunks"
    if not chunks.is_dir():
        return texts
    for path in sorted(chunks.glob("*.md")):
        if BANK_FILENAME_PATTERN.fullmatch(path.name) is None:
            continue
        try:
            metadata, body = _parse_chunk(path)
        except BrainError:
            continue
        if metadata.get("status") != "active":
            continue
        memory_id = metadata.get("id")
        if not isinstance(memory_id, str):
            continue
        texts.append((memory_id, f"{metadata.get('title', '')}\n{body}"))
    return texts


def near_duplicate_chunk(
    content: str, chunks: list[tuple[str, str]], threshold: float
) -> Optional[str]:
    """The closest active chunk this content would duplicate, if any."""
    best_id: Optional[str] = None
    best_score = threshold
    for memory_id, text in chunks:
        score = text_similarity(content, text)
        if score >= best_score:
            best_id, best_score = memory_id, score
    return best_id


def bank_duplicate_ratio(config: dict[str, Any]) -> float:
    value = config.get("bank_duplicate_ratio", BANK_DUPLICATE_RATIO)
    try:
        ratio = float(value)
    except (TypeError, ValueError):
        return BANK_DUPLICATE_RATIO
    return ratio if 0.0 < ratio <= 1.0 else BANK_DUPLICATE_RATIO


PROMOTION_RETRY_SECONDS = 30


def _promotion_retry_file(repository: Path, owner: str) -> Path:
    directory = brain_root(repository) / "local"
    for parent in (directory, directory.parent):
        if os.path.lexists(parent):
            image = parent.lstat()
            if not stat.S_ISDIR(image.st_mode) or linked_reparse(image):
                raise BrainError("Automatic promotion retry state is unavailable")
    key = hashlib.sha256(owner.encode("utf-8")).hexdigest()
    destination = directory / ("promotion-retry-" + key + ".json")
    if os.path.lexists(destination):
        image = destination.lstat()
        if not stat.S_ISREG(image.st_mode) or image.st_nlink != 1 or linked_reparse(image):
            raise BrainError("Automatic promotion retry state is unavailable")
    return destination


def _promotion_retry_state(repository: Path, owner: str) -> Optional[dict[str, Any]]:
    try:
        destination = _promotion_retry_file(repository, owner)
        with destination.open("rb") as stream:
            raw = stream.read(513)
        value = json.loads(raw) if len(raw) <= 512 else None
        if (isinstance(value, dict)
                and set(value) in ({"profile", "next_attempt"}, {"profile", "next_attempt", "after_record"})
                and value["profile"] == "automatic-promotion-retry-v1"
                and type(value["next_attempt"]) is int and value["next_attempt"] >= 0
                and (value.get("after_record") is None or is_uuid4(value["after_record"]))):
            return value
    except (OSError, BrainError, ValueError, UnicodeError, RecursionError):
        pass
    return None


def _promotion_retry_due(repository: Path, owner: str) -> bool:
    value = _promotion_retry_state(repository, owner)
    return value is not None and value["next_attempt"] <= time.time()


def retry_pending_auto_promotion(repository: Path, *, owner: str) -> dict[str, Any]:
    """Retry one due batch; an ordinary clean turn reads only a local marker."""
    empty = {"enabled": False, "promoted": [], "failed": [],
             "blocked": [], "closed": [], "skipped": 0}
    if not _promotion_retry_due(repository, owner):
        return empty
    with mutation_lock(repository):
        # Another Stop hook may have drained or rescheduled it while we waited.
        if not _promotion_retry_due(repository, owner):
            return empty
        return auto_promote(repository, owner=owner, limit=1, max_attempts=1)


def auto_promote(
    repository: Path,
    *,
    owner: str,
    limit: int = 5,
    only: Optional[set[str]] = None,
    max_attempts: Optional[int] = None,
) -> dict[str, Any]:
    """Promote resolved, verified knowledge into durable memory without review.

    Every applied chunk is tagged `auto-promoted` and its promotion names no
    reviewer, so the absence of human approval stays visible in both stores.
    `only` restricts the run to the named records - the resolving update
    promotes its own record at once instead of waiting for a turn boundary.
    A record revised since its promotion is promoted again, and the new
    chunk supersedes the old one. A stalled promotion that can never apply
    is withdrawn (`closed`) rather than retried for good. `limit` caps new
    nonduplicate proposal attempts (including failures), not duplicate scans
    or the separate stalled-promotion retries. Clean-turn recovery supplies
    `max_attempts` to bound both stalled and new attempts together.
    """
    config = load_config(repository)
    if not config.get("automatic_promotion"):
        return {
            "enabled": False, "promoted": [], "failed": [],
            "blocked": [], "closed": [], "skipped": 0,
        }
    if max_attempts is not None and (type(max_attempts) is not int or max_attempts < 1):
        raise BrainError("Automatic promotion attempt limit must be positive")
    promoted: list[dict[str, Any]] = []
    failed: list[dict[str, str]] = []
    closed: list[dict[str, str]] = []
    # Choosing a record, proposing it and applying the proposal are one step.
    # Two runs at once - a Stop hook and a Harness save - each chose the same
    # record and each wrote a proposal; one chunk was written and the other
    # proposal stayed `reviewed`, refused on every retry, or wrote a second
    # chunk of the same record on a later day.
    with mutation_lock(repository):
        retry_file = None
        previous_retry = _promotion_retry_state(repository, owner) or {}
        retry_value = {"profile": "automatic-promotion-retry-v1",
                       "next_attempt": int(time.time()) + PROMOTION_RETRY_SECONDS,
                       "after_record": previous_retry.get("after_record")}
        try:
            retry_file = _promotion_retry_file(repository, owner)
            make_private_directory(retry_file.parent)
            # Persist before any attempt: interruption leaves a retry, with a
            # delay already in place instead of a scan on every clean turn.
            atomic_json(retry_file, retry_value)
        except (OSError, BrainError, ValueError, OverflowError):
            retry_file = None
        if max_attempts is not None and retry_file is None:
            return {"enabled": False, "promoted": [], "failed": [],
                    "blocked": [], "closed": [], "skipped": 0, "retry_unavailable": True}
        attempts = deferred_stalled = 0
        _, applied = promotion_ledger(repository)
        work_only, deferred_records = only, 0
        retry_blocked = []
        if max_attempts is not None:
            # Rotate across stalled AND fresh records. A single unavailable
            # citation must not occupy every bounded recovery batch forever.
            fresh, retry_blocked = promotable_records(repository, config, allow_changed_sources=True, only=only)
            bank = active_chunk_texts(repository)
            eligible = []
            for candidate in fresh:
                record = candidate['record']
                duplicate = near_duplicate_chunk(
                    f"{record['title']}\n{candidate['content']}",
                    [item for item in bank if item[0] != candidate.get('supersedes')], bank_duplicate_ratio(config))
                if duplicate is None:
                    eligible.append(candidate)
                else:
                    retry_blocked.append({'record_id': record['id'],
                        'reason': f'near-duplicate of {duplicate}; merge or supersede first'})
            fresh = eligible
            identities = sorted({item['record']['id'] for item in fresh} | {
                proposal['source_records'][0]['id']
                for _, proposal in stalled_automatic_promotions(repository)
                if only is None or proposal['source_records'][0]['id'] in only})
            after = previous_retry.get('after_record') or ''
            ordered = [key for key in identities if key > after] + [key for key in identities if key <= after]
            work_only = set(ordered[:max_attempts])
            deferred_records = len(identities) - len(work_only)
            if work_only and retry_file is not None:
                retry_value['after_record'] = ordered[len(work_only) - 1]
                try:
                    atomic_json(retry_file, retry_value)
                except (OSError, BrainError):
                    return {"enabled": False, "promoted": [], "failed": [],
                            "blocked": [], "closed": [], "skipped": 0, "retry_unavailable": True}
        # Retry a promotion that stalled after its record was created, in
        # place: creating a second one would leave the first orphaned. One
        # that can never apply is withdrawn instead.
        for promotion_id, proposal in stalled_automatic_promotions(repository):
            source = proposal["source_records"][0]
            if work_only is not None and source["id"] not in work_only:
                continue
            if max_attempts is not None and attempts >= max_attempts:
                deferred_stalled += 1
                continue
            attempts += 1
            try:
                obsolete = obsolete_promotion(repository, proposal, applied)
                if obsolete is not None:
                    withdraw_promotion(repository, promotion_id, obsolete)
                    closed.append({"record_id": source["id"], "promotion_id": promotion_id,
                                   "reason": obsolete})
                    continue
                if proposal["status"] == "proposed":
                    auto_review_promotion(repository, promotion_id)
                applied_proposal = apply_promotion(repository, promotion_id)
            except BrainError as error:
                failed.append({"record_id": source["id"], "reason": str(error)})
                continue
            applied[source["id"]] = applied_proposal
            promoted.append(
                {
                    "record_id": source["id"],
                    "type": source["type"],
                    "memory_id": applied_proposal["destination_memory_id"],
                }
            )
        # A record whose promotion is still open is retried in place on the
        # next run, never proposed a second time.
        pending = {
            proposal["source_records"][0]["id"]
            for _, proposal in stalled_automatic_promotions(repository)
        }
        candidates, blocked = promotable_records(
            repository, config, allow_changed_sources=True, only=work_only
        )
        blocked = list({(item['record_id'], item['reason']): item
                        for item in [*retry_blocked, *blocked]}.values())
        candidates = [
            candidate for candidate in candidates
            if candidate["record"]["id"] not in pending
        ]
        # Re-read on every iteration rather than once before the loop: a
        # single flush promotes up to `limit` records, so the batch case - one
        # review producing several findings that say the same thing - is
        # exactly the one a pre-loop snapshot would miss.
        threshold = bank_duplicate_ratio(config)
        existing = active_chunk_texts(repository)
        # Duplicate rejections must not consume all admission slots forever.
        # Preserve the existing slice's argument semantics, but use its length
        # to cap new proposal attempts, not the duplicate-bearing prefix.
        admission_limit = len(candidates[:limit])
        if max_attempts is not None:
            admission_limit = min(admission_limit, max(0, max_attempts - attempts))
        admitted = considered = 0
        for candidate in candidates:
            if admitted >= admission_limit:
                break
            considered += 1
            record = candidate["record"]
            # A revision is measured against everything but the chunk it
            # replaces, which it resembles by design.
            replaces = candidate.get("supersedes")
            others = [item for item in existing if item[0] != replaces]
            duplicate = near_duplicate_chunk(
                f"{record['title']}\n{candidate['content']}", others, threshold
            )
            if duplicate is not None:
                # The policy "update an existing chunk instead of creating a
                # near duplicate" had no executor on the automatic path:
                # promotion never read the bank it writes into. Naming the
                # chunk turns the block into an instruction.
                blocked.append(
                    {
                        "record_id": record["id"],
                        "reason": (
                            f"near-duplicate of {duplicate}; merge or supersede first"
                        ),
                    }
                )
                continue
            # Failed attempts also spend admission: systematic errors must
            # not create an unbounded batch of proposals or writes.
            admitted += 1
            attempts += 1
            try:
                proposal = create_promotion(
                    repository,
                    [record["id"]],
                    record["title"],
                    candidate["content"],
                    proposer=owner,
                    review_mode="automatic",
                )
                auto_review_promotion(repository, proposal["id"])
                applied_proposal = apply_promotion(repository, proposal["id"])
            except BrainError as error:
                # One unpromotable record must not stop the rest, but a
                # systematic failure - an absent Memory Bank, say - has to
                # stay visible rather than looking like "nothing was worth
                # promoting".
                failed.append({"record_id": record["id"], "reason": str(error)})
                continue
            promoted.append(
                {
                    "record_id": record["id"],
                    "type": record["type"],
                    "memory_id": applied_proposal["destination_memory_id"],
                }
            )
            # The chunk just written is a duplicate candidate for the rest of
            # this same batch; the one it superseded no longer is.
            existing = others + [
                (
                    applied_proposal["destination_memory_id"],
                    f"{record['title']}\n{candidate['content']}",
                )
            ]
        skipped = len(candidates) - considered + deferred_stalled + deferred_records
        if retry_file is not None and only is None and not failed and not skipped:
            try:
                # Recheck the local destination before removing it. A scoped
                # success cannot establish that earlier unrelated work drained.
                _promotion_retry_file(repository, owner).unlink(missing_ok=True)
            except (OSError, BrainError):
                retry_file = None
    result = {
        "enabled": True,
        "promoted": promoted,
        "failed": failed,
        "blocked": blocked,
        "closed": closed,
        "skipped": skipped,
    }
    if retry_file is None and (failed or skipped):
        result["retry_unavailable"] = True
    return result


def archivable_records(repository: Path) -> int:
    """Count active records that compaction would move."""
    total = 0
    for _, record, _ in iter_records(repository):
        if (
            record["status"] in LIFECYCLES[record["type"]]["terminal"]
            or record.get("superseded_by") is not None
        ) and not _retain_consolidated(record):
            total += 1
    return total


def auto_consolidate(repository: Path, *, owner: str = "local") -> dict[str, Any]:
    """Collapse completed events on the turn boundary, without a user command.

    The same boundary that promotes durable memory and archives terminal
    records first turns a ticket's completed-event series into one finding,
    and lets events leave the diary once a chunk carries their finding. Off
    unless `automatic_consolidation` is set. A failure is reported, never
    raised: the checkpoint that already landed must not be presented as a
    failed turn because maintenance could not run.
    """
    config = load_config(repository)
    empty = {
        "enabled": False, "created": 0, "updated": 0, "superseded": 0,
        "blocked": 0, "tickets": 0, "error": None,
    }
    if not config.get("automatic_consolidation"):
        return empty
    try:
        result = consolidate(repository, apply=True, owner=owner)
    except (BrainError, OSError, ValueError) as error:
        return {**empty, "enabled": True, "error": str(error)}
    return {
        "enabled": True,
        "created": result["created"],
        "updated": result["updated"],
        "superseded": result["superseded"],
        "blocked": result["blocked"],
        "tickets": len(result["tickets"]),
        "error": None,
    }


def auto_compact(repository: Path, *, owner: str = "local") -> dict[str, Any]:
    """Archive terminal records once enough of them have accumulated.

    Compaction moves Git-tracked files and rebuilds indexes, so it runs in
    batches rather than on every turn: archiving one record at a time would
    churn history for no benefit. A failure is reported rather than raised, so
    a checkpoint that already landed is not reported as a failed turn.
    """
    config = load_config(repository)
    if not config.get("automatic_compaction"):
        return {"enabled": False, "moved": 0, "pending": 0, "error": None}
    threshold = config.get("compaction_threshold", 5)
    if not isinstance(threshold, int) or threshold < 1:
        return {
            "enabled": True, "moved": 0, "pending": 0,
            "error": "compaction_threshold must be a positive integer",
        }
    pending = archivable_records(repository)
    if pending < threshold:
        return {"enabled": True, "moved": 0, "pending": pending, "error": None}
    try:
        moved = compact(repository)["moved"]
    except BrainError as error:
        return {"enabled": True, "moved": 0, "pending": pending, "error": str(error)}
    return {"enabled": True, "moved": moved, "pending": 0, "error": None}


# INDEX.md is a derived view over chunk frontmatter, regenerated in full by
# `render_bank_index` rather than appended to. The preamble below is used only
# when the index is missing or unrecognizable; an existing preamble (anything
# above the table header) is preserved verbatim, so per-edition prose survives
# reindexing.
BANK_INDEX_PREAMBLE = (
    "# Memory Index\n\n"
    "Read [README.md](README.md) before using this index. Load only active "
    "chunks relevant to the current task, then verify them against their "
    "cited sources.\n\n"
)
BANK_INDEX_HEADER = (
    "| ID | Title | Type | Scope | Tags | Status | Last Verified | File |\n"
    "| --- | --- | --- | --- | --- | --- | --- | --- |\n"
)


def _bank_index_sort_key(memory_id: str) -> tuple[int, str, str]:
    """Stable index order: numeric legacy IDs first, then date-based IDs.

    Legacy `MEM-0001` identifiers sort numerically (so `MEM-10000` follows
    `MEM-9999`); date-based `MEM-YYYYMMDD-xxxxxxxx` identifiers sort
    lexicographically, which is chronological up to the UUID fragment.
    """
    legacy = re.fullmatch(r"MEM-(\d{4,})", memory_id)
    if legacy:
        return (0, f"{int(legacy.group(1)):020d}", memory_id)
    return (1, memory_id, memory_id)


def render_bank_index(bank: Path) -> tuple[str, int]:
    """Render INDEX.md deterministically from chunk frontmatter.

    Returns the full index text and the number of chunk rows. Because every
    row is derived from a chunk file, merging two branches that each promoted
    knowledge reduces to a union of chunk files followed by one reindex —
    there is no hand-maintained table left to conflict on.
    """
    chunks_dir = bank / "chunks"
    rows: dict[str, str] = {}
    if chunks_dir.is_dir():
        for path in sorted(chunks_dir.iterdir()):
            if (
                path.is_symlink()
                or not path.is_file()
                or BANK_FILENAME_PATTERN.fullmatch(path.name) is None
            ):
                continue
            metadata, _ = parse_markdown_record(path)
            memory_id = metadata.get("id")
            if not isinstance(memory_id, str) or not memory_id:
                raise BrainError(f"{path}: chunk frontmatter has no id")
            if memory_id in rows:
                raise BrainError(f"Duplicate Memory Bank chunk ID: {memory_id}")
            try:
                rows[memory_id] = (
                    f"| {memory_id} | {metadata['title']} | {metadata['type']} | "
                    f"{', '.join(metadata['scope'])} | {', '.join(metadata['tags'])} | "
                    f"{metadata['status']} | {metadata['last_verified']} | "
                    f"chunks/{path.name} |\n"
                )
            except (KeyError, TypeError) as error:
                raise BrainError(
                    f"{path}: chunk frontmatter is incomplete for indexing"
                ) from error
    preamble = BANK_INDEX_PREAMBLE
    index_path = bank / "INDEX.md"
    if index_path.is_file():
        head, header, _ = index_path.read_text(encoding="utf-8").partition(
            "| ID | Title |"
        )
        if header:
            preamble = head
    body = "".join(
        rows[memory_id] for memory_id in sorted(rows, key=_bank_index_sort_key)
    )
    return preamble + BANK_INDEX_HEADER + body, len(rows)


def _parse_chunk(path: Path) -> tuple[dict[str, Any], str]:
    """A chunk's frontmatter and its body, kept apart so the body is untouched.

    Deliberately not `parse_markdown_record` / `render_markdown_record`: those
    render with `sort_keys=True` and re-expand every list, so a round trip
    reorders frontmatter a human wrote and rewrites lines the retire never
    meant to change. `json.loads` preserves key order, so re-dumping the same
    mapping keeps the file recognisable as the one that was edited.
    """
    text = path.read_text(encoding="utf-8")
    if not text.startswith("---\n"):
        raise BrainError(f"Memory Bank chunk has no frontmatter: {path.name}")
    try:
        raw, body = text[4:].split("\n---\n", 1)
    except ValueError as error:
        raise BrainError(
            f"Memory Bank chunk frontmatter is unterminated: {path.name}"
        ) from error
    try:
        metadata = json.loads(raw)
    except json.JSONDecodeError as error:
        raise BrainError(
            f"Memory Bank chunk frontmatter is not valid JSON: {path.name}"
        ) from error
    if not isinstance(metadata, dict):
        raise BrainError(f"Memory Bank chunk frontmatter is not an object: {path.name}")
    return metadata, body


def _render_chunk(metadata: dict[str, Any], body: str) -> str:
    return f"---\n{json.dumps(metadata, indent=2)}\n---\n{body}"


def _find_chunk(bank: Path, memory_id: str) -> tuple[Path, dict[str, Any], str]:
    chunks = bank / "chunks"
    if chunks.is_dir():
        for path in sorted(chunks.glob("*.md")):
            match = BANK_FILENAME_PATTERN.fullmatch(path.name)
            if match is None or match.group(1) != memory_id:
                continue
            metadata, body = _parse_chunk(path)
            return path, metadata, body
    raise BrainError(f"Memory Bank chunk not found: {memory_id}")


def retire_chunk(
    repository: Path,
    memory_id: str,
    *,
    valid_to: str,
    superseded_by: Optional[str] = None,
    reason: Optional[str] = None,
) -> dict[str, Any]:
    """Close a chunk's period, and its replacement link, in one transaction.

    Retire was designed but only ever documented: the sole code that writes
    `valid_to` is promotion, and it always writes null, so the operation was
    an editing convention - hand-edit both sides of a supersession, then
    regenerate the index, then validate. Between the first edit and the last
    the bank is invalid, and the word `valid_to` appears in no skill, so an
    agent following policy honestly sets a status and never writes a date.

    `valid_to` says when the knowledge stopped being true; `superseded_by`
    says what replaced it. With a successor the chunk becomes `superseded` and
    the successor's `supersedes` list gains it in the same write; without one
    it becomes `archived`, which is the honest status for knowledge that
    simply ceased. The body is never touched.

    Every write is taken under the mutation lock against a snapshot, so an
    interruption between the two sides of a link leaves the bank exactly as it
    was rather than half-retired.
    """
    try:
        closing = date.fromisoformat(valid_to)
    except (TypeError, ValueError) as error:
        raise BrainError("valid_to must be an ISO date (YYYY-MM-DD)") from error
    if superseded_by is not None and superseded_by == memory_id:
        raise BrainError("A chunk cannot supersede itself")
    bank = repository / "memory-bank"
    index_path = bank / "INDEX.md"
    with mutation_lock(repository):
        path, metadata, body = _find_chunk(bank, memory_id)
        successor_path: Optional[Path] = None
        successor: Optional[dict[str, Any]] = None
        successor_body = ""
        if superseded_by is not None:
            # Resolved before any write: letting the whole-bank validation
            # discover a missing successor would roll back with a message
            # about a broken link rather than about the argument that caused
            # it.
            successor_path, successor, successor_body = _find_chunk(
                bank, superseded_by
            )
        before = validate_bank(bank)
        snapshot = snapshot_files(
            [p for p in (path, successor_path, index_path) if p is not None]
        )
        try:
            metadata["status"] = "superseded" if superseded_by else "archived"
            # Only a superseded chunk may carry a replacement link, and every
            # superseded chunk must.
            metadata["superseded_by"] = superseded_by
            metadata["valid_to"] = closing.isoformat()
            atomic_write(path, _render_chunk(metadata, body))
            if successor_path is not None and successor is not None:
                supersedes = list(successor.get("supersedes") or [])
                if memory_id not in supersedes:
                    supersedes.append(memory_id)
                successor["supersedes"] = supersedes
                atomic_write(
                    successor_path, _render_chunk(successor, successor_body)
                )
            index_text, chunk_count = render_bank_index(bank)
            atomic_write(index_path, index_text)
            errors = bank_write_errors(
                repository,
                before,
                tuple(p for p in (path, successor_path) if p is not None),
            )
            if errors:
                raise BrainError(
                    "Memory Bank retire failed validation: " + "; ".join(errors)
                )
        except Exception:
            restore_files(snapshot)
            raise
    return {
        "id": memory_id,
        "status": metadata["status"],
        "valid_to": metadata["valid_to"],
        "superseded_by": superseded_by,
        # Recorded for the operator's transcript only: the chunk schema is a
        # closed key set, so a reason written into the frontmatter would fail
        # validation and roll the whole retire back.
        "reason": reason,
        "chunk": path.relative_to(repository).as_posix(),
        "index": index_path.relative_to(repository).as_posix(),
        "chunks": chunk_count,
    }


def audit_bank(repository: Path, *, provenance: bool = False) -> dict[str, Any]:
    """Report chunks whose citations moved on, and chunks overdue for review.

    Read-only and dependency-free. Answers the question the digests exist to
    make answerable: which durable knowledge is still standing on the file it
    was drawn from.
    """
    bank = repository / "memory-bank"
    chunks = bank / "chunks"
    today = datetime.now(timezone.utc).date()
    changed: list[dict[str, Any]] = []
    overdue: list[dict[str, Any]] = []
    undigested: list[str] = []
    auto_promoted: list[dict[str, Any]] = []
    external_sources: list[dict[str, Any]] = []
    instructions: list[str] = []
    total = 0
    if chunks.is_dir():
        for path in sorted(chunks.glob("*.md")):
            if BANK_FILENAME_PATTERN.fullmatch(path.name) is None:
                continue
            try:
                metadata, body = _parse_chunk(path)
            except BrainError:
                continue
            total += 1
            relative = path.relative_to(repository).as_posix()
            if metadata.get("status") != "active":
                continue
            tags = metadata.get("tags") if isinstance(metadata.get("tags"), list) else []
            if provenance and "auto-promoted" in tags:
                auto_promoted.append(
                    {
                        "chunk": relative,
                        "id": metadata.get("id"),
                        "created": metadata.get("created"),
                        "last_verified": metadata.get("last_verified"),
                    }
                )
            sources = metadata.get("sources") if isinstance(metadata.get("sources"), list) else []
            urls = [
                source for source in sources
                if isinstance(source, str) and source.startswith(("https://", "http://"))
            ]
            if provenance and urls:
                external_sources.append(
                    {"chunk": relative, "id": metadata.get("id"), "sources": urls}
                )
            if provenance and instruction_like(metadata.get("title"), body):
                instructions.append(relative)
            digests = metadata.get("source_digests")
            if not isinstance(digests, list) or not digests:
                if digestible_sources(metadata.get("sources")):
                    undigested.append(relative)
            else:
                for entry in digests:
                    if not isinstance(entry, dict):
                        continue
                    try:
                        current = fingerprint(repository, entry.get("path", ""))
                    except BrainError:
                        changed.append(
                            {
                                "chunk": relative,
                                "id": metadata.get("id"),
                                "source": entry.get("path"),
                                "state": "missing",
                            }
                        )
                        continue
                    if current["sha256"] != entry.get("sha256"):
                        changed.append(
                            {
                                "chunk": relative,
                                "id": metadata.get("id"),
                                "source": entry.get("path"),
                                "state": "changed",
                            }
                        )
            review_after = metadata.get("review_after")
            if isinstance(review_after, str):
                try:
                    if date.fromisoformat(review_after) < today:
                        overdue.append(
                            {
                                "chunk": relative,
                                "id": metadata.get("id"),
                                "review_after": review_after,
                            }
                        )
                except ValueError:
                    continue
    duplicates: list[dict[str, Any]] = []
    texts = active_chunk_texts(repository)
    threshold = bank_duplicate_ratio(load_config(repository))
    for index, (left_id, left_text) in enumerate(texts):
        for right_id, right_text in texts[index + 1 :]:
            score = text_similarity(left_text, right_text)
            if score >= threshold:
                duplicates.append(
                    {
                        "left": left_id,
                        "right": right_id,
                        "similarity": round(score, 3),
                    }
                )
    result: dict[str, Any] = {
        "chunks": total,
        "source_changed": changed,
        "overdue_review": overdue,
        "undigested": undigested,
        # Pairs a human decides on: update the survivor, or close the older
        # one with `bank-retire`. The engine never merges chunks by itself.
        "duplicates": duplicates,
    }
    if provenance:
        result["provenance"] = {
            "auto_promoted": auto_promoted,
            "external_sources": external_sources,
            "instruction_like": instructions,
        }
    return result


def reverify_chunk(
    repository: Path, memory_id: str, *, review_after: Optional[str] = None
) -> dict[str, Any]:
    """Re-attest a chunk against its sources as they are now.

    Without this the digests would be a one-way ratchet: the first time a
    cited file changed, the chunk would leave retrieval and stay out, because
    nothing but promotion has ever written chunk frontmatter and hand-editing
    is what the rest of this system exists to avoid. Calling it is an
    assertion by whoever ran it that they re-read the sources and the chunk
    still holds - the command records that, it does not decide it.
    """
    bank = repository / "memory-bank"
    index_path = bank / "INDEX.md"
    today = datetime.now(timezone.utc).date()
    if review_after is not None:
        try:
            next_review = date.fromisoformat(review_after)
        except (TypeError, ValueError) as error:
            raise BrainError("review_after must be an ISO date (YYYY-MM-DD)") from error
    else:
        next_review = today + timedelta(days=365)
    with mutation_lock(repository):
        path, metadata, body = _find_chunk(bank, memory_id)
        if metadata.get("status") != "active":
            raise BrainError(
                f"Only an active chunk can be re-verified: {memory_id} is "
                f"{metadata.get('status')}"
            )
        before = validate_bank(bank)
        snapshot = snapshot_files([path, index_path])
        try:
            metadata["last_verified"] = today.isoformat()
            metadata["review_after"] = next_review.isoformat()
            metadata["source_digests"] = chunk_source_digests(
                repository, metadata.get("sources")
            )
            atomic_write(path, _render_chunk(metadata, body))
            index_text, chunk_count = render_bank_index(bank)
            atomic_write(index_path, index_text)
            # Re-verifying is an attestation, so this chunk meets the full
            # contract; another chunk's overdue review is not its problem.
            errors = bank_write_errors(repository, before, (path,))
            if errors:
                raise BrainError(
                    "Memory Bank re-verify failed validation: " + "; ".join(errors)
                )
        except Exception:
            restore_files(snapshot)
            raise
    return {
        "id": memory_id,
        "last_verified": metadata["last_verified"],
        "review_after": metadata["review_after"],
        "source_digests": metadata["source_digests"],
        "chunk": path.relative_to(repository).as_posix(),
        "chunks": chunk_count,
    }


def reindex_bank(repository: Path) -> dict[str, Any]:
    """Regenerate memory-bank/INDEX.md from chunk frontmatter.

    Idempotent: rerunning against unchanged chunks rewrites nothing, and a
    deleted index is reconstructed in full. This replaces every manual
    "update INDEX.md and increment .memory-counter" step; the retired
    `.memory-counter` file is neither read nor written.
    """
    bank = repository / "memory-bank"
    index_path = bank / "INDEX.md"
    with mutation_lock(repository):
        rendered, chunks = render_bank_index(bank)
        previous = (
            index_path.read_text(encoding="utf-8") if index_path.is_file() else None
        )
        changed = previous != rendered
        if changed:
            atomic_write(index_path, rendered)
    return {
        "index": "memory-bank/INDEX.md",
        "chunks": chunks,
        "changed": changed,
    }


def apply_promotion(repository: Path, promotion_id: str) -> dict[str, Any]:
    promotion_path = brain_root(repository) / "control" / "promotions" / f"{promotion_id}.json"
    bank = repository / "memory-bank"
    index_path = bank / "INDEX.md"
    with mutation_lock(repository):
        try:
            proposal = json.loads(promotion_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise BrainError(f"Promotion not found or invalid: {promotion_id}") from error
        validate_promotion_record(
            repository, proposal, expected_id=promotion_id
        )
        automatic = proposal.get("review_mode") == "automatic"
        if proposal.get("status") != "reviewed" or (
            not proposal.get("reviewer") and not automatic
        ):
            raise BrainError("Promotion requires an approved human review before apply")
        config = load_config(repository)
        # What the promoted records themselves cited, carried through to the
        # chunk. Without this the citation chain breaks at promotion: the
        # chunk names the record, the record names the document, and nothing
        # traverses two hops — so `links` and `retrieve --path` on the
        # document reach the record and never the knowledge derived from it.
        # Measured on a real project: two chunks promoted from findings about
        # one design document shared no source at all.
        #
        # Only `sources`, never `files`. A record's `files` is Git churn in
        # the Git-toplevel frame — phpunit cache, vendored JavaScript — and
        # merging it here would put code paths under `chunk_source_digests`,
        # where the next edit evicts the chunk as `source-changed`.
        inherited: list[str] = []
        # The digests each cited file had when the record was verified. A
        # chunk digests what its knowledge was checked against, not what the
        # files hold today, or promotion would launder an edit that happened
        # after verification into a "fresh" citation.
        verified_digests: dict[str, dict[str, str]] = {}
        for source in proposal["source_records"]:
            current_path, current, _ = find_record(
                repository, source["id"], include_archive=True
            )
            current_relative = current_path.relative_to(repository).as_posix()
            if (
                current["id"] != source["id"]
                or current["type"] != source["type"]
                or current_relative != source["path"]
                or current["revision"] != source["revision"]
            ):
                raise BrainError(f"Promotion source revision changed: {source['id']}")
            ineligible = promotion_eligibility_error(
                repository, current, config, allow_changed_sources=automatic
            )
            if ineligible is not None:
                raise BrainError(f"Promotion source is not eligible: {ineligible}")
            for item in current.get("source_fingerprints") or []:
                if isinstance(item, dict) and isinstance(item.get("path"), str):
                    verified_digests[item["path"]] = item
            for cited in current.get("sources") or []:
                if not isinstance(cited, str):
                    continue
                # A citation whose file is already gone is not carried: the
                # chunk would fail `validate_metadata` at birth and block a
                # promotion that has nothing to do with that file.
                if workspace_roots.resolve(repository, cited).is_file():
                    inherited.append(cited)
        today = datetime.now(timezone.utc).date()
        # Conflict-free identifier: the promotion date plus eight hex
        # characters of the source record's UUID. Two machines or branches
        # promoting concurrently cannot collide the way the retired shared
        # `.memory-counter` file did, so a later merge is a plain union of
        # chunk files. The legacy counter file may remain on disk; it is
        # neither read nor written here.
        source_uuid = proposal["source_records"][0]["id"]
        suffix = source_uuid.replace("-", "")[:8]
        # An automatic promotion of a record the bank already learned from,
        # at an earlier revision, is that knowledge revised: its chunk
        # supersedes the earlier one in this same write. Its identifier hashes
        # the revision in, since the earlier chunk may carry today's date too.
        predecessor: Optional[tuple[Path, dict[str, Any], str]] = None
        if automatic and len(proposal["source_records"]) == 1:
            _, applied = promotion_ledger(repository)
            previous = applied.get(source_uuid)
            if previous is not None and previous.get("id") != proposal["id"]:
                revision = proposal["source_records"][0]["revision"]
                if previous["source_records"][0]["revision"] >= revision:
                    raise BrainError(
                        "Promotion source was already promoted as "
                        f"{previous.get('destination_memory_id')}"
                    )
                predecessor = _find_chunk(bank, str(previous.get("destination_memory_id")))
                if predecessor[1].get("status") != "active":
                    raise BrainError(
                        f"The chunk this promotion revises is {predecessor[1].get('status')}: "
                        f"{previous.get('destination_memory_id')}"
                    )
                suffix = hashlib.sha256(f"{source_uuid}#{revision}".encode("utf-8")).hexdigest()[:8]
        memory_id = f"MEM-{today.strftime('%Y%m%d')}-{suffix}"
        slug = re.sub(r"[^a-z0-9]+", "-", proposal["title"].lower()).strip("-") or "promoted"
        destination = bank / "chunks" / f"{memory_id}-{slug}.md"
        if destination.exists():
            raise BrainError(
                "Promotion destination already exists: "
                + bank_display_path(destination, bank)
            )
        if automatic:
            # Stalled proposals enter apply without fresh-candidate admission.
            # A prior interrupted apply may already have written this claim
            # under another date. Refuse duplication without adopting or
            # rewriting an unproven chunk; a valid revision may replace its
            # own recorded predecessor as before.
            replaces = predecessor[1]["id"] if predecessor else None
            duplicate = near_duplicate_chunk(
                f"{proposal['title']}\n{proposal['content']}",
                [item for item in active_chunk_texts(repository) if item[0] != replaces],
                bank_duplicate_ratio(config),
            )
            if duplicate is not None:
                raise BrainError(f"near-duplicate of {duplicate}; merge or supersede first")
        # Tag the chunk so the bank itself shows which knowledge no human
        # approved; a reader must not have to open the promotion to find out.
        tags = ["project-brain", "promoted"] + (["auto-promoted"] if automatic else [])
        # A claim only its own agent ever checked stays marked as such in
        # the bank, so a later reader can weigh it.
        attested = []
        for item in proposal["source_records"]:
            try:
                # Archived sources too: promotion accepts an archived record,
                # and its provenance must not drop out on the way.
                attested.append(record_attestation(
                    find_record(repository, item["id"], include_archive=True)[1]))
            except BrainError:
                attested.append("")
        if "agent" in attested:
            tags.append("agent-attested")
        metadata = {
            "id": memory_id, "title": proposal["title"],
            # Derived from what was promoted, not asserted. `decision` remains
            # the fallback for a source type this map has not learned yet.
            "type": PROMOTION_TYPE_BY_RECORD.get(
                proposal["source_records"][0]["type"], "decision"
            ),
            "status": "active",
            "scope": ["application"], "tags": tags,
            "created": today.isoformat(), "last_verified": today.isoformat(),
            "review_after": (today + timedelta(days=365)).isoformat(),
            # The records that were promoted, plus what those records cited.
            # Both are durable citations under the same freshness discipline:
            # an edit to either re-opens the chunk for `bank-reverify`.
            "sources": sorted(
                dict.fromkeys(
                    [item["path"] for item in proposal["source_records"]] + inherited
                )
            ),
            "supersedes": [predecessor[1]["id"]] if predecessor else [],
            "superseded_by": None,
            # Open-ended validity: the knowledge holds from today until a
            # successor closes it with a valid_to, rather than being deleted.
            "valid_from": today.isoformat(), "valid_to": None,
        }
        # Digest what the chunk itself cites. A Brain record is checked against
        # its sources on every retrieval and lives for days; a chunk lives a
        # year or more and carried no such check at all, so a promoted
        # conclusion outlived any means of noticing that the record it was
        # drawn from had moved on.
        metadata["source_digests"] = [
            {"path": entry["path"], "sha256": verified_digests[entry["path"]]["sha256"]}
            if entry["path"] in verified_digests
            and isinstance(verified_digests[entry["path"]].get("sha256"), str)
            else entry
            for entry in chunk_source_digests(repository, metadata["sources"])
        ]
        heading = f"# {proposal['title']}"
        content = proposal["content"].strip()
        chunk = render_markdown_record(
            metadata,
            content if content.startswith(heading) else f"{heading}\n\n{content}",
        )
        before = validate_bank(bank)
        snapshot = snapshot_files(
            [destination, index_path, promotion_path] + ([predecessor[0]] if predecessor else [])
        )
        try:
            atomic_write(destination, chunk)
            if predecessor is not None:
                # Closed the way `retire_chunk` closes one: superseded, linked
                # to its successor, valid until today. The body is untouched.
                predecessor[1]["status"] = "superseded"
                predecessor[1]["superseded_by"] = memory_id
                predecessor[1]["valid_to"] = today.isoformat()
                atomic_write(predecessor[0], _render_chunk(predecessor[1], predecessor[2]))
            # The index is derived state: regenerate it from chunk
            # frontmatter instead of appending a row, so the same rendering
            # path serves promotions, manual capture, and post-merge repair.
            atomic_write(index_path, render_bank_index(bank)[0])
            validation_errors = bank_write_errors(repository, before, (destination,))
            if validation_errors:
                raise BrainError(
                    "Promoted Memory Bank chunk failed validation: "
                    + "; ".join(validation_errors)
                )
            proposal["status"] = "applied"
            proposal["destination_memory_id"] = memory_id
            proposal["destination_revision"] = 1
            proposal["updated_at"] = utc_now()
            proposal["revision"] += 1
            atomic_json(promotion_path, proposal)
        except Exception:
            restore_files(snapshot)
            raise
        return proposal
