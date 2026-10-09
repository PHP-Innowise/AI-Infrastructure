"""Record a run's result so that replaying it completes it (standard library only).

One write path for every writer that saves memory without a person at the
keyboard: the Memory MCP's `memory_record_result`, the Harness after a
completed run, and `context.py record-result`. A result is the task's progress
and next steps plus up to three source-backed reusable learnings.

Each step is atomic and the chain is not, so the chain is made replayable
instead: a learning's record is named by its content (task, type, title,
sources), so a second attempt finds the record the first one wrote - and
closes it if the first attempt stopped half way - instead of writing another.
The task update goes last. A receipt in the runtime's local state then says
the result is done, which is how a replay is recognised and how the same
result ID with different content is refused; a task that already holds the
result's progress and steps counts as updated even if the receipt was lost.
"""
from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path, PurePosixPath
from typing import Any, Optional

import workspace_roots
from automatic_query import RAW_TEXT_PATTERN
from brain_runtime import (
    LIFECYCLES,
    BrainError,
    atomic_json,
    auto_promote,
    brain_root,
    create_record,
    find_record,
    guard_shared_text,
    load_config,
    mutation_lock,
    source_fingerprints,
    update_record,
)

# A cited path that names a secret, a key or an environment file.
SOURCE_PATH_DENYLIST = re.compile(
    r"(^|/)(\.env(\..+)?|secrets?|id_[a-z0-9]+|[^/]+\.(pem|key|p12|pfx|jks|keystore))$",
    re.IGNORECASE,
)
# Directory names a learning never cites: tooling, dependencies, credentials.
BLOCKED_PARTS = frozenset({
    ".git", ".ssh", ".aws", ".kube", "node_modules", "vendor", ".venv",
    "__pycache__", "secrets", ".secrets", "credentials",
})
# Runtime state and derived memory: a learning cites what it was derived
# from, never memory derived from something else.
DERIVED_PREFIXES = (
    "memory-bank/local/", "memory-bank/chunks/", "project-brain/local/",
    "project-brain/dynamic/", "project-brain/archive/", "project-brain/control/",
)
RESULT_ID_PATTERN = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,63}")
TASK_ID_PATTERN = re.compile(r"[A-Za-z0-9][A-Za-z0-9._/-]{0,199}")
LEARNING_TYPES = ("finding", "decision")
LEARNING_LIMIT = 3
NEXT_STEP_LIMIT = 3
SOURCE_LIMIT = 10
ATTESTATIONS = ("agent", "person")
# What a learning becomes when it is recorded: a finding is resolved, a
# decision accepted - the states promotion carries into the Memory Bank.
CLOSED_STATE = {"finding": "resolved", "decision": "accepted"}


def _text(value: object, label: str, limit: int) -> str:
    if not isinstance(value, str) or not value.strip() or len(value) > limit:
        raise BrainError(f"{label} must be nonempty text within {limit} characters")
    guard_shared_text(label, [value])
    # Shared memory holds summaries, never logs or transcripts: a line that
    # opens like one ("stderr:", "user:") is refused, as a query's is cut.
    if RAW_TEXT_PATTERN.search(value):
        raise BrainError(f"{label} looks like a pasted log or transcript; summarize it instead")
    return value.strip()


def _strings(values: object, label: str, count: int, limit: int) -> list[str]:
    if not isinstance(values, list) or len(values) > count:
        raise BrainError(f"{label} must be a list of at most {count} items")
    return list(dict.fromkeys(_text(value, label, limit) for value in values))


def check_source(root: Path, value: object) -> str:
    """A canonical project file a learning may cite, or BrainError."""
    value = _text(value, "source path", 1024)
    head = value.split("#", 1)[0]
    if SOURCE_PATH_DENYLIST.search(head):
        raise BrainError("Sensitive source paths are refused")
    if head.startswith(DERIVED_PREFIXES):
        raise BrainError("Cite canonical project sources, not private runtime state or derived memory")
    relative = PurePosixPath(head)
    if (any(part.casefold() in BLOCKED_PARTS or part.casefold().startswith(".env") for part in relative.parts)
            or relative.suffix.casefold() in {".pem", ".key", ".p12", ".pfx", ".sqlite", ".db"}
            or relative.name.casefold() == "credentials.json"):
        raise BrainError("Private or dependency source paths are refused")
    if relative.is_absolute() or ".." in relative.parts or "\\" in head or ":" in head:
        raise BrainError("Use project-relative source paths")
    # The project's root: the runtime's own when installed, the project the
    # launcher named when the edition is attached and its state lives apart.
    current = workspace_roots.project_root(root)
    for part in relative.parts:
        current = current / part
        if current.is_symlink():
            raise BrainError("Symlink source paths are refused")
    if not current.is_file():
        raise BrainError("Source is missing or is not a regular file")
    return value


