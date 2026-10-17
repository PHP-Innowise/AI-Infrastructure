"""Guarded result saves and passive exact replay (standard library only).

The native lock orders a shared metadata-only started fence before record
writes, and a complete fence before the disposable local receipt. An uncertain
started attempt is refused without further mutation. Exact complete proof
permits passive replay after local-cache loss; current ownership/privacy still
apply. This is application ordering, not a power-loss transaction or signed
proof. Missing both proofs cannot authorize historical adoption.

New saves capture the current locked task revision for auto, or validate the
caller's numeric task CAS. A learning conflict is retained individually.
Automatic agent correction needs its exact last-native-write image; an old
agent attestation alone cannot authorize overwriting a later manual edit.
"""
from __future__ import annotations

import hashlib
import json
import os
import re
import stat
from pathlib import Path, PurePosixPath
from typing import Any, Optional

import workspace_roots
from automatic_query import RAW_TEXT_PATTERN, SOURCE_PATH_DENYLIST, source_path_problem
from brain_runtime import (
    LIFECYCLES,
    linked_reparse,
    RESULT_ATTEMPTS,
    RESULT_LEARNING_IMAGES,
    UUID4_PATTERN,
    BrainError,
    BrainRecordNotFound,
    atomic_json,
    auto_promote,
    brain_root,
    create_record,
    find_record,
    guard_shared_text,
    iter_records,
    load_config,
    mutation_lock,
    record_attestation,
    source_fingerprints,
    update_record,
)

RESULT_ID_PATTERN = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,63}")
TASK_ID_PATTERN = re.compile(r"[^\W_][\w\u0300-\u036f./#+-]{0,199}")  # letters and digits of any script
LEARNING_TYPES = ("finding", "decision")
LEARNING_LIMIT = 3
NEXT_STEP_LIMIT = 3
SOURCE_LIMIT = 10
ATTESTATIONS = ("agent", "person")
PREPARE_PROFILE = "harness-memory-request-v1"
ATTEMPT_PROFILE = "memory-result-attempt-v1"
FRONTIER_PROFILE = "memory-result-frontier-v2"
# What a finished save leaves in the shared Brain: only what an exact replay
# checks, so the same save finished in two clones writes the same bytes.
PROOF_PROFILE = "memory-result-proof-v1"
PROOF_FIELDS = {"profile", "task_id", "result_id", "digest", "owner", "attestation", "state"}
# Completion proofs kept per task. A replay comes from the session that saved,
# whose clone also keeps its own receipt; a replay with neither is a new
# write, held to the task revision like any other.
PROOFS_KEPT = 64
EVIDENCE_NAME = re.compile(r"[0-9a-f]{64}\.json")
ATTEMPT_LIMIT = 4096
DOMAIN_NODE_LIMIT = 4096
DOMAIN_BYTE_LIMIT = 32 * 1024 * 1024
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


def check_source(root: Path, value: object, *, require_file: bool = True) -> str:
    """A canonical project file a learning may cite, or BrainError."""
    value = _text(value, "source path", 1024)
    head = value.split("#", 1)[0]
    problem = source_path_problem(value)
    if problem:
        raise BrainError(problem)
    if not require_file:
        return value
    relative = PurePosixPath(head)
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


def normalize(root: Path, data: object, *, check_files: bool = True) -> dict[str, Any]:
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
    identities = {}
    for learning in learnings:
        fields = {"type", "title", "consequence", "sources"}
        if (not isinstance(learning, dict) or not fields <= set(learning)
                or set(learning) - fields - {"expected_revision"}):
            raise BrainError("Each learning needs type, title, consequence and sources")
        if "expected_revision" in learning and (type(learning["expected_revision"]) is not int
                                                or learning["expected_revision"] < 1):
            raise BrainError("Learning expected_revision must be a positive original revision")
        if learning["type"] not in LEARNING_TYPES:
            raise BrainError("Use finding or decision")
        sources = _strings(learning["sources"], "sources", SOURCE_LIMIT, 1024)
        if not sources:
            raise BrainError("Cite sources for each learning")
        for source in sources:
            check_source(root, source, require_file=check_files)
        checked.append({
            "type": learning["type"],
            "title": _text(learning["title"], "title", 200),
            "consequence": _text(learning["consequence"], "consequence", 1000),
            "sources": sources,
        })
        if "expected_revision" in learning:
            checked[-1]["expected_revision"] = learning["expected_revision"]
        # This static conflict must precede native task provisioning and
        # digest binding, so a refused format can be corrected in place.
        # Match learning_id's semantic key; its hashed collision guard also
        # remains in record_result before any authoritative result effect.
        item = checked[-1]
        identity = (item["type"], " ".join(item["title"].casefold().split()),
                    tuple(sorted(item["sources"])))
        if identity in identities and identities[identity] != item:
            raise BrainError("Conflicting learning identities in one result")
        identities[identity] = item
    if checked and data.get("verified") is not True:
        raise BrainError("Attest that each learning was checked against its sources with verified=true")
    if progress is None and steps is None and not checked:
        raise BrainError("Nothing to record")
    return {"progress": progress, "next_steps": steps, "learnings": checked}


