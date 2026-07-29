#!/usr/bin/env python3
"""Governed FTS5 indexing, budgeting, and retrieval manifests."""

from __future__ import annotations

import hashlib
import json
import re
import sqlite3
from pathlib import Path
from typing import Any, Iterable, Optional

from brain_runtime import (
    BrainError,
    LIFECYCLES,
    atomic_json,
    brain_root,
    get_task,
    iter_records,
    load_config,
    mutation_lock,
    new_uuid,
    parse_markdown_record,
    record_is_eligible,
    sources_are_fresh,
    utc_now,
    validate_handoff,
    validate_record,
    validate_schema_file,
)


BUDGETS = {
    "policy": 1200,
    "handoff": 1500,
    "durable": 3500,
    "dynamic": 1500,
    "evidence": 2000,
}
TARGET_BUDGET = 8000
HARD_BUDGET = 12000
MAX_SNIPPET_CHARS = 1200
SKILL_EDITIONS = (".agents", ".claude", ".cursor", ".codex")


class RetrievalError(Exception):
    """A safe, user-facing retrieval error."""


def ensure_metadata_tables(connection: sqlite3.Connection) -> None:
    connection.execute(
        """
        CREATE TABLE IF NOT EXISTS document_metadata(
            path TEXT PRIMARY KEY,
            category TEXT NOT NULL,
            privacy TEXT NOT NULL,
            owner TEXT NOT NULL,
            authority TEXT NOT NULL,
            lifecycle TEXT NOT NULL,
            source_hash TEXT NOT NULL,
            record_id TEXT,
            conflicts TEXT NOT NULL,
            source_fingerprints TEXT NOT NULL DEFAULT '[]'
        )
        """
    )
    metadata_columns = {
        row["name"]
        for row in connection.execute("PRAGMA table_info(document_metadata)")
    }
    if "source_fingerprints" not in metadata_columns:
        connection.execute(
            """
            ALTER TABLE document_metadata
            ADD COLUMN source_fingerprints TEXT NOT NULL DEFAULT '[]'
            """
        )
    connection.execute(
        """
        CREATE TABLE IF NOT EXISTS task_bindings(
            external_id TEXT PRIMARY KEY,
            task_uuid TEXT NOT NULL UNIQUE,
            revision INTEGER NOT NULL,
            updated_at TEXT NOT NULL
        )
        """
    )


def category_for(kind: str) -> str:
    if kind in {"policy", "skill"}:
        return "policy"
    if kind == "memory":
        return "durable"
    if kind == "brain-handoff":
        return "handoff"
    if kind.startswith("brain-"):
        return "dynamic"
    return "evidence"


def _content_hash(content: str) -> str:
    return hashlib.sha256(content.encode("utf-8")).hexdigest()


def skill_mirror_drift(repository: Path, canonical_edition: str) -> list[dict[str, object]]:
    logical: dict[str, dict[str, Path]] = {}
    for edition in SKILL_EDITIONS:
        root = repository / edition / "skills"
        if not root.is_dir():
            continue
        for path in sorted(root.glob("**/*.md")):
            if path.is_file() and not path.is_symlink():
                key = path.relative_to(root).as_posix()
                # Edition orchestration catalogs are wrappers, not mirrored
                # skill implementations. Their wording is intentionally owned
                # by each tool edition and is outside runtime parity.
                if key == "SKILL FLOW.md":
                    continue
                logical.setdefault(key, {})[edition] = path
    drift: list[dict[str, object]] = []
    for key, copies in sorted(logical.items()):
        if canonical_edition not in copies:
            continue
        canonical = copies[canonical_edition].read_bytes()
        drifted = [
            edition for edition, path in copies.items()
            if edition != canonical_edition and path.read_bytes() != canonical
        ]
        if drifted:
            drift.append(
                {
                    "logical_path": key,
                    "canonical": canonical_edition,
                    "mismatched": sorted(drifted),
                }
            )
    return drift


def assert_skill_mirror_parity(repository: Path, canonical_edition: str) -> None:
    drift = skill_mirror_drift(repository, canonical_edition)
    if drift:
        first = drift[0]
        raise RetrievalError(
            f"Skill mirror parity drift for {first['logical_path']}: "
            f"canonical {first['canonical']}, mismatched "
            f"{', '.join(first['mismatched'])}"
        )


def _legacy_metadata(path: str, kind: str, content: str) -> tuple[object, ...]:
    return (
        path, category_for(kind), "public", "*", "verified", "active",
        _content_hash(content), None, "[]", "[]",
    )


