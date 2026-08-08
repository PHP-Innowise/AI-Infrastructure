#!/usr/bin/env python3
"""Governed Project Brain storage primitives (Python 3.9+, standard library only)."""

from __future__ import annotations

import contextlib
import fcntl
import hashlib
import json
import os
import re
import tempfile
import threading
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any, Iterator, Optional

from validate import FILENAME_PATTERN as BANK_FILENAME_PATTERN, validate_bank


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
        "automatic_compaction": False,
        "compaction_threshold": 5,
        "codebase_map_max_drift": 25,
    }
    if not path.is_file():
        return defaults
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
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


@contextlib.contextmanager
def mutation_lock(repository: Path) -> Iterator[None]:
    """Serialize every repository mutation through one global lock file."""
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
            fcntl.flock(handle.fileno(), fcntl.LOCK_EX)
            depths[key] = 1
            try:
                yield
            finally:
                depths.pop(key, None)
                fcntl.flock(handle.fileno(), fcntl.LOCK_UN)


def atomic_write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=str(path.parent))
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(str(temporary), str(path))
    finally:
        if temporary.exists():
            temporary.unlink()


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


def parse_markdown_record(path: Path) -> tuple[dict[str, Any], str]:
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as error:
        raise BrainError(f"Cannot read Project Brain record: {path}") from error
    if not text.startswith("---\n"):
        raise BrainError(f"{path}: missing JSON frontmatter")
    try:
        raw, body = text[4:].split("\n---\n", 1)
        metadata = json.loads(raw)
    except (ValueError, json.JSONDecodeError) as error:
        raise BrainError(f"{path}: invalid JSON frontmatter") from error
    if not isinstance(metadata, dict):
        raise BrainError(f"{path}: frontmatter must be an object")
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
        return
    try:
        schema = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise BrainError(f"Cannot load Project Brain schema: {path}") from error
    validate_schema_value(value, schema)


def fingerprint(repository: Path, source: str) -> dict[str, str]:
    relative = source.split("#", 1)[0]
    path = (repository / relative).resolve()
    try:
        path.relative_to(repository.resolve())
    except ValueError as error:
        raise BrainError(f"Source escapes repository: {relative}") from error
    if not path.is_file() or path.is_symlink():
        raise BrainError(f"Source does not exist or is not a regular file: {relative}")
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    return {"path": relative, "sha256": digest}


def source_fingerprints(repository: Path, sources: list[str]) -> list[dict[str, str]]:
    fingerprints = []
    for source in sorted(set(sources)):
        relative = source.split("#", 1)[0]
        path = (repository / relative).resolve()
        try:
            path.relative_to(repository.resolve())
        except ValueError as error:
            raise BrainError(f"Source escapes repository: {relative}") from error
        if not path.is_file() or path.is_symlink():
            raise BrainError(
                f"Source does not exist or is not a regular file: {relative}"
            )
        fingerprints.append(fingerprint(repository, source))
    return fingerprints


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