def request_digest(task_id: str, result: dict[str, Any]) -> str:
    """What a result ID promises: the same content, whatever revision it is replayed at."""
    canonical = json.dumps({"task": task_id, **result}, sort_keys=True, ensure_ascii=False)
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def prepared_scope(root: Path, owner: str, reason: str, attestation: str) -> str:
    """Local implementation/policy identity, independent of mutable result evidence.

    This is a consistency guard, not runtime authentication. Task revisions,
    archive placement, citations and receipts deliberately do not enter it.
    """
    _text(owner, "owner", 200)
    _text(reason, "reason", 2000)
    if attestation not in ATTESTATIONS:
        raise BrainError("attestation must be agent or person")
    scripts = Path(__file__).resolve().parent
    images = {}
    for name in ("memory_results.py", "brain_runtime.py", "automatic_query.py", "workspace_roots.py", "context.py"):
        path = scripts / name
        if path.stat().st_size > 4 * 1024 * 1024:
            raise BrainError("Prepared runtime profile exceeds its source limit")
        images[name] = hashlib.sha256(path.read_bytes()).hexdigest()
    value = {"profile": PREPARE_PROFILE, "root": str(root.resolve()),
             "project": str(workspace_roots.project_root(root).resolve()),
             "scripts": str(scripts), "images": images, "config": load_config(root),
             "owner": owner, "reason": reason, "attestation": attestation}
    return hashlib.sha256(json.dumps(value, sort_keys=True, ensure_ascii=False,
                                    allow_nan=False).encode("utf-8")).hexdigest()


def prepare_result(root: Path, task_id: str, result_id: str, revision: int,
                   data: object, *, owner: str, reason: str,
                   attestation: str = "agent") -> dict[str, Any]:
    """Validate a safe retained wire request without recording a result."""
    task_id, result_id = check_task_id(task_id), check_result_id(result_id)
    if type(revision) is not int or revision < 1:
        raise BrainError("Supply the current task revision")
    result = normalize(root, data, check_files=False)
    _text(owner, "owner", 200)
    _text(reason, "reason", 2000)
    with mutation_lock(root):
        scope = prepared_scope(root, owner, reason, attestation)
        task = writable_task(root, task_id, owner)
        if task["status"] in ("completed", "cancelled"):
            raise BrainError("Use an active task to record new memory")
        if task["revision"] != revision:
            raise BrainError("memory-result-v1:prepared_stale_no_write")
        wire = {"learnings": result["learnings"], "verified": True}
        for field in ("progress", "next_steps"):
            if result[field] is not None:
                wire[field] = result[field]
        return {"profile": PREPARE_PROFILE, "task_id": task["id"],
                "result_id": result_id, "revision": revision, "owner": owner,
                "reason": reason, "attestation": attestation, "scope": scope,
                "digest": request_digest(task["id"], result), "request": wire}


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
    except BrainRecordNotFound as error:
        if error.identifier != identifier or error.record_type is not None:
            raise
        return None, False
    archive = brain_root(root) / "archive"
    try:
        path.resolve().relative_to(archive.resolve())
    except ValueError:
        return record, False
    return record, True


def _authorized_task(root: Path, task: dict[str, Any], owner: str) -> dict[str, Any]:
    config = load_config(root)
    if task["privacy"] not in set(config["allowed_privacy"]) & {"public", "team"}:
        raise BrainError("Task is outside the allowed memory privacy scope")
    if owner not in task["authorized_owners"]:
        raise BrainError("The configured actor is not authorized for this task")
    return task


def writable_task(root: Path, task_id: str, owner: str) -> dict[str, Any]:
    _, task, _ = find_record(root, task_id, record_type="task")
    return _authorized_task(root, task, owner)


def _result_task(root: Path, task_id: str, owner: str) -> tuple[dict[str, Any], bool]:
    """Resolve a result namespace, retaining live alias precedence.

    UUID-shaped identifiers must be unambiguous across active and archived
    tasks: a new alias cannot silently take over an archived UUID's receipt.
    Ordinary aliases select the active task before considering archives.
    """
    if UUID4_PATTERN.fullmatch(task_id.lower()):
        path, task, _ = find_record(root, task_id, record_type="task", include_archive=True)
    else:
        try:
            path, task, _ = find_record(root, task_id, record_type="task")
        except BrainRecordNotFound as error:
            if error.identifier != task_id or error.record_type != 'task':
                raise
            path, task, _ = find_record(root, task_id, record_type="task", include_archive=True)
    task = _authorized_task(root, task, owner)
    try:
        path.relative_to(brain_root(root) / "archive")
    except ValueError:
        return task, False
    return task, True


def _receipt(root: Path, task_id: str, result_id: str) -> Path:
    key = hashlib.sha256(f"{task_id}\0{result_id}".encode("utf-8")).hexdigest()
    return brain_root(root) / "local" / "results" / f"{key}.json"


def _read_receipt(path: Path) -> Optional[dict[str, Any]]:
    try:
        image = path.lstat()
        if (not stat.S_ISREG(image.st_mode) or image.st_nlink != 1 or image.st_size > ATTEMPT_LIMIT
                or linked_reparse(image)):
            return None
        with path.open("rb") as handle:
            raw = handle.read(ATTEMPT_LIMIT + 1)
        if len(raw) > ATTEMPT_LIMIT:
            return None
        value = json.loads(raw)
    except (OSError, ValueError, UnicodeError, RecursionError):
        return None
    return value if isinstance(value, dict) else None


def _attempt(root: Path, task_id: str, result_id: str, digest: str) -> Path:
    """Shared fencing evidence survives disposal of the local receipt cache.

    Named by task, result ID and request content: a save in another clone
    that reuses a result ID for other content is another save, not a refusal,
    and the two clones' files never meet in a merge. A folder per task lets
    compaction drop an archived task's evidence whole.
    """
    key = hashlib.sha256(f"{task_id}\0{result_id}\0{digest}".encode("utf-8")).hexdigest()
    return brain_root(root).joinpath(*RESULT_ATTEMPTS) / task_id / f"{key}.json"