def check_task_id(value: object) -> str:
    task_id = _text(value, "task ID", 200)
    if not TASK_ID_PATTERN.fullmatch(task_id) or ".." in task_id.split("/"):
        raise BrainError("Invalid task ID")
    return task_id


def check_result_id(value: object) -> str:
    result_id = _text(value, "result ID", 64)
    if not RESULT_ID_PATTERN.fullmatch(result_id):
        raise BrainError("Invalid result ID")
    return result_id


def normalize(root: Path, data: object) -> dict[str, Any]:
    """The result as it will be written, checked before anything is.

    `next_steps` absent leaves the task's steps alone; an empty list clears
    them, which is how a finished plan says so.
    """
    if not isinstance(data, dict):
        raise BrainError("A result is an object")
    unknown = set(data) - {"progress", "next_steps", "learnings", "verified"}
    if unknown:
        raise BrainError("Unknown result fields: " + ", ".join(sorted(unknown)))
    if "progress" in data and data["progress"] is not None and not isinstance(data["progress"], str):
        raise BrainError("Progress must be text")
    if "verified" in data and type(data["verified"]) is not bool:
        raise BrainError("Verification attestation must be boolean")
    progress = _text(data["progress"], "progress", 1000) if data.get("progress") else None
    steps = (_strings(data["next_steps"], "next steps", NEXT_STEP_LIMIT, 300)
             if "next_steps" in data else None)
    learnings = data.get("learnings", [])
    if not isinstance(learnings, list) or len(learnings) > LEARNING_LIMIT:
        raise BrainError(f"Keep at most {LEARNING_LIMIT} learnings")
    checked = []
    for learning in learnings:
        if not isinstance(learning, dict) or set(learning) != {"type", "title", "consequence", "sources"}:
            raise BrainError("Each learning needs type, title, consequence and sources")
        if learning["type"] not in LEARNING_TYPES:
            raise BrainError("Use finding or decision")
        sources = _strings(learning["sources"], "sources", SOURCE_LIMIT, 1024)
        if not sources:
            raise BrainError("Cite sources for each learning")
        for source in sources:
            check_source(root, source)
        checked.append({
            "type": learning["type"],
            "title": _text(learning["title"], "title", 200),
            "consequence": _text(learning["consequence"], "consequence", 1000),
            "sources": sources,
        })
    if checked and data.get("verified") is not True:
        raise BrainError("Attest that each learning was checked against its sources with verified=true")
    if progress is None and steps is None and not checked:
        raise BrainError("Nothing to record")
    return {"progress": progress, "next_steps": steps, "learnings": checked}


def request_digest(task_id: str, result: dict[str, Any]) -> str:
    """What a result ID promises: the same content, whatever revision it is replayed at."""
    canonical = json.dumps({"task": task_id, **result}, sort_keys=True, ensure_ascii=False)
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def learning_id(task_id: str, learning: dict[str, Any]) -> str:
    """The external ID a learning is recorded under: its content, not its position.

    The same learning restated by a retry, or by a later result of the same
    task, names the same record.
    """
    key = json.dumps(
        [task_id, learning["type"], " ".join(learning["title"].casefold().split()),
         sorted(learning["sources"])],
        ensure_ascii=False,
    )
    return "learning-" + hashlib.sha256(key.encode("utf-8")).hexdigest()[:24]


def _existing(root: Path, identifier: str) -> tuple[Optional[dict[str, Any]], bool]:
    """The record a learning ID names and whether it is archived.

    Archived records count: compaction moves a record, it does not end its
    identity, and an archived learning is never reopened by a restatement.
    """
    try:
        path, record, _ = find_record(root, identifier, include_archive=True)
    except BrainError as error:
        if "not found" not in str(error):
            raise
        return None, False
    archive = brain_root(root) / "archive"
    try:
        path.resolve().relative_to(archive.resolve())
    except ValueError:
        return record, False
    return record, True


def writable_task(root: Path, task_id: str, owner: str) -> dict[str, Any]:
    _, task, _ = find_record(root, task_id, record_type="task")
    config = load_config(root)
    if task["privacy"] not in set(config["allowed_privacy"]) & {"public", "team"}:
        raise BrainError("Task is outside the allowed memory privacy scope")
    if owner not in task["authorized_owners"]:
        raise BrainError("The configured actor is not authorized for this task")
    return task


def _receipt(root: Path, task_id: str, result_id: str) -> Path:
    key = hashlib.sha256(f"{task_id}\0{result_id}".encode("utf-8")).hexdigest()
    return brain_root(root) / "local" / "results" / f"{key}.json"


def _read_receipt(path: Path) -> Optional[dict[str, Any]]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    return value if isinstance(value, dict) else None


def _holds(task: dict[str, Any], result: dict[str, Any]) -> bool:
    """Whether the task already says what this result would write."""
    return (result["progress"] is None or task.get("progress") == result["progress"]) and (
        result["next_steps"] is None or list(task.get("next_steps") or []) == result["next_steps"]
    )