def _brain_documents(
    repository: Path, config: dict[str, Any]
) -> tuple[list[tuple[str, str, str, str, str]], list[tuple[object, ...]], list[dict[str, str]]]:
    documents: list[tuple[str, str, str, str, str]] = []
    metadata_rows: list[tuple[object, ...]] = []
    excluded: list[dict[str, str]] = []
    eligible_tasks: dict[str, dict[str, Any]] = {}
    for path, record, body in iter_records(repository):
        relative = path.relative_to(repository).as_posix()
        try:
            validate_record(record)
            eligible, reason = record_is_eligible(repository, record, config)
        except BrainError:
            eligible, reason = False, "invalid"
        if not eligible:
            excluded.append({"path": relative, "reason": reason})
            continue
        eligible_tasks[record["id"]] = record
        title = record["title"]
        documents.append((relative, "semantic", f"brain-{record['type']}", title, body))
        metadata_rows.append(
            (
                relative, "dynamic", record["privacy"], record["owner"], record["authority"],
                record["status"],
                _content_hash(path.read_text(encoding="utf-8")),
                record["id"],
                json.dumps(record["conflicts"], sort_keys=True),
                json.dumps(record["source_fingerprints"], sort_keys=True),
            )
        )
    handoffs = brain_root(repository) / "control" / "handoffs"
    if handoffs.is_dir():
        for path in sorted(handoffs.glob("*.md")):
            relative = path.relative_to(repository).as_posix()
            try:
                handoff, body = parse_markdown_record(path)
                task = eligible_tasks[handoff["task_id"]]
                validate_handoff(handoff, task)
            except (BrainError, KeyError):
                excluded.append({"path": relative, "reason": "task-filter-or-invalid"})
                continue
            documents.append(
                (relative, "semantic", "brain-handoff", f"Handoff {task['external_id']}", body)
            )
            metadata_rows.append(
                (
                    relative, "handoff", task["privacy"], task["owner"], task["authority"],
                    handoff["status"],
                    _content_hash(path.read_text(encoding="utf-8")),
                    handoff["id"],
                    "[]",
                    json.dumps(task["source_fingerprints"], sort_keys=True),
                )
            )
    return documents, metadata_rows, excluded