def _require_string(record: dict[str, Any], key: str) -> None:
    if not isinstance(record.get(key), str) or not record[key].strip():
        raise BrainError(f"{key} must be a non-empty string")


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
    if record["schema_version"] != SCHEMA_VERSION:
        raise BrainError("unsupported schema_version")
    if not is_uuid4(record["id"]):
        raise BrainError("record id must be UUIDv4")
    if record["type"] not in RECORD_TYPES:
        raise BrainError(f"type must be one of: {', '.join(RECORD_TYPES)}")
    for key in ("external_id", "title", "owner", "goal"):
        _require_string(record, key)
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
    if not isinstance(record["confidence"], (int, float)) or not 0 <= record["confidence"] <= 1:
        raise BrainError("confidence must be between 0 and 1")
    if not isinstance(record["revision"], int) or record["revision"] < 1:
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
    for fingerprint_item in record["source_fingerprints"]:
        if (
            not isinstance(fingerprint_item, dict)
            or set(fingerprint_item) != {"path", "sha256"}
            or not isinstance(fingerprint_item["path"], str)
            or re.fullmatch(r"[0-9a-f]{64}", str(fingerprint_item["sha256"])) is None
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
    if "phase" in record and record["phase"] not in TASK_PHASES:
        raise BrainError(f"phase must be one of: {', '.join(TASK_PHASES)}")
    if not is_uuid4(record["id"]) or not is_uuid4(record["task_id"]):
        raise BrainError("handoff and task IDs must be UUIDv4")
    if record["type"] != "handoff" or record["status"] not in {"active", "closed"}:
        raise BrainError("invalid handoff type or status")
    if record["privacy"] not in PRIVACY:
        raise BrainError("invalid handoff privacy")
    for key in ("owner", "objective"):
        _require_string(record, key)
    for key in ("created_at", "updated_at"):
        _require_datetime(record, key)
    for key in ("task_revision", "revision"):
        if not isinstance(record[key], int) or record[key] < 1:
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


def iter_records(repository: Path, *, include_archive: bool = False) -> Iterator[tuple[Path, dict[str, Any], str]]:
    roots = [brain_root(repository) / "dynamic"]
    if include_archive:
        roots.append(brain_root(repository) / "archive")
    for root in roots:
        if not root.is_dir():
            continue
        for path in sorted(root.glob("**/*.md")):
            if path.is_file() and not path.is_symlink():
                metadata, body = parse_markdown_record(path)
                if metadata.get("type") == "handoff":
                    continue
                yield path, metadata, body


def find_record(
    repository: Path,
    identifier: str,
    *,
    record_type: Optional[str] = None,
    include_archive: bool = False,
) -> tuple[Path, dict[str, Any], str]:
    matches = []
    for path, record, body in iter_records(repository, include_archive=include_archive):
        if (
            (record_type is None or record.get("type") == record_type)
            and identifier in {record.get("id"), record.get("external_id")}
        ):
            matches.append((path, record, body))
    if not matches:
        label = record_type or "record"
        raise BrainError(f"Project Brain {label} not found: {identifier}")
    if len(matches) > 1:
        raise BrainError(f"Project Brain record identifier is ambiguous: {identifier}")
    return matches[0]


def find_task(repository: Path, identifier: str) -> tuple[Path, dict[str, Any], str]:
    return find_record(repository, identifier, record_type="task")


def _record_body(record: dict[str, Any]) -> str:
    next_steps = "\n".join(f"- {item}" for item in record["next_steps"]) or "- None"
    files = "\n".join(f"- `{item}`" for item in record["files"]) or "- None"
    return (
        f"# {record['title']}\n\n"
        f"## Goal\n{record['goal']}\n\n"
        f"## Progress\n{record['progress'] or 'Not started.'}\n\n"
        f"## Next Steps\n{next_steps}\n\n"
        f"## Files\n{files}\n"
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
    try:
        find_record(repository, external_id)
    except BrainError as error:
        if "not found" not in str(error):
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
    with mutation_lock(repository):
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
    allow_phase_regression: bool = False,
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
    """
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
            or authority is not None
        ):
            raise BrainError("Events are immutable; only lifecycle supersession is allowed")
        if auto_checkpoint is not None and record["type"] != "task":
            raise BrainError("only a task may record an auto checkpoint")
        if authority is not None:
            if authority not in AUTHORITY_TRANSITIONS.get(record["authority"], set()):
                raise BrainError(
                    f"Illegal authority transition: {record['authority']} -> "
                    f"{authority}; only observed -> verified is allowed"
                )
            record["transitions"].append(
                {
                    "from": record["authority"], "to": authority, "at": utc_now(),
                    "actor": actor, "reason": reason,
                }
            )
            record["authority"] = authority
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
        record["next_steps"] = list(dict.fromkeys([*record["next_steps"], *next_steps]))
        record["files"] = list(dict.fromkeys([*record["files"], *files]))
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
        with path.open("a", encoding="utf-8") as handle:
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
    if not sources_are_fresh(repository, record):
        return False, "stale"
    return True, "eligible"


def build_index_data(
    repository: Path,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    active: list[dict[str, Any]] = []
    archived: list[dict[str, Any]] = []
    for path, record, _ in iter_records(repository, include_archive=True):
        item = {
            "id": record.get("id"), "type": record.get("type"), "status": record.get("status"),
            "revision": record.get("revision"), "path": path.relative_to(repository).as_posix(),
        }
        if "archive" in path.parts:
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


def validate_repository(repository: Path) -> list[str]:
    errors: list[str] = []
    seen: set[str] = set()
    records: dict[str, tuple[Path, dict[str, Any]]] = {}
    for path, record, _ in iter_records(repository, include_archive=True):
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
            if not sources_are_fresh(repository, record):
                raise BrainError("source fingerprint is stale")
            records[record["id"]] = (path, record)
        except BrainError as error:
            errors.append(f"{path}: {error}")
    for record_id, (path, record) in records.items():
        for relationship in [*record["conflicts"], *record["supersedes"]]:
            if relationship not in records:
                errors.append(f"{path}: relationship points to missing {relationship}")
        replacement = record["superseded_by"]
        if replacement is not None and replacement not in records:
            errors.append(f"{path}: superseded_by points to missing {replacement}")
        if replacement in records and record_id not in records[replacement][1]["supersedes"]:
            errors.append(f"{path}: supersession backlink is missing from {replacement}")
    handoff_root = brain_root(repository) / "control" / "handoffs"
    if handoff_root.is_dir():
        for path in sorted(handoff_root.glob("*.md")):
            try:
                handoff, _ = parse_markdown_record(path)
                validate_schema_file(repository, "handoff.schema.json", handoff)
                task = get_task(repository, handoff["task_id"])
                validate_handoff(handoff, task)
            except (BrainError, KeyError) as error:
                errors.append(f"{path}: {error}")
    archived_handoffs = brain_root(repository) / "archive" / "handoffs"
    if archived_handoffs.is_dir():
        for path in sorted(archived_handoffs.glob("*.md")):
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
    manifest_keys = {
        "schema_version", "id", "created_at", "query", "task_id", "task_revision",
        "filters", "selected", "excluded", "token_estimates", "provider",
        "escalation_reason",
    }
    manifests = brain_root(repository) / "control" / "retrieval-manifests"
    if manifests.is_dir():
        for path in sorted(manifests.glob("*.json")):
            try:
                manifest = json.loads(path.read_text(encoding="utf-8"))
                validate_schema_file(
                    repository, "retrieval-manifest.schema.json", manifest
                )
                if not isinstance(manifest, dict) or set(manifest) != manifest_keys:
                    raise BrainError("retrieval manifest does not match strict schema")
                if not is_uuid4(manifest["id"]) or not is_uuid4(manifest["task_id"]):
                    raise BrainError("retrieval manifest IDs must be UUIDv4")
                if manifest["id"] != path.stem:
                    raise BrainError("retrieval manifest ID does not match filename")
                estimates = manifest["token_estimates"]
                if not isinstance(estimates, dict) or estimates.get("total", 0) > estimates.get("hard", 12000):
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
    try:
        expected_active, expected_archive = build_index_data(repository)
        active_path, archive_path = index_paths(repository)
        for path, expected in (
            (active_path, expected_active),
            (archive_path, expected_archive),
        ):
            actual = json.loads(path.read_text(encoding="utf-8"))
            if actual != expected:
                errors.append(f"{path}: deterministic index is stale or non-canonical")
    except (OSError, json.JSONDecodeError, BrainError) as error:
        errors.append(f"{brain_root(repository) / 'indexes'}: {error}")
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
            rewrites.append((path, render_markdown_record(metadata, body)))
    return rewrites


def compact(repository: Path) -> dict[str, int]:
    with mutation_lock(repository):
        errors = validate_repository(repository)
        if errors:
            raise BrainError("Compaction refused because active records are invalid")
        moves: list[tuple[Path, Path]] = []
        for path, record, _ in list(iter_records(repository)):
            if (
                record["status"] not in LIFECYCLES[record["type"]]["terminal"]
                and record.get("superseded_by") is None
            ):
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
            post_errors = validate_repository(repository)
            if post_errors:
                raise BrainError(
                    "Archive validation failed after compaction: "
                    + "; ".join(post_errors)
                )
            bank = repository / "memory-bank"
            if bank.is_dir():
                bank_errors = validate_bank(bank)
                if bank_errors:
                    raise BrainError(
                        "Memory Bank validation failed after compaction: "
                        + "; ".join(bank_errors)
                    )
        except Exception:
            restore_files(snapshot)
            raise
    moved_records = sum(
        1 for source, _ in moves if "dynamic" in source.parts
    )
    return {"moved": moved_records}


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


def promotable_records(
    repository: Path, config: dict[str, Any]
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
        if record["id"] in already:
            continue
        if record["status"] not in PROMOTABLE_STATES.get(record["type"], set()):
            continue
        reason: Optional[str] = None
        if record["authority"] != "verified":
            reason = f"authority is {record['authority']}, not verified"
        elif record["privacy"] not in config["allowed_privacy"]:
            reason = f"privacy {record['privacy']} is not allowed for retrieval"
        elif not sources_are_fresh(repository, record):
            stale = [
                item["path"]
                for item in record["source_fingerprints"]
                if not sources_are_fresh(
                    repository, {"sources": [item["path"]], "source_fingerprints": [item]}
                )
            ]
            reason = "cited source changed since the record was written: " + ", ".join(
                stale or record["sources"]
            )
        content = promotion_content(record)
        if reason is None and content is None:
            reason = "record carries no content beyond its own title"
        if reason is not None:
            blocked.append({"record_id": record["id"], "reason": reason})
            continue
        candidates.append({"record": record, "content": content})
    return candidates, blocked


def auto_promote(repository: Path, *, owner: str, limit: int = 5) -> dict[str, Any]:
    """Promote resolved, verified knowledge into durable memory without review.

    Every applied chunk is tagged `auto-promoted` and its promotion names no
    reviewer, so the absence of human approval stays visible in both stores.
    """
    config = load_config(repository)
    if not config.get("automatic_promotion"):
        return {
            "enabled": False, "promoted": [], "failed": [],
            "blocked": [], "skipped": 0,
        }
    promoted: list[dict[str, Any]] = []
    failed: list[dict[str, str]] = []

    # Retry a promotion that stalled after its record was created, in place.
    # Creating a second one would leave the first orphaned forever.
    for promotion_id, proposal in stalled_automatic_promotions(repository):
        source = proposal["source_records"][0]
        try:
            if proposal["status"] == "proposed":
                auto_review_promotion(repository, promotion_id)
            applied = apply_promotion(repository, promotion_id)
        except BrainError as error:
            failed.append({"record_id": source["id"], "reason": str(error)})
            continue
        promoted.append(
            {
                "record_id": source["id"],
                "type": source["type"],
                "memory_id": applied["destination_memory_id"],
            }
        )

    candidates, blocked = promotable_records(repository, config)
    for candidate in candidates[:limit]:
        record = candidate["record"]
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
            applied = apply_promotion(repository, proposal["id"])
        except BrainError as error:
            # One unpromotable record must not stop the rest, but a systematic
            # failure — an absent Memory Bank, say — has to stay visible rather
            # than looking like "nothing was worth promoting".
            failed.append({"record_id": record["id"], "reason": str(error)})
            continue
        promoted.append(
            {
                "record_id": record["id"],
                "type": record["type"],
                "memory_id": applied["destination_memory_id"],
            }
        )
    return {
        "enabled": True,
        "promoted": promoted,
        "failed": failed,
        "blocked": blocked,
        "skipped": max(0, len(candidates) - limit),
    }


def archivable_records(repository: Path) -> int:
    """Count active records that compaction would move."""
    total = 0
    for _, record, _ in iter_records(repository):
        if (
            record["status"] in LIFECYCLES[record["type"]]["terminal"]
            or record.get("superseded_by") is not None
        ):
            total += 1
    return total


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
        today = datetime.now(timezone.utc).date()
        # Conflict-free identifier: the promotion date plus eight hex
        # characters of the source record's UUID. Two machines or branches
        # promoting concurrently cannot collide the way the retired shared
        # `.memory-counter` file did, so a later merge is a plain union of
        # chunk files. The legacy counter file may remain on disk; it is
        # neither read nor written here.
        source_uuid = proposal["source_records"][0]["id"]
        memory_id = (
            f"MEM-{today.strftime('%Y%m%d')}-{source_uuid.replace('-', '')[:8]}"
        )
        slug = re.sub(r"[^a-z0-9]+", "-", proposal["title"].lower()).strip("-") or "promoted"
        destination = bank / "chunks" / f"{memory_id}-{slug}.md"
        if destination.exists():
            raise BrainError(f"Promotion destination already exists: {destination}")
        # Tag the chunk so the bank itself shows which knowledge no human
        # approved; a reader must not have to open the promotion to find out.
        tags = ["project-brain", "promoted"] + (["auto-promoted"] if automatic else [])
        metadata = {
            "id": memory_id, "title": proposal["title"], "type": "decision", "status": "active",
            "scope": ["application"], "tags": tags,
            "created": today.isoformat(), "last_verified": today.isoformat(),
            "review_after": (today + timedelta(days=365)).isoformat(),
            "sources": [item["path"] for item in proposal["source_records"]],
            "supersedes": [], "superseded_by": None,
            # Open-ended validity: the knowledge holds from today until a
            # successor closes it with a valid_to, rather than being deleted.
            "valid_from": today.isoformat(), "valid_to": None,
        }
        heading = f"# {proposal['title']}"
        content = proposal["content"].strip()
        chunk = render_markdown_record(
            metadata,
            content if content.startswith(heading) else f"{heading}\n\n{content}",
        )
        snapshot = snapshot_files([destination, index_path, promotion_path])
        try:
            atomic_write(destination, chunk)
            # The index is derived state: regenerate it from chunk
            # frontmatter instead of appending a row, so the same rendering
            # path serves promotions, manual capture, and post-merge repair.
            atomic_write(index_path, render_bank_index(bank)[0])
            validation_errors = validate_bank(bank)
            if validation_errors:
                raise BrainError(
                    "Promoted Memory Bank chunk failed validation: "
                    + "; ".join(validation_errors)
                )
            proposal["status"] = "applied"
            proposal["outcome"] = "promoted"
            proposal["destination_memory_id"] = memory_id
            proposal["destination_revision"] = 1
            proposal["updated_at"] = utc_now()
            proposal["revision"] += 1
            atomic_json(promotion_path, proposal)
        except Exception:
            restore_files(snapshot)
            raise
        return proposal