def _same_file_image(left, right) -> bool:
    """Same file and bytes by stat, without atime.

    Comparing whole stat results compared st_atime too, and reading the
    file - which this very check does - moves atime on strictatime mounts and
    often on relatime ones, so the image never matched and recovery failed.
    The change time is the one a path's stat and an open file's fstat agree
    on: Windows gives them different ones.
    """
    keys = ("st_dev", "st_ino", "st_mode", "st_nlink", "st_size", "st_mtime_ns")
    return (all(getattr(left, key) == getattr(right, key) for key in keys)
            and workspace_roots.change_time_ns(left) == workspace_roots.change_time_ns(right))


def _learning_image(root: Path, identifier: str) -> Optional[str]:
    """Exact current record bytes; a verification label alone is not ownership."""
    try:
        path, _, _ = find_record(root, identifier, include_archive=True)
        image = path.lstat()
        if (not stat.S_ISREG(image.st_mode) or image.st_nlink != 1 or image.st_size > 4 * 1024 * 1024
                or linked_reparse(image)):
            return None
        with path.open('rb') as stream:
            if not _same_file_image(os.fstat(stream.fileno()), image):
                return None
            raw = stream.read(4 * 1024 * 1024 + 1)
        if len(raw) != image.st_size or not _same_file_image(path.stat(), image):
            return None
        return hashlib.sha256(raw).hexdigest()
    except (OSError, BrainError):
        return None


def _learning_image_path(root: Path, identifier: str) -> Path:
    key = hashlib.sha256(identifier.encode("utf-8")).hexdigest()
    return brain_root(root).joinpath(*RESULT_LEARNING_IMAGES) / (key + ".json")


def _own_open_creation(record: dict[str, Any], learning: dict[str, Any], owner: str) -> bool:
    """Whether an open record is exactly the one this learning would create."""
    transitions = record.get("transitions") or []
    return (record.get("revision") == 1
            and record.get("status") == LIFECYCLES[learning["type"]]["initial"]
            and record.get("authority") == "observed"
            and record.get("goal") == learning["consequence"]
            and sorted(record.get("sources") or []) == sorted(learning["sources"])
            and owner in (record.get("authorized_owners") or [])
            and len(transitions) == 1 and transitions[0].get("actor") == owner)


def _owned_learning_image(root: Path, identifier: str, record: dict[str, Any], owner: str) -> bool:
    path = _learning_image_path(root, identifier)
    try:
        _attempt_directory(path)
        proof = _read_receipt(path)
        return (isinstance(proof, dict)
                and set(proof) == {"profile", "record_id", "revision", "sha256", "owner"}
                and proof["profile"] == "native-learning-image-v1"
                and proof["record_id"] == record["id"] and proof["owner"] == owner
                and type(proof["revision"]) is int and proof["revision"] == record["revision"]
                and isinstance(proof["sha256"], str) and re.fullmatch(r"[0-9a-f]{64}", proof["sha256"])
                and proof["sha256"] == _learning_image(root, identifier))
    except (OSError, BrainError):
        return False


def _remember_learning_image(root: Path, identifier: str, record: dict[str, Any], owner: str) -> None:
    path = _learning_image_path(root, identifier)
    try:
        _attempt_directory(path)
        image = _learning_image(root, identifier)
        if image is not None:
            atomic_json(path, {"profile": "native-learning-image-v1", "record_id": record["id"],
                              "revision": record["revision"], "sha256": image, "owner": owner})
    except (OSError, BrainError):
        # The record did save. Missing proof withholds only later implicit
        # corrections, never the independent progress/facts of this save.
        pass


def _attempt_directory(path: Path) -> None:
    # Existing supported lock/atomic-write profile, not a CAS against foreign
    # directory swaps. Refuse linked/reparse parents rather than follow them,
    # from the file's own folder up to the Brain root.
    for parent in path.parents:
        if parent.exists() or parent.is_symlink():
            image = parent.lstat()
            if (not stat.S_ISDIR(image.st_mode) or stat.S_ISLNK(image.st_mode)
                    or linked_reparse(image)):
                raise BrainError("memory-result-v1:attempt_unavailable")
        if parent.name == "project-brain":
            break