def index_documents(
    connection: sqlite3.Connection,
    repository: Path,
    legacy_documents: list[tuple[str, str, str, str, str]],
) -> dict[str, object]:
    config = load_config(repository)
    parity_drift = skill_mirror_drift(repository, str(config["canonical_edition"]))
    brain_documents, brain_metadata, excluded = _brain_documents(repository, config)
    documents = [*legacy_documents, *brain_documents]
    metadata = [
        _legacy_metadata(path, kind, content)
        for path, _, kind, _, content in legacy_documents
    ] + brain_metadata
    indexed_paths = {item[0] for item in documents}
    ensure_metadata_tables(connection)
    with connection:
        existing = {
            row[0] for row in connection.execute("SELECT path FROM documents").fetchall()
        }
        connection.execute("DELETE FROM documents")
        connection.execute("DELETE FROM document_metadata")
        connection.executemany(
            "INSERT INTO documents(path, layer, kind, title, content) VALUES (?, ?, ?, ?, ?)",
            documents,
        )
        connection.executemany(
            """
            INSERT INTO document_metadata(
                path, category, privacy, owner, authority, lifecycle, source_hash,
                record_id, conflicts, source_fingerprints
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            metadata,
        )
    layers = ("procedural", "semantic", "episodic")
    return {
        "documents": len(documents),
        "removed": len(existing - indexed_paths),
        "layers": {
            layer: sum(document[1] == layer for document in documents) for layer in layers
        },
        "brain": len(brain_documents),
        "excluded": excluded,
        "canonical_edition": config["canonical_edition"],
        "parity_drift": parity_drift,
    }


def fts_query(query: str) -> str:
    tokens = re.findall(r"\w+", query, flags=re.UNICODE)
    if not tokens:
        raise RetrievalError("Search query must contain a word")
    return " OR ".join(f'"{token}"' for token in tokens)


def _estimate_tokens(value: str) -> int:
    return max(1, (len(value) + 3) // 4)


def _candidates(
    connection: sqlite3.Connection, query: str, limit: int = 100
) -> list[dict[str, Any]]:
    ensure_metadata_tables(connection)
    rows = connection.execute(
        """
        SELECT
            d.path, d.layer, d.kind, d.title,
            snippet(documents, 4, '[', ']', ' … ', 32) AS snippet,
            m.category, m.privacy, m.owner, m.authority, m.lifecycle,
            m.source_hash, m.record_id, m.conflicts,
            m.source_fingerprints,
            bm25(documents) AS score
        FROM documents AS d
        JOIN document_metadata AS m ON m.path = d.path
        WHERE documents MATCH ?
        ORDER BY score, d.path
        LIMIT ?
        """,
        (fts_query(query), limit),
    ).fetchall()
    result = []
    for row in rows:
        item = dict(row)
        item["snippet"] = str(item["snippet"])[:MAX_SNIPPET_CHARS]
        item["conflicts"] = json.loads(item["conflicts"])
        item["source_fingerprints"] = json.loads(item["source_fingerprints"])
        item["estimated_tokens"] = _estimate_tokens(item["title"] + item["snippet"])
        result.append(item)
    return result


def _conflict_candidates(
    connection: sqlite3.Connection, record_ids: set[str]
) -> list[dict[str, Any]]:
    if not record_ids:
        return []
    placeholders = ", ".join("?" for _ in record_ids)
    rows = connection.execute(
        f"""
        SELECT
            d.path, d.layer, d.kind, d.title,
            substr(d.content, 1, ?) AS snippet,
            m.category, m.privacy, m.owner, m.authority, m.lifecycle,
            m.source_hash, m.record_id, m.conflicts,
            m.source_fingerprints,
            0.0 AS score
        FROM documents AS d
        JOIN document_metadata AS m ON m.path = d.path
        WHERE m.record_id IN ({placeholders})
        ORDER BY d.path
        """,
        (MAX_SNIPPET_CHARS, *sorted(record_ids)),
    ).fetchall()
    result = []
    for row in rows:
        item = dict(row)
        item["snippet"] = str(item["snippet"])[:MAX_SNIPPET_CHARS]
        item["conflicts"] = json.loads(item["conflicts"])
        item["source_fingerprints"] = json.loads(item["source_fingerprints"])
        item["estimated_tokens"] = _estimate_tokens(
            item["title"] + item["snippet"]
        )
        result.append(item)
    return result


def _runtime_filter(
    repository: Path,
    candidates: Iterable[dict[str, Any]],
    config: dict[str, Any],
) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    selected = []
    excluded = []
    owners = set(config.get("owners", ["*"]))
    for candidate in candidates:
        reason: Optional[str] = None
        if candidate["privacy"] not in config["allowed_privacy"]:
            reason = "privacy"
        elif (
            candidate["privacy"] == "restricted"
            and "*" not in owners
            and candidate["owner"] not in owners
        ):
            reason = "owner"
        elif candidate["authority"] not in config["allowed_authority"]:
            reason = "authority"
        elif candidate["kind"] == "brain-handoff" and candidate["lifecycle"] == "closed":
            reason = "lifecycle"
        elif candidate["kind"].startswith("brain-"):
            record_type = candidate["kind"].removeprefix("brain-")
            if (
                record_type not in LIFECYCLES
                or candidate["lifecycle"] in LIFECYCLES[record_type]["terminal"]
            ):
                reason = "lifecycle"
        if reason is None:
            path = repository / candidate["path"]
            try:
                fresh = path.is_file() and _content_hash(path.read_text(encoding="utf-8")) == candidate["source_hash"]
            except OSError:
                fresh = False
            if not fresh:
                reason = "stale"
        if (
            reason is None
            and candidate["kind"].startswith("brain-")
            and candidate["source_fingerprints"]
        ):
            fingerprints = candidate["source_fingerprints"]
            fresh = sources_are_fresh(
                repository,
                {
                    "sources": [item["path"] for item in fingerprints],
                    "source_fingerprints": fingerprints,
                },
            )
            if not fresh:
                reason = "stale"
        if reason:
            excluded.append({"path": candidate["path"], "reason": reason})
        else:
            selected.append(candidate)
    return selected, excluded


def _apply_budgets(
    candidates: list[dict[str, Any]]
) -> tuple[
    list[dict[str, Any]], list[dict[str, str]], dict[str, int], Optional[str]
]:
    selected: list[dict[str, Any]] = []
    excluded: list[dict[str, str]] = []
    usage = {category: 0 for category in BUDGETS}
    total = 0
    required_conflicts = {
        record_id
        for item in candidates
        if item.get("record_id") is not None and item["conflicts"]
        for record_id in [item["record_id"], *item["conflicts"]]
    }
    queue = list(candidates)
    escalation_reasons: list[str] = []
    for item in queue:
        category = item["category"]
        cost = item["estimated_tokens"]
        over_normal = (
            usage[category] + cost > BUDGETS[category]
            or total + cost > TARGET_BUDGET
        )
        required = item.get("record_id") in required_conflicts
        if over_normal and required and total + cost <= HARD_BUDGET:
            escalation_reasons.append(
                f"conflict pair {item['record_id']} retained beyond normal budget"
            )
        elif over_normal:
            reason = "hard-budget" if required else "budget"
            excluded.append({"path": item["path"], "reason": "budget"})
            if required:
                excluded[-1]["reason"] = reason
                escalation_reasons.append(
                    f"conflict pair {item['record_id']} could not fit hard ceiling"
                )
            continue
        selected.append(item)
        usage[category] += cost
        total += cost
    usage["total"] = total
    usage["target"] = TARGET_BUDGET
    usage["hard"] = HARD_BUDGET
    escalation = "; ".join(dict.fromkeys(escalation_reasons)) or None
    return selected, excluded, usage, escalation


def retrieve(
    connection: sqlite3.Connection,
    repository: Path,
    query: str,
    task_identifier: str,
    *,
    limit: int,
    provider: Optional[str] = None,
) -> dict[str, Any]:
    if limit < 1:
        raise RetrievalError("--limit must be a positive integer")
    task = get_task(repository, task_identifier)
    config = load_config(repository)
    candidates = _candidates(connection, query, max(20, limit * 10))
    filtered, filter_excluded = _runtime_filter(repository, candidates, config)
    known_ids = {
        item["record_id"] for item in filtered if item.get("record_id") is not None
    }
    pending = {
        conflict_id
        for item in filtered
        for conflict_id in item["conflicts"]
        if conflict_id not in known_ids
    }
    while pending:
        requested = set(pending)
        conflict_candidates = _conflict_candidates(connection, requested)
        eligible_conflicts, conflict_excluded = _runtime_filter(
            repository, conflict_candidates, config
        )
        filter_excluded.extend(conflict_excluded)
        found_ids = {
            item["record_id"]
            for item in conflict_candidates
            if item.get("record_id") is not None
        }
        eligible_ids = {
            item["record_id"]
            for item in eligible_conflicts
            if item.get("record_id") is not None
        }
        for missing_id in sorted(requested - found_ids):
            filter_excluded.append(
                {"path": f"record:{missing_id}", "reason": "conflict-unavailable"}
            )
        filtered.extend(eligible_conflicts)
        known_ids.update(eligible_ids)
        pending = {
            conflict_id
            for item in eligible_conflicts
            for conflict_id in item["conflicts"]
            if conflict_id not in known_ids
        }
    selected, budget_excluded, usage, escalation_reason = _apply_budgets(filtered)
    manifest_id = new_uuid()
    manifest = {
        "schema_version": 1,
        "id": manifest_id,
        "created_at": utc_now(),
        "query": query,
        "task_id": task["id"],
        "task_revision": task["revision"],
        "filters": {
            "privacy": config["allowed_privacy"],
            "owners": config["owners"],
            "authority": config["allowed_authority"],
            "freshness": True,
            "active_only": True,
        },
        "selected": [
            {
                "path": item["path"], "category": item["category"],
                "estimated_tokens": item["estimated_tokens"], "source_hash": item["source_hash"],
            }
            for item in selected
        ],
        "excluded": [*filter_excluded, *budget_excluded],
        "token_estimates": usage,
        "provider": provider or config["provider"],
        "escalation_reason": escalation_reason,
    }
    manifest_path = (
        brain_root(repository) / "control" / "retrieval-manifests" / f"{manifest_id}.json"
    )
    validate_schema_file(
        repository, "retrieval-manifest.schema.json", manifest
    )
    with mutation_lock(repository):
        atomic_json(manifest_path, manifest)
    groups = {category: [] for category in BUDGETS}
    for item in selected:
        public = {
            key: item[key]
            for key in (
                "path", "layer", "kind", "title", "snippet", "category",
                "estimated_tokens", "record_id", "conflicts",
            )
        }
        groups[item["category"]].append(public)
    procedural = groups["policy"][:limit]
    semantic = [
        *groups["handoff"], *groups["durable"], *groups["dynamic"], *groups["evidence"]
    ][:limit]
    return {
        "query": query,
        "task_id": task["external_id"],
        "task_uuid": task["id"],
        "task_revision": task["revision"],
        "working": {
            "task_id": task["external_id"], "goal": task["goal"], "progress": task["progress"],
            "next_steps": task["next_steps"], "files": task["files"], "sources": task["sources"],
            "created_at": task["created_at"], "updated_at": task["updated_at"],
        },
        "categories": groups,
        "procedural": procedural,
        "semantic": semantic,
        "episodic": [],
        "selected": selected,
        "token_estimates": usage,
        "manifest": manifest_path.relative_to(repository).as_posix(),
    }