def _summary(record: dict[str, Any], state: str) -> dict[str, Any]:
    return {"id": record["id"], "external_id": record["external_id"], "type": record["type"],
            "title": record["title"], "status": record["status"], "revision": record["revision"],
            "state": state}


def record_result(
    root: Path,
    task_id: str,
    result_id: str,
    revision: int,
    data: object,
    *,
    owner: str,
    reason: str,
    attestation: str = "agent",
) -> dict[str, Any]:
    """Write a result, or finish writing it, and say what each part became.

    A learning already recorded under its content ID is left as it is
    (`existing`) or, when an earlier attempt stopped before closing it,
    closed now (`completed`). The task update comes last; a result with a
    receipt is a replay and leaves the task alone. A stale `revision` is
    refused before anything is written; replay the same result ID with the
    current revision.
    """
    task_id = check_task_id(task_id)
    result_id = check_result_id(result_id)
    if type(revision) is not int or revision < 1:
        raise BrainError("Supply the current task revision")
    if attestation not in ATTESTATIONS:
        raise BrainError("attestation must be agent or person")
    result = normalize(root, data)
    with mutation_lock(root):
        task = writable_task(root, task_id, owner)
        # One task, one namespace: its external ID and its UUID both name it,
        # so every key below is derived from the UUID the lookup resolved.
        canonical = task["id"]
        digest = request_digest(canonical, result)
        receipt = _receipt(root, canonical, result_id)
        prior = _read_receipt(receipt)
        if prior is not None and prior.get("digest") != digest:
            raise BrainError("Result ID was already used with different content")
        if prior is None and task["status"] in ("completed", "cancelled"):
            raise BrainError("Use an active task to record new memory")
        task_fields = result["progress"] is not None or result["next_steps"] is not None
        update_task = task_fields and prior is None and not _holds(task, result)
        # Everything that can refuse the result is checked before the first
        # write: a stale revision - for any new result, learnings alone
        # included, since they too were written against what the agent last
        # read - and every cited file's fingerprint. Only a result with a
        # receipt is a proven replay. The lock is held to the end, so the
        # revision cannot move in between.
        if prior is None and task["revision"] != revision:
            raise BrainError(
                f"Stale task revision: expected {revision}, current {task['revision']}. "
                "Nothing was written; replay this result ID with the current revision."
            )
        for learning in result["learnings"]:
            source_fingerprints(root, learning["sources"])
        records = []
        wrote = False
        for learning in result["learnings"]:
            identifier = learning_id(canonical, learning)
            current, archived = _existing(root, identifier)
            closed_state = CLOSED_STATE[learning["type"]]
            state = "existing"
            if archived:
                records.append(_summary(current, "archived"))
                continue
            if current is None:
                current = create_record(
                    root, learning["type"], identifier, learning["title"], [],
                    learning["sources"], owner=owner, authority="observed",
                    goal=learning["consequence"],
                )
                state = "created"
            # Only a record still in its initial state is this result's to
            # finish: created just now, or left open by an attempt that
            # stopped between writing and closing it. One that has moved on
            # since (superseded, under investigation) is someone's later work.
            unfinished = current["status"] == LIFECYCLES[learning["type"]]["initial"] or (
                current["status"] == closed_state and current["authority"] != "verified")
            if unfinished:
                if owner not in current["authorized_owners"]:
                    raise BrainError("An earlier attempt's record belongs to another owner")
                current = update_record(
                    root, current["id"], expected_revision=current["revision"],
                    progress=learning["consequence"], next_steps=[], files=[], sources=[],
                    actor=owner, reason=reason,
                    authority="verified" if current["authority"] != "verified" else None,
                    transition_to=closed_state if current["status"] != closed_state else None,
                    attestation=attestation,
                )
                state = "created" if state == "created" else "completed"
            wrote = wrote or state != "existing"
            records.append(_summary(current, state))
        updated = task if task_fields else None
        if update_task:
            updated = update_record(
                root, task["id"], expected_revision=revision,
                progress=result["progress"], next_steps=result["next_steps"] or [],
                files=[], sources=[], actor=owner, reason=reason,
                replace_next_steps=result["next_steps"] is not None,
            )
            wrote = True
        receipt.parent.mkdir(parents=True, exist_ok=True)
        atomic_json(receipt, {"result_id": result_id, "task_id": task_id, "digest": digest,
                              "task_revision": updated["revision"] if updated else None})
        # Promotion runs for any result with learnings, a replay included:
        # an attempt interrupted after its records were written must still
        # reach the Memory Bank, and promotion is itself idempotent.
        promotion = auto_promote(root, owner=owner) if result["learnings"] else None
    return {
        "task": {"id": updated["id"], "revision": updated["revision"]} if updated else None,
        "records": records,
        "promotion": promotion,
        "attestation": attestation,
        # Nothing was written: the result, or every part of it, was already there.
        "replayed": not wrote,
    }