def _read_attempt(path: Path) -> Optional[dict[str, Any]]:
    _attempt_directory(path)
    try:
        image = path.lstat()
    except FileNotFoundError:
        return None
    if (not stat.S_ISREG(image.st_mode) or image.st_nlink != 1 or image.st_size > ATTEMPT_LIMIT
            or linked_reparse(image)):
        raise BrainError("memory-result-v1:attempt_unavailable")
    def unique(pairs):
        value = {}
        for key, item in pairs:
            if key in value:
                raise ValueError()
            value[key] = item
        return value
    try:
        with path.open("rb") as handle:
            raw = handle.read(ATTEMPT_LIMIT + 1)
        value = json.loads(raw, object_pairs_hook=unique)
        if isinstance(value, dict) and value.get("profile") == PROOF_PROFILE:
            if (len(raw) > ATTEMPT_LIMIT or set(value) != PROOF_FIELDS or value["state"] != "complete"
                    or not UUID4_PATTERN.fullmatch(value["task_id"])
                    or not RESULT_ID_PATTERN.fullmatch(value["result_id"])
                    or not re.fullmatch(r"[0-9a-f]{64}", value["digest"])
                    or not isinstance(value["owner"], str) or not 0 < len(value["owner"]) <= 200
                    or value["attestation"] not in ATTESTATIONS):
                raise ValueError()
            return {**value, "task_revision": None}
        fields = {"profile", "task_id", "result_id", "digest", "original_revision", "owner",
                  "attestation", "plan_sha256", "state", "task_revision"}
        frontier = value.get("profile") == FRONTIER_PROFILE if isinstance(value, dict) else False
        if frontier:
            fields |= {"scope_sha256", "source_sha256", "domain_sha256", "stages"}
        if (len(raw) > ATTEMPT_LIMIT or not isinstance(value, dict) or set(value) != fields
                or value["profile"] not in (ATTEMPT_PROFILE, FRONTIER_PROFILE) or value["state"] not in ("started", "complete")
                or not UUID4_PATTERN.fullmatch(value["task_id"])
                or not RESULT_ID_PATTERN.fullmatch(value["result_id"])
                or any(not re.fullmatch(r"[0-9a-f]{64}", value[key]) for key in ("digest", "plan_sha256"))
                or type(value["original_revision"]) is not int or value["original_revision"] < 1
                or not isinstance(value["owner"], str) or not 0 < len(value["owner"]) <= 200
                or value["attestation"] not in ATTESTATIONS
                or (value["task_revision"] is not None and (type(value["task_revision"]) is not int
                                                          or value["task_revision"] < 1))
                or (not frontier and value["state"] == "started" and value["task_revision"] is not None)):
            raise ValueError()
        if frontier:
            for name in ("scope_sha256", "source_sha256"):
                if not isinstance(value[name], str) or not re.fullmatch(r"[0-9a-f]{64}", value[name]):
                    raise ValueError()
            if (value["domain_sha256"] is not None and (not isinstance(value["domain_sha256"], str)
                    or not re.fullmatch(r"[0-9a-f]{64}", value["domain_sha256"]))):
                raise ValueError()
            if not isinstance(value["stages"], dict) or len(value["stages"]) > LEARNING_LIMIT + 1:
                raise ValueError()
            for key, stage in value["stages"].items():
                if (not (key == "task" or re.fullmatch(r"learning-[0-9a-f]{24}", key))
                        or not isinstance(stage, dict)
                        or set(stage) != {"record_id", "revision", "sha256", "state", "done"}
                        or not UUID4_PATTERN.fullmatch(stage["record_id"])
                        or type(stage["revision"]) is not int or stage["revision"] < 1
                        or not isinstance(stage["sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", stage["sha256"])
                        or stage["state"] not in ("created", "completed", "updated", "task")
                        or type(stage["done"]) is not bool
                        or (key == 'task' and (stage['state'] != 'task' or not stage['done']))
                        or (key != 'task' and stage['state'] == 'task')):
                    raise ValueError()
    except (OSError, ValueError, TypeError, UnicodeError, RecursionError) as error:
        raise BrainError("memory-result-v1:attempt_unavailable") from error
    return value


# Content hashes of Brain files this process has read, by path, valid while
# the file keeps the identity it was hashed under (device, inode, size,
# modification and change times). A save checks its frontier after every
# stage, and each check hashed every record again.
_DOMAIN_HASHES: dict[str, tuple[tuple[int, ...], str]] = {}
_DOMAIN_HASHES_LIMIT = 50000


def _domain_image(root: Path) -> Optional[str]:
    """Bounded exact frontier hint, not a transaction or an authenticity proof.

    Hash all record/index/handoff bytes so a failed helper return is never
    assumed to mean no effects. If admission is unavailable, saving still
    works, but that started attempt has no automatic recovery authority.
    """
    try:
        bank = brain_root(root)
        prefix = len(str(bank)) + 1
        rows = []
        total = 0
        nodes = 0
        for folder in (bank / "dynamic", bank / "indexes", bank / "control" / "handoffs"):
            if not folder.exists():
                continue
            if folder.is_symlink() or linked_reparse(folder.lstat()):
                return None
            pending = [(str(folder), 0)]
            while pending:
                directory, depth = pending.pop()
                with os.scandir(directory) as entries:
                    for entry in entries:
                        nodes += 1
                        if nodes > DOMAIN_NODE_LIMIT:
                            return None
                        if depth >= 32 or entry.is_symlink():
                            return None
                        image = os.lstat(entry.path)
                        if linked_reparse(image):
                            return None
                        if stat.S_ISDIR(image.st_mode):
                            pending.append((entry.path, depth + 1))
                            continue
                        if not stat.S_ISREG(image.st_mode) or image.st_nlink != 1:
                            return None
                        total += image.st_size
                        if total > DOMAIN_BYTE_LIMIT or len(rows) >= DOMAIN_NODE_LIMIT:
                            return None
                        key = (image.st_dev, image.st_ino, image.st_size,
                               image.st_mtime_ns, image.st_ctime_ns)
                        cached = _DOMAIN_HASHES.get(entry.path)
                        if cached is not None and cached[0] == key:
                            digest = cached[1]
                        else:
                            with open(entry.path, 'rb') as stream:
                                if not _same_file_image(os.fstat(stream.fileno()), image):
                                    return None
                                raw = stream.read(image.st_size + 1)
                            if len(raw) != image.st_size or not _same_file_image(os.stat(entry.path), image):
                                return None
                            digest = hashlib.sha256(raw).hexdigest()
                            if len(_DOMAIN_HASHES) >= _DOMAIN_HASHES_LIMIT:
                                _DOMAIN_HASHES.clear()
                            _DOMAIN_HASHES[entry.path] = (key, digest)
                        # The path below the Brain root in the platform's own
                        # separators, as str(path.relative_to(bank)) gave it.
                        rows.append([entry.path[prefix:], digest])
        return hashlib.sha256(json.dumps(sorted(rows), sort_keys=True).encode('utf-8')).hexdigest()
    except (OSError, ValueError):
        return None


def _result_sources_image(root: Path, result: dict[str, Any]) -> str:
    images = []
    for learning in result['learnings']:
        for source in learning['sources']:
            check_source(root, source)
        images.append([learning_id('00000000-0000-4000-8000-000000000000', learning),
                       source_fingerprints(root, learning['sources'])])
    return hashlib.sha256(json.dumps(images, sort_keys=True, ensure_ascii=False).encode('utf-8')).hexdigest()


def _checkpoint_attempt(root: Path, path: Path, attempt: dict[str, Any], key: str,
                        record: dict[str, Any], state: str, done: bool) -> None:
    if attempt.get('profile') != FRONTIER_PROFILE:
        return
    image = _learning_image(root, record['id'])
    if image is None:
        attempt['domain_sha256'] = None
    else:
        attempt['stages'][key] = {'record_id': record['id'], 'revision': record['revision'],
                                  'sha256': image, 'state': state, 'done': done}
        attempt['domain_sha256'] = _domain_image(root)
    if key == 'task':
        attempt['task_revision'] = record['revision']
    atomic_json(path, attempt)


def _prune_proofs(path: Path) -> None:
    """Keep the newest PROOFS_KEPT completion proofs in a task's folder.

    Started attempts stay: they are what an interrupted save resumes from.
    Best effort - a proof left behind is harmless.
    """
    try:
        entries = [entry for entry in os.scandir(path.parent)
                   if entry.name != path.name and EVIDENCE_NAME.fullmatch(entry.name)
                   and entry.is_file(follow_symlinks=False)]
        if len(entries) < PROOFS_KEPT:
            return
        proofs = []
        for entry in entries:
            try:
                value = _read_attempt(Path(entry.path))
            except BrainError:
                continue
            if value is not None and value["state"] == "complete":
                proofs.append((entry.stat(follow_symlinks=False).st_mtime_ns, entry.name, entry.path))
        proofs.sort()
        for _, _, name in proofs[:max(0, len(proofs) - (PROOFS_KEPT - 1))]:
            os.unlink(name)
    except OSError:
        pass


def _valid_receipt(value: object, task_id: str, result_id: str, digest: str,
                   aliases: tuple[str, ...] = ()) -> bool:
    # Receipts written before the runtime keyed everything by UUID name the
    # task as the caller did, usually by its branch alias; the file is still
    # found under the UUID key, and its digest is the UUID's.
    return (isinstance(value, dict) and set(value) == {"task_id", "result_id", "digest", "task_revision"}
            and value["task_id"] in (task_id, *aliases) and value["result_id"] == result_id
            and value["digest"] == digest
            and (value["task_revision"] is None or (type(value["task_revision"]) is int
                                                   and value["task_revision"] > 0)))


def _holds(task: dict[str, Any], result: dict[str, Any]) -> bool:
    """Whether the task already says what this result would write."""
    return (result["progress"] is None or task.get("progress") == result["progress"]) and (
        result["next_steps"] is None or list(task.get("next_steps") or []) == result["next_steps"]
    )


def _consequence(record: dict[str, Any]) -> str:
    """What a learning record says follows from it, as recorded: the progress
    its closing wrote, or the goal it was created with."""
    return " ".join(str(record.get("progress") or record.get("goal") or "").split())


def _summary(record: dict[str, Any], state: str, *, learning: dict[str, Any],
             identifier: str, allowed_privacy: set[str]) -> dict[str, Any]:
    # Existing aliases survive later privacy/title/owner edits. A completed
    # receipt proves a past save, not permission to disclose the current image.
    # Only already-normalized incoming fields may describe an inaccessible one.
    if record['privacy'] not in allowed_privacy:
        return {'id': None, 'external_id': identifier, 'type': learning['type'],
                'title': learning['title'], 'status': None, 'revision': None,
                'state': 'differs', 'reason': 'not-authorized'}
    return {"id": record["id"], "external_id": record["external_id"], "type": record["type"],
            "title": record["title"], "status": record["status"], "revision": record["revision"],
            "state": state}


def _promotion_summary(root: Path, promotion: dict[str, Any],
                       allowed_privacy: set[str]) -> dict[str, Any]:
    """Project native promotion diagnostics without revealing protected IDs.

    Promotion still runs normally for the whole project. Its blocked/closed
    diagnostics may name private records unrelated to this result, so filtering
    only the submitted aliases is insufficient. One catalogue pass avoids a
    lookup scan per outcome. Missing/ambiguous current identities stay withheld.
    """
    fields = ('promoted', 'failed', 'blocked', 'closed')
    wanted = {item['record_id'] for field in fields for item in promotion[field]}
    if not wanted:
        return promotion
    visible = {}
    for _, record, _ in iter_records(root, include_archive=True):
        identifier = record['id']
        if identifier in wanted:
            visible[identifier] = (identifier not in visible and record['privacy'] in allowed_privacy)
    result = {**promotion, **{field: [item for item in promotion[field]
                                    if visible.get(item['record_id'], False)] for field in fields}}
    if any(len(result[field]) != len(promotion[field]) for field in fields):
        result['details_withheld'] = True
    return result


def record_result(
    root: Path,
    task_id: str,
    result_id: str,
    revision: int | str,
    data: object,
    *,
    owner: str,
    reason: str,
    attestation: str = "agent",
    prepared: Optional[str] = None,
    replay_only: bool = False,
) -> dict[str, Any]:
    """Save once or passively replay a proven completed result.

    Exact complete replay preserves later task/learning edits. A started
    attempt continues at an identical positively observed frontier; any other
    interrupted attempt runs again from the start, every effect idempotent
    against the current records. Numeric revision is task CAS; auto captures
    the current revision under lock. Learning conflicts do not discard
    independent progress or facts. Only this save's own open record is
    finished; no other record is adopted.
    """
    task_id = check_task_id(task_id)
    result_id = check_result_id(result_id)
    automatic_revision = type(revision) is str and revision == "auto"
    if not automatic_revision and (type(revision) is not int or revision < 1):
        raise BrainError("Supply the current task revision")
    if attestation not in ATTESTATIONS:
        raise BrainError("attestation must be agent or person")
    if type(replay_only) is not bool or (replay_only and prepared is None):
        raise BrainError("Receipt-only replay needs a prepared scope")
    if prepared is not None and (not isinstance(prepared, str)
                                 or not re.fullmatch(r"[0-9a-f]{64}", prepared)):
        raise BrainError("Invalid prepared scope")
    # Static shape, text/privacy and source-path policy apply to every
    # request. Current filesystem evidence is needed only for a new write.
    result = normalize(root, data, check_files=False)
    _text(owner, "owner", 200)
    _text(reason, "reason", 2000)
    with mutation_lock(root):
        if prepared is not None and prepared_scope(root, owner, reason, attestation) != prepared:
            raise BrainError("memory-result-v1:scope_changed")
        task, archived_task = _result_task(root, task_id, owner)
        allowed_learning_privacy = set(load_config(root)['allowed_privacy']) & {'public', 'team'}
        if automatic_revision:
            revision = task["revision"]
            if type(revision) is not int or revision < 1:
                raise BrainError("Supply the current task revision")
        # One task, one namespace: its external ID and its UUID both name it,
        # so every key below is derived from the UUID the lookup resolved.
        canonical = task["id"]
        digest = request_digest(canonical, result)
        receipt = _receipt(root, canonical, result_id)
        prior = _read_receipt(receipt)
        try:
            receipt.lstat()
            receipt_present = True
        except FileNotFoundError:
            receipt_present = False
        except OSError:
            receipt_present = True  # Unobservable proof cannot admit a fresh write.
        aliases = (task["external_id"],) if isinstance(task.get("external_id"), str) else ()
        valid_receipt = _valid_receipt(prior, canonical, result_id, digest, aliases)
        attempt_path = _attempt(root, canonical, result_id, digest)
        attempt = _read_attempt(attempt_path)
        resuming = False
        restarted = False
        if (receipt_present and not valid_receipt and attempt is None):
            if (prior is not None and set(prior) == {"task_id", "result_id", "digest", "task_revision"}
                    and prior.get("task_id") in (canonical, *aliases) and prior.get("result_id") == result_id
                    and prior.get("digest") != digest):
                raise BrainError("Result ID was already used with different content")
            raise BrainError("memory-result-v1:receipt_unavailable")
        if attempt is not None:
            if (attempt["task_id"] != canonical or attempt["result_id"] != result_id
                    or attempt["digest"] != digest):
                # The file is named by exactly these three: it was moved or
                # edited, and proves nothing.
                raise BrainError("memory-result-v1:attempt_unavailable")
            if attempt["owner"] != owner or attempt["attestation"] != attestation:
                raise BrainError("memory-result-v1:attempt_scope_changed")
            if attempt["state"] == "started" and not valid_receipt:
                if (attempt.get('profile') != FRONTIER_PROFILE
                        or attempt.get('domain_sha256') is None
                        or attempt['domain_sha256'] != _domain_image(root)
                        or attempt['scope_sha256'] != prepared_scope(root, owner, reason, attestation)
                        or attempt['source_sha256'] != _result_sources_image(root, result)):
                    # The save stopped between an effect and its checkpoint, or
                    # its frontier cannot be observed. Rather than refuse this
                    # result ID for good, run it again from the start: every
                    # effect is idempotent against what is there now - a
                    # learning is created only when absent, finished only when
                    # it is exactly this save's open record, revised only under
                    # the usual proofs, and the task only takes progress it
                    # does not already hold.
                    restarted = True
                else:
                    resuming = True
                    revision = attempt['original_revision']
            if attempt["state"] == "complete":
                prior = {"task_id": canonical, "result_id": result_id, "digest": digest,
                         "task_revision": attempt["task_revision"]}
                valid_receipt = True
        if not valid_receipt:
            prior = None
        # A started attempt with this digest, owner and attestation proves
        # the save was begun; a replay may finish it. Without one, a replay
        # never becomes a first write.
        if replay_only and prior is None and not (resuming or restarted):
            raise BrainError("memory-result-v1:receipt_unavailable")
        if prior is None and (archived_task or task["status"] in ("completed", "cancelled")):
            raise BrainError("Use an active task to record new memory")
        # Auto admits a fresh operation at the current locked task image.
        # An existing result ID still passes the exact proof checks above;
        # this convenience never rebases an ambiguous started attempt.
        task_fields = result["progress"] is not None or result["next_steps"] is not None
        update_task = task_fields and prior is None and not _holds(task, result)
        completed_task_stage = attempt.get('stages', {}).get('task') if resuming else None
        if completed_task_stage is not None:
            if (completed_task_stage['record_id'] != canonical
                    or completed_task_stage['revision'] != task['revision']
                    or completed_task_stage['sha256'] != _learning_image(root, canonical)
                    or not _holds(task, result)):
                raise BrainError("memory-result-v1:attempt_incomplete")
            update_task = False
        # Everything that can refuse the result is checked before the first
        # write: a stale revision - for any new result, learnings alone
        # included, since they too were written against what the agent last
        # read - and every cited file's fingerprint. Only a result with a
        # receipt is a proven replay. The lock is held to the end, so the
        # revision cannot move in between.
        # A restarted save whose task already holds its progress may have
        # written it itself before stopping: the revision it started from
        # still names this save.
        restarted_holds = (restarted and _holds(task, result)
                           and revision == attempt.get('original_revision'))
        if prior is None and completed_task_stage is None and task["revision"] != revision and not restarted_holds:
            if prepared is not None:
                raise BrainError("memory-result-v1:prepared_stale_no_write")
            raise BrainError(
                f"Stale task revision: expected {revision}, current {task['revision']}. "
                "Nothing was written; replay this result ID with the current revision."
            )
        plan = []
        identities = {}
        held = {}
        implicit_cas = set()
        owned_created = set()
        if prior is None:
            if resuming and set(attempt['stages']) - {'task'} - {
                    learning_id(canonical, learning) for learning in result['learnings']}:
                raise BrainError('memory-result-v1:attempt_incomplete')
            for learning in result["learnings"]:
                identifier = learning_id(canonical, learning)
                if identifier in identities and identities[identifier] != learning:
                    raise BrainError("Conflicting learning identities in one result")
                identities[identifier] = learning
                for source in learning["sources"]:
                    check_source(root, source)
                fingerprints = source_fingerprints(root, learning["sources"])
                current, archived = _existing(root, identifier)
                stage = attempt.get('stages', {}).get(identifier) if resuming else None
                if stage is not None:
                    if (current is None or archived or current['type'] != learning['type']
                            or stage['record_id'] != current['id']
                            or stage['revision'] != current['revision']
                            or stage['sha256'] != _learning_image(root, identifier)):
                        raise BrainError("memory-result-v1:attempt_incomplete")
                    if stage['done']:
                        if (current['status'] != CLOSED_STATE[learning['type']]
                                or current['authority'] != 'verified'
                                or _consequence(current) != ' '.join(learning['consequence'].split())):
                            raise BrainError('memory-result-v1:attempt_incomplete')
                    elif (stage['state'] != 'created' or current['revision'] != 1
                          or current['status'] != LIFECYCLES[learning['type']]['initial']
                          or current['authority'] != 'observed'
                          or current['goal'] != learning['consequence']
                          or sorted(current['sources']) != sorted(learning['sources'])):
                        raise BrainError('memory-result-v1:attempt_incomplete')
                    if not stage['done']:
                        owned_created.add(identifier)
                expected = learning.get("expected_revision")
                unfinished = current is not None and (
                    current["status"] == LIFECYCLES[learning["type"]]["initial"]
                    or (current["status"] == CLOSED_STATE[learning["type"]] and current["authority"] != "verified"))
                same = current is not None and _consequence(current) == " ".join(learning["consequence"].split())
                if current is not None and current['privacy'] not in allowed_learning_privacy:
                    held[identifier] = 'not-authorized'
                elif archived:
                    held[identifier] = "archived"
                elif current is not None and current['type'] != learning['type']:
                    held[identifier] = "kind-conflict"
                elif expected is not None and current is not None and current["revision"] != expected:
                    # A learning that does not exist yet has no revision to
                    # conflict with: it is created, not held as "kept".
                    held[identifier] = "revision-conflict"
                elif current is not None and owner not in current["authorized_owners"]:
                    held[identifier] = "not-authorized"
                elif (current is not None and attestation == "agent" and current["authority"] == "verified"
                      and (unfinished or not same) and record_attestation(current) != "agent"):
                    held[identifier] = "person-or-unknown-attestation"
                elif current is not None and expected is None and (unfinished or not same) and identifier not in owned_created:
                    closed = not unfinished and current["status"] == CLOSED_STATE[learning["type"]]
                    if closed and attestation == "person":
                        # A person's attested correction is the person's to
                        # make; it needs no proof of the agent's last image.
                        implicit_cas.add(identifier)
                    elif (closed and current["authority"] == "verified" and attestation == "agent"
                            and record_attestation(current) == "agent"
                            and _owned_learning_image(root, identifier, current, owner)):
                        # A missing image may be a failed write or a later
                        # deletion, not evidence that a legacy record is still
                        # ours to correct. Without proof, require explicit CAS.
                        implicit_cas.add(identifier)
                    elif unfinished and _own_open_creation(current, learning, owner):
                        # An interrupted save created this record and stopped
                        # before closing it. It holds exactly what this
                        # learning says, so this result finishes it instead
                        # of leaving it open and unpromoted for good.
                        owned_created.add(identifier)
                    else:
                        held[identifier] = "original-image-unproven"
                plan.append({"learning_id": identifier, "record_id": current["id"] if current else None,
                             "revision": current["revision"] if current else None,
                             "archived": archived, "fingerprints": fingerprints})
            if not resuming:
                attempt = {"profile": FRONTIER_PROFILE, "task_id": canonical, "result_id": result_id,
                       "digest": digest, "original_revision": revision, "owner": owner,
                       "attestation": attestation, "state": "started", "task_revision": None,
                       "plan_sha256": hashlib.sha256(json.dumps(
                           {"task": task, "learnings": plan}, sort_keys=True,
                           ensure_ascii=False).encode("utf-8")).hexdigest(),
                        'scope_sha256': prepared_scope(root, owner, reason, attestation),
                        'source_sha256': _result_sources_image(root, result),
                        'domain_sha256': _domain_image(root), 'stages': {}}
                _attempt_directory(attempt_path)
                atomic_json(attempt_path, attempt)
        records = []
        wrote = False
        emitted = set()
        for learning in result["learnings"]:
            identifier = learning_id(canonical, learning)
            if identifier in emitted:
                continue
            emitted.add(identifier)
            current, archived = _existing(root, identifier)
            def summary(state):
                return _summary(current, state, learning=learning, identifier=identifier,
                                allowed_privacy=allowed_learning_privacy)
            # Before archive/revision/stage distinctions, hold inaccessible
            # images without disclosing or updating them. An ambiguous started
            # frontier has already failed its unchanged-domain admission above.
            if current is not None and current['privacy'] not in allowed_learning_privacy:
                records.append(summary('differs'))
                continue
            created = current is None
            closed_state = CLOSED_STATE[learning["type"]]
            state = "existing"
            same = current is not None and _consequence(current) == " ".join(learning["consequence"].split())
            stage = attempt.get('stages', {}).get(identifier) if resuming else None
            if stage is not None and stage['done']:
                records.append(summary(stage['state']))
                continue
            if archived:
                # An archived record is never rewritten; a different
                # consequence is said, not dropped behind "already there".
                records.append(summary("archived" if same else "archived-differs"))
                continue
            if identifier in held:
                item = (summary("differs") if current is not None else
                        {"id": None, "external_id": identifier, "type": learning["type"],
                         "title": learning["title"], "status": None, "revision": None, "state": "differs"})
                records.append({**item, "reason": held[identifier]})
                continue
            if prior is not None:
                # A proven replay: this result was recorded in full before its
                # receipt was written, so whatever its records say now is what
                # happened since - a later result's revision, a person's
                # correction - and a delayed retry never rolls that back.
                # Only the promotion below is tried again.
                if current is None:
                    records.append({"id": None, "external_id": identifier, "type": learning["type"],
                                    "title": learning["title"], "status": None, "revision": None,
                                    "state": "missing"})
                else:
                    records.append(summary("existing" if same else "differs"))
                continue
            if current is None:
                current = create_record(
                    root, learning["type"], identifier, learning["title"], [],
                    learning["sources"], owner=owner, authority="observed",
                    goal=learning["consequence"],
                )
                state = "created"
                _checkpoint_attempt(root, attempt_path, attempt, identifier, current, state, False)
            # Only a record still in its initial state is this result's to
            # finish: created just now, or left open by an attempt that
            # stopped between writing and closing it. One that has moved on
            # since (superseded, under investigation) is someone's later work.
            unfinished = current["status"] == LIFECYCLES[learning["type"]]["initial"] or (
                current["status"] == closed_state and current["authority"] != "verified")
            # Neither matching content nor a fresh lookup proves an old
            # unfinished record belongs to this attempt. Deliberate repair
            # needs its original record revision, not a reloaded one.
            if unfinished and not created and "expected_revision" not in learning and identifier not in owned_created:
                state = "differs"
            elif unfinished:
                if owner not in current["authorized_owners"]:
                    raise BrainError("An earlier attempt's record belongs to another owner")
                if (not created and attestation == "agent" and current["authority"] == "verified"
                        and record_attestation(current) != "agent"):
                    raise BrainError("memory-result-v1:learning_correction_not_authorized")
                current = update_record(
                    root, current["id"], expected_revision=current["revision"],
                    progress=learning["consequence"], next_steps=[], files=[], sources=[],
                    actor=owner, reason=reason,
                    authority="verified" if current["authority"] != "verified" else None,
                    transition_to=closed_state if current["status"] != closed_state else None,
                    reverify=not created and current["authority"] == "verified",
                    attestation=attestation,
                )
                state = "created" if state == "created" else "completed"
            elif not same:
                # The same learning with a revised consequence: the record
                # takes the new one when an agent revises what an agent
                # attested, or a person revises it. What a person verified -
                # or nobody said who - an agent does not overwrite, and a
                # record of someone else's or one that has moved on is left
                # too; the response says so instead of "already there". The
                # revised claim is verified again in the record's ledger, so
                # a person's correction is the person's from then on.
                revisable = attestation == "person" or record_attestation(current) == "agent"
                if (("expected_revision" not in learning and identifier not in implicit_cas)
                        or not revisable or owner not in current["authorized_owners"]
                        or current["status"] != closed_state):
                    state = "differs"
                else:
                    current = update_record(
                        root, current["id"], expected_revision=current["revision"],
                        progress=learning["consequence"], next_steps=[], files=[], sources=[],
                        actor=owner, reason=reason, reverify=True, attestation=attestation,
                    )
                    state = "updated"
            if state in ("created", "completed", "updated"):
                _remember_learning_image(root, identifier, current, owner)
                _checkpoint_attempt(root, attempt_path, attempt, identifier, current, state, True)
            wrote = wrote or state not in ("existing", "differs")
            records.append(summary(state))
        updated = task if task_fields else None
        if update_task:
            updated = update_record(
                root, task["id"], expected_revision=revision,
                progress=result["progress"], next_steps=result["next_steps"] or [],
                files=[], sources=[], actor=owner, reason=reason,
                replace_next_steps=result["next_steps"] is not None,
            )
            wrote = True
            _checkpoint_attempt(root, attempt_path, attempt, 'task', updated, 'task', True)
        completed_revision = updated["revision"] if updated else None
        if attempt is not None and attempt["state"] == "started":
            atomic_json(attempt_path, {"profile": PROOF_PROFILE, "task_id": canonical, "result_id": result_id,
                                       "digest": digest, "owner": owner, "attestation": attestation,
                                       "state": "complete"})
            _prune_proofs(attempt_path)
        receipt.parent.mkdir(parents=True, exist_ok=True)
        atomic_json(receipt, {"result_id": result_id, "task_id": canonical, "digest": digest,
                              "task_revision": completed_revision})
        # Promotion runs for any result with learnings, a replay included:
        # an attempt interrupted after its records were written must still
        # reach the Memory Bank, and promotion is itself idempotent.
        promotion = (_promotion_summary(root, auto_promote(root, owner=owner), allowed_learning_privacy)
                     if result["learnings"] else None)
    return {
        "task": {"id": updated["id"], "revision": updated["revision"]} if updated else None,
        "records": records,
        "promotion": promotion,
        "attestation": attestation,
        # A receipt proves the result was recorded before; without one,
        # nothing was written because every part of it was already there -
        # not because a part of it was refused.
        "replayed": prior is not None or (not wrote and not any(
            item["state"] in ("differs", "archived-differs") for item in records)),
    }
