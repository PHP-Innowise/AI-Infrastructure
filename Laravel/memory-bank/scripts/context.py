#!/usr/bin/env python3
"""Canonical Project Brain + repository-local context CLI facade."""

from __future__ import annotations

import argparse
import json
import os
import re
import sqlite3
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional

from brain_runtime import (
    BrainError,
    cancel_task,
    close_task,
    compact,
    configured_mode,
    create_promotion,
    create_record,
    create_task,
    get_record,
    get_task,
    load_config,
    mutation_lock,
    rollback_created_record,
    review_promotion,
    apply_promotion,
    restore_record_state,
    snapshot_record_state,
    update_record,
    update_task,
    validate_repository,
)
from context_retrieval import (
    RetrievalError,
    assert_skill_mirror_parity,
    ensure_metadata_tables,
    index_documents,
    retrieve,
)
from validate import (
    SECRET_PATTERNS,
    ValidationError,
    parse_frontmatter,
    validate_metadata,
    validate_secret_patterns,
)


class ContextError(Exception):
    """A safe, user-facing context-engine error."""


SOURCE_PATTERNS = (
    ("procedural", "policy", "AGENTS.md"),
    ("procedural", "policy", "CLAUDE.md"),
    ("procedural", "skill", ".agents/skills/**/*.md"),
    ("procedural", "skill", ".claude/skills/**/*.md"),
    ("procedural", "skill", ".cursor/skills/**/*.md"),
    ("procedural", "skill", ".codex/skills/**/*.md"),
    ("semantic", "overview", "README.md"),
    ("semantic", "spec", "specs/**/*.md"),
    ("semantic", "spec", "docs/**/*.md"),
    ("semantic", "memory", "memory-bank/chunks/*.md"),
    ("semantic", "task", "tasks/**/*.md"),
    ("semantic", "capability", "Task/Epics/**/*.md"),
    ("episodic", "changelog", "CHANGELOG.md"),
)
DOCUMENT_LAYERS = ("procedural", "semantic", "episodic")
TASK_ID_PATTERN = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._/-]{0,127}$")


def default_root() -> Path:
    return Path(__file__).resolve().parents[2]


def default_database(repository: Path) -> Path:
    return repository / "memory-bank" / "local" / "context.db"


def connect(database: Path) -> sqlite3.Connection:
    database.parent.mkdir(parents=True, exist_ok=True)
    connection = sqlite3.connect(database)
    connection.row_factory = sqlite3.Row
    try:
        connection.execute("BEGIN IMMEDIATE")
        document_columns = {
            row["name"] for row in connection.execute("PRAGMA table_info(documents)")
        }
        if document_columns and "layer" not in document_columns:
            connection.execute("DROP TABLE documents")
            document_columns = set()
        if not document_columns:
            create_document_table(connection)
        episode_schema = connection.execute(
            "SELECT sql FROM sqlite_master WHERE name = 'episodes'"
        ).fetchone()
        if episode_schema is None:
            create_episode_table(connection)
        elif any(
            re.search(
                rf"\b{column}\s+UNINDEXED\b",
                episode_schema["sql"],
                flags=re.IGNORECASE,
            )
            for column in ("files", "verification", "sources")
        ):
            migrate_episode_table(connection)
        connection.execute(
            """
            CREATE TABLE IF NOT EXISTS working_tasks(
                task_id TEXT PRIMARY KEY,
                goal TEXT NOT NULL,
                progress TEXT NOT NULL,
                next_steps TEXT NOT NULL,
                files TEXT NOT NULL,
                sources TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )
            """
        )
        ensure_metadata_tables(connection)
        connection.commit()
    except Exception as error:
        connection.rollback()
        connection.close()
        if isinstance(error, sqlite3.OperationalError) and "fts5" in str(error).lower():
            raise ContextError("SQLite FTS5 support is required") from error
        raise
    return connection


def create_document_table(connection: sqlite3.Connection) -> None:
    connection.execute(
        """
        CREATE VIRTUAL TABLE documents USING fts5(
            path UNINDEXED,
            layer UNINDEXED,
            kind UNINDEXED,
            title,
            content,
            tokenize = 'unicode61'
        )
        """
    )


def create_episode_table(connection: sqlite3.Connection) -> None:
    connection.execute(
        """
        CREATE VIRTUAL TABLE episodes USING fts5(
            summary,
            outcome,
            files,
            verification,
            sources,
            created_at UNINDEXED,
            tokenize = 'unicode61'
        )
        """
    )


def migrate_episode_table(connection: sqlite3.Connection) -> None:
    rows = connection.execute(
        """
        SELECT rowid, summary, outcome, files, verification, sources, created_at
        FROM episodes
        """
    ).fetchall()
    connection.execute("DROP TABLE episodes")
    create_episode_table(connection)
    connection.executemany(
        """
        INSERT INTO episodes(
            rowid, summary, outcome, files, verification, sources, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?)
        """,
        rows,
    )


def document_title(path: Path, content: str) -> str:
    for line in content.splitlines():
        if line.startswith("# "):
            return line[2:].strip()
    return path.stem.replace("-", " ").replace("_", " ")


def active_memory(path: Path, repository: Path) -> bool:
    try:
        metadata = parse_frontmatter(path)
        validate_metadata(path, metadata, repository)
        validate_secret_patterns(path)
    except (OSError, ValidationError):
        return False
    return metadata["status"] == "active"


def git_ignored_paths(repository: Path, paths: list[str]) -> set[bytes]:
    """Return untracked ignored candidates without reading their contents."""
    if not paths:
        return set()
    try:
        result = subprocess.run(
            ["git", "-C", str(repository), "check-ignore", "--stdin", "-z"],
            input=b"".join(os.fsencode(path) + b"\0" for path in paths),
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            check=False,
        )
    except OSError:
        return set()
    if result.returncode not in (0, 1):
        return set()
    return set(result.stdout.split(b"\0")) - {b""}


def discover_documents(repository: Path) -> list[tuple[str, str, str, str, str]]:
    documents: list[tuple[str, str, str, str, str]] = []
    skill_keys: set[str] = set()
    candidates: list[tuple[str, str, Path, str]] = []
    for layer, kind, pattern in SOURCE_PATTERNS:
        for path in sorted(repository.glob(pattern)):
            if not path.is_file() or path.is_symlink():
                continue
            candidates.append(
                (layer, kind, path, path.relative_to(repository).as_posix())
            )

    ignored_paths = git_ignored_paths(
        repository, [relative_path for _, _, _, relative_path in candidates]
    )
    for layer, kind, path, relative_path in candidates:
        if os.fsencode(relative_path) in ignored_paths:
            continue
        try:
            if kind == "memory":
                if not active_memory(path, repository):
                    continue
            else:
                try:
                    validate_secret_patterns(path)
                except (OSError, ValidationError):
                    continue
            content = path.read_text(encoding="utf-8")
        except UnicodeDecodeError as error:
            raise ContextError(
                f"Source document is not valid UTF-8: {relative_path}"
            ) from error
        if kind == "skill":
            skill_key = relative_path.split("/skills/", maxsplit=1)[1]
            if skill_key in skill_keys:
                continue
            skill_keys.add(skill_key)
        documents.append(
            (
                relative_path,
                layer,
                kind,
                document_title(path, content),
                content,
            )
        )
    return documents


def index_repository(connection: sqlite3.Connection, repository: Path) -> dict[str, object]:
    documents = discover_documents(repository)
    return index_documents(connection, repository, documents)


def fts_query(query: str) -> str:
    # ponytail: FTS5-only retrieval; add local embeddings only after a
    # golden-query evaluation shows lexical recall is insufficient.
    tokens = re.findall(r"\w+", query, flags=re.UNICODE)
    if not tokens:
        raise ContextError("Search query must contain a word")
    return " OR ".join(f'"{token}"' for token in tokens)


def search_documents(
    connection: sqlite3.Connection,
    query: str,
    limit: int,
    layer: Optional[str] = None,
) -> list[dict[str, object]]:
    conditions = ["documents MATCH ?"]
    parameters: list[object] = [fts_query(query)]
    if layer is not None:
        conditions.append("layer = ?")
        parameters.append(layer)
    parameters.append(limit)
    rows = connection.execute(
        f"""
        SELECT
            path,
            layer,
            kind,
            title,
            snippet(documents, 4, '[', ']', ' … ', 18) AS snippet
        FROM documents
        WHERE {' AND '.join(conditions)}
        ORDER BY bm25(documents), path
        LIMIT ?
        """,
        parameters,
    ).fetchall()
    return [dict(row) for row in rows]


def search_episodes(
    connection: sqlite3.Connection,
    query: str,
    limit: int,
) -> list[dict[str, object]]:
    rows = connection.execute(
        """
        SELECT
            rowid AS id,
            summary,
            outcome,
            files,
            verification,
            sources,
            created_at
        FROM episodes
        WHERE episodes MATCH ?
        ORDER BY bm25(episodes), rowid DESC
        LIMIT ?
        """,
        (fts_query(query), limit),
    ).fetchall()
    return [
        {
            "id": row["id"],
            "layer": "episodic",
            "summary": row["summary"],
            "outcome": row["outcome"],
            "files": json.loads(row["files"]),
            "verification": json.loads(row["verification"]),
            "sources": json.loads(row["sources"]),
            "created_at": row["created_at"],
        }
        for row in rows
    ]


def build_context_packet(
    connection: sqlite3.Connection, query: str, task_id: str, limit: int
) -> dict[str, object]:
    if limit < 1:
        raise ContextError("--limit must be a positive integer")
    return {
        "query": query,
        "task_id": task_id,
        "working": get_working_task(connection, task_id),
        "procedural": search_documents(connection, query, limit, "procedural"),
        "semantic": search_documents(connection, query, limit, "semantic"),
        "episodic": (
            search_documents(connection, query, limit, "episodic")
            + search_episodes(connection, query, limit)
        )[:limit],
    }


def validate_task_id(task_id: str) -> str:
    normalized = task_id.strip()
    if TASK_ID_PATTERN.fullmatch(normalized) is None:
        raise ContextError("Task ID must use letters, digits, '.', '_', '/', or '-'")
    reject_secrets("working task", [normalized])
    return normalized


def normalize_values(label: str, values: list[str]) -> list[str]:
    normalized = [value.strip() for value in values]
    if any(not value for value in normalized):
        raise ContextError(f"{label} must not be empty")
    return normalized


def validate_paths(label: str, values: list[str]) -> list[str]:
    """Validate exact path argv values without destroying Git provenance."""
    if any(not value for value in values):
        raise ContextError(f"{label} must not be empty")
    return list(values)


def merge_unique(existing: list[str], additions: list[str]) -> list[str]:
    return list(dict.fromkeys((*existing, *additions)))


def reject_secrets(record_type: str, values: list[str]) -> None:
    candidate = "\n".join(values)
    for label, pattern in SECRET_PATTERNS.items():
        if pattern.search(candidate):
            raise ContextError(f"possible {label} detected; {record_type} not stored")


def insert_episode(
    connection: sqlite3.Connection,
    summary: str,
    outcome: str,
    files: list[str],
    verification: list[str],
    sources: list[str],
) -> int:
    summary = summary.strip()
    outcome = outcome.strip()
    if not summary or not outcome:
        raise ContextError("Episode summary and outcome must not be empty")
    files = validate_paths("Episode file", files)
    verification = normalize_values("Episode verification", verification)
    sources = normalize_values("Episode source", sources)
    reject_secrets("episode", [summary, outcome, *files, *verification, *sources])
    cursor = connection.execute(
        """
        INSERT INTO episodes(summary, outcome, files, verification, sources, created_at)
        VALUES (?, ?, ?, ?, ?, ?)
        """,
        (
            summary,
            outcome,
            json.dumps(files, ensure_ascii=False),
            json.dumps(verification, ensure_ascii=False),
            json.dumps(sources, ensure_ascii=False),
            datetime.now(timezone.utc).isoformat(),
        ),
    )
    return int(cursor.lastrowid)


def record_episode(
    connection: sqlite3.Connection,
    summary: str,
    outcome: str,
    files: list[str],
    verification: list[str],
    sources: list[str],
) -> int:
    with connection:
        return insert_episode(
            connection, summary, outcome, files, verification, sources
        )


def get_working_task(
    connection: sqlite3.Connection, task_id: str
) -> dict[str, object]:
    task_id = validate_task_id(task_id)
    row = connection.execute(
        """
        SELECT task_id, goal, progress, next_steps, files, sources, created_at, updated_at
        FROM working_tasks
        WHERE task_id = ?
        """,
        (task_id,),
    ).fetchone()
    if row is None:
        raise ContextError(f"Working task not found: {task_id}")
    return {
        "task_id": row["task_id"],
        "goal": row["goal"],
        "progress": row["progress"],
        "next_steps": json.loads(row["next_steps"]),
        "files": json.loads(row["files"]),
        "sources": json.loads(row["sources"]),
        "created_at": row["created_at"],
        "updated_at": row["updated_at"],
    }


def start_working_task(
    connection: sqlite3.Connection,
    task_id: str,
    goal: str,
    files: list[str],
    sources: list[str],
) -> dict[str, object]:
    task_id = validate_task_id(task_id)
    goal = goal.strip()
    if not goal:
        raise ContextError("Working task goal must not be empty")
    files = validate_paths("Working task file", files)
    sources = normalize_values("Working task source", sources)
    reject_secrets("working task", [task_id, goal, *files, *sources])
    timestamp = datetime.now(timezone.utc).isoformat()
    try:
        connection.execute("BEGIN IMMEDIATE")
        if connection.execute(
            "SELECT 1 FROM working_tasks WHERE task_id = ?", (task_id,)
        ).fetchone() is not None:
            raise ContextError(f"Working task already exists: {task_id}")
        connection.execute(
            """
            INSERT INTO working_tasks(
                task_id, goal, progress, next_steps, files, sources, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                task_id,
                goal,
                "",
                json.dumps([], ensure_ascii=False),
                json.dumps(files, ensure_ascii=False),
                json.dumps(sources, ensure_ascii=False),
                timestamp,
                timestamp,
            ),
        )
        connection.commit()
    except Exception:
        connection.rollback()
        raise
    return get_working_task(connection, task_id)


def update_working_task(
    connection: sqlite3.Connection,
    task_id: str,
    progress: Optional[str],
    next_steps: list[str],
    files: list[str],
    sources: list[str],
) -> dict[str, object]:
    task_id = validate_task_id(task_id)
    next_steps = normalize_values("Working task next step", next_steps)
    files = validate_paths("Working task file", files)
    sources = normalize_values("Working task source", sources)
    if progress is None and not (next_steps or files or sources):
        raise ContextError("Working task update requires a changed field")
    if progress is not None:
        progress = progress.strip()
        if not progress:
            raise ContextError("Working task progress must not be empty")
    reject_secrets(
        "working task",
        [task_id] + ([progress] if progress is not None else []) + next_steps + files + sources,
    )
    try:
        connection.execute("BEGIN IMMEDIATE")
        row = connection.execute(
            "SELECT progress, next_steps, files, sources FROM working_tasks WHERE task_id = ?",
            (task_id,),
        ).fetchone()
        if row is None:
            raise ContextError(f"Working task not found: {task_id}")
        connection.execute(
            """
            UPDATE working_tasks
            SET progress = ?, next_steps = ?, files = ?, sources = ?, updated_at = ?
            WHERE task_id = ?
            """,
            (
                row["progress"] if progress is None else progress,
                json.dumps(
                    merge_unique(json.loads(row["next_steps"]), next_steps),
                    ensure_ascii=False,
                ),
                json.dumps(merge_unique(json.loads(row["files"]), files), ensure_ascii=False),
                json.dumps(
                    merge_unique(json.loads(row["sources"]), sources), ensure_ascii=False
                ),
                datetime.now(timezone.utc).isoformat(),
                task_id,
            ),
        )
        connection.commit()
    except Exception:
        connection.rollback()
        raise
    return get_working_task(connection, task_id)


def clear_working_task(connection: sqlite3.Connection, task_id: str) -> None:
    task_id = validate_task_id(task_id)
    with connection:
        cursor = connection.execute(
            "DELETE FROM working_tasks WHERE task_id = ?", (task_id,)
        )
        if cursor.rowcount != 1:
            raise ContextError(f"Working task not found: {task_id}")


def complete_working_task(
    connection: sqlite3.Connection,
    task_id: str,
    outcome: str,
    summary: Optional[str],
    files: list[str],
    verification: list[str],
    sources: list[str],
) -> int:
    files = validate_paths("Episode file", files)
    sources = normalize_values("Episode source", sources)
    try:
        connection.execute("BEGIN IMMEDIATE")
        task = get_working_task(connection, task_id)
        episode_id = insert_episode(
            connection,
            task["goal"] if summary is None else summary,
            outcome,
            merge_unique(task["files"], files),
            verification,
            merge_unique(task["sources"], sources),
        )
        cursor = connection.execute(
            "DELETE FROM working_tasks WHERE task_id = ?", (task["task_id"],)
        )
        if cursor.rowcount != 1:
            raise ContextError(f"Working task not found: {task_id}")
        connection.commit()
        return episode_id
    except Exception:
        connection.rollback()
        raise


def bind_governed_task(
    connection: sqlite3.Connection, external_id: str, task_uuid: str, revision: int
) -> None:
    try:
        with connection:
            timestamp = datetime.now(timezone.utc).isoformat()
            connection.execute(
                """
                INSERT INTO task_bindings(external_id, task_uuid, revision, updated_at)
                VALUES (?, ?, ?, ?)
                """,
                (external_id, task_uuid, revision, timestamp),
            )
            # Compatibility pointer only: no goal, progress, files, or sources
            # are duplicated from the authoritative Project Brain task.
            connection.execute(
                """
                INSERT INTO working_tasks(
                    task_id, goal, progress, next_steps, files, sources, created_at, updated_at
                ) VALUES (?, ?, '', '[]', '[]', '[]', ?, ?)
                """,
                (external_id, task_uuid, timestamp, timestamp),
            )
    except sqlite3.IntegrityError as error:
        raise ContextError(f"Working task already exists: {external_id}") from error


def governed_binding(connection: sqlite3.Connection, task_id: str) -> sqlite3.Row:
    task_id = validate_task_id(task_id)
    row = connection.execute(
        """
        SELECT external_id, task_uuid, revision, updated_at
        FROM task_bindings
        WHERE external_id = ? OR task_uuid = ?
        """,
        (task_id, task_id),
    ).fetchone()
    if row is None:
        raise ContextError(f"Working task not found: {task_id}")
    return row


def refresh_governed_binding(
    connection: sqlite3.Connection, external_id: str, revision: int
) -> None:
    with connection:
        cursor = connection.execute(
            """
            UPDATE task_bindings SET revision = ?, updated_at = ?
            WHERE external_id = ?
            """,
            (revision, datetime.now(timezone.utc).isoformat(), external_id),
        )
        if cursor.rowcount != 1:
            raise ContextError(f"Working task not found: {external_id}")


def remove_governed_binding(connection: sqlite3.Connection, external_id: str) -> None:
    with connection:
        cursor = connection.execute(
            "DELETE FROM task_bindings WHERE external_id = ?", (external_id,)
        )
        if cursor.rowcount != 1:
            raise ContextError(f"Working task not found: {external_id}")
        connection.execute("DELETE FROM working_tasks WHERE task_id = ?", (external_id,))


def compensate_governed_binding(
    connection: sqlite3.Connection,
    external_id: str,
    *,
    revision: Optional[int],
) -> None:
    """Restore the local side after a cross-store operation fails."""
    connection.rollback()
    connection.execute("BEGIN IMMEDIATE")
    try:
        if revision is None:
            connection.execute(
                "DELETE FROM task_bindings WHERE external_id = ?", (external_id,)
            )
            connection.execute(
                "DELETE FROM working_tasks WHERE task_id = ?", (external_id,)
            )
        else:
            cursor = connection.execute(
                """
                UPDATE task_bindings SET revision = ?, updated_at = ?
                WHERE external_id = ?
                """,
                (
                    revision,
                    datetime.now(timezone.utc).isoformat(),
                    external_id,
                ),
            )
            if cursor.rowcount != 1:
                raise ContextError(
                    f"Cannot restore governed binding: {external_id}"
                )
        connection.commit()
    except Exception:
        connection.rollback()
        raise


def governed_task_view(record: dict[str, object]) -> dict[str, object]:
    return {
        "task_id": record["external_id"],
        "task_uuid": record["id"],
        "revision": record["revision"],
        "status": record["status"],
        "goal": record["goal"],
        "progress": record["progress"],
        "next_steps": record["next_steps"],
        "files": record["files"],
        "sources": record["sources"],
        "created_at": record["created_at"],
        "updated_at": record["updated_at"],
        "authority": "project-brain",
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=default_root())
    parser.add_argument("--db", type=Path)
    parser.add_argument("--mode", choices=("governed", "lightweight"))
    parser.add_argument(
        "--owner",
        default=None,
        help="Project Brain actor/owner (default: PROJECT_BRAIN_OWNER or local)",
    )
    commands = parser.add_subparsers(dest="command", required=True)

    index = commands.add_parser("index", help="refresh the local document index")
    index.add_argument("--json", action="store_true")

    search = commands.add_parser("search", help="search indexed repository context")
    search.add_argument("query")
    search.add_argument("--limit", type=int, default=8)
    search.add_argument("--layer", choices=DOCUMENT_LAYERS)
    search.add_argument("--json", action="store_true")

    context = commands.add_parser("context", help="assemble layered task context")
    context.add_argument("query")
    context.add_argument("--task-id", required=True)
    context.add_argument("--limit", type=int, default=3)
    context.add_argument("--json", action="store_true")

    retrieve_command = commands.add_parser(
        "retrieve", help="assemble governed, budgeted task context"
    )
    retrieve_command.add_argument("query")
    retrieve_command.add_argument("--task-id", required=True)
    retrieve_command.add_argument("--limit", type=int, default=3)
    retrieve_command.add_argument("--json", action="store_true")

    record = commands.add_parser("record", help="store a local completed-task episode")
    record.add_argument("--summary", required=True)
    record.add_argument("--outcome", required=True)
    record.add_argument("--file", action="append", default=[])
    record.add_argument("--verification", action="append", default=[])
    record.add_argument("--source", action="append", default=[])
    record.add_argument("--json", action="store_true")

    start = commands.add_parser("start", help="store an active task")
    start.add_argument("--task-id", required=True)
    start.add_argument("--goal", required=True)
    start.add_argument("--file", action="append", default=[])
    start.add_argument("--source", action="append", default=[])
    start.add_argument("--json", action="store_true")

    update = commands.add_parser("update", help="update an active task")
    update.add_argument("--task-id", required=True)
    update.add_argument("--progress")
    update.add_argument("--next-step", action="append", default=[])
    update.add_argument("--file", action="append", default=[])
    update.add_argument("--source", action="append", default=[])
    update.add_argument("--revision", type=int)
    update.add_argument("--json", action="store_true")

    get = commands.add_parser("get", help="show an active task")
    get.add_argument("--task-id", required=True)
    get.add_argument("--json", action="store_true")

    clear = commands.add_parser("clear", help="clear an active task")
    clear.add_argument("--task-id", required=True)
    clear.add_argument("--json", action="store_true")

    complete = commands.add_parser("complete", help="complete an active task")
    complete.add_argument("--task-id", required=True)
    complete.add_argument("--outcome", required=True)
    complete.add_argument("--summary")
    complete.add_argument("--file", action="append", default=[])
    complete.add_argument("--verification", action="append", default=[])
    complete.add_argument("--source", action="append", default=[])
    complete.add_argument("--revision", type=int)
    complete.add_argument("--json", action="store_true")

    create_brain = commands.add_parser(
        "brain-create", help="create a governed dynamic Brain record"
    )
    create_brain.add_argument(
        "type", choices=("finding", "bug", "incident", "decision", "event")
    )
    create_brain.add_argument("--external-id", required=True)
    create_brain.add_argument("--title", required=True)
    create_brain.add_argument("--goal")
    create_brain.add_argument("--file", action="append", default=[])
    create_brain.add_argument("--source", action="append", default=[])
    create_brain.add_argument("--conflict", action="append", default=[])
    create_brain.add_argument(
        "--privacy", choices=("public", "team", "restricted", "private"), default="team"
    )
    create_brain.add_argument(
        "--authority", choices=("inferred", "observed", "verified"), default="observed"
    )
    create_brain.add_argument("--confidence", type=float, default=1.0)
    create_brain.add_argument("--json", action="store_true")

    update_brain = commands.add_parser(
        "brain-update", help="CAS-update a governed dynamic Brain record"
    )
    update_brain.add_argument("--record-id", required=True)
    update_brain.add_argument("--revision", type=int, required=True)
    update_brain.add_argument("--progress")
    update_brain.add_argument("--next-step", action="append", default=[])
    update_brain.add_argument("--file", action="append", default=[])
    update_brain.add_argument("--source", action="append", default=[])
    update_brain.add_argument("--conflict", action="append", default=[])
    update_brain.add_argument("--transition")
    update_brain.add_argument("--reason", default="Record updated")
    update_brain.add_argument("--json", action="store_true")

    get_brain = commands.add_parser(
        "brain-get", help="show a governed dynamic Brain record"
    )
    get_brain.add_argument("--record-id", required=True)
    get_brain.add_argument("--json", action="store_true")

    status = commands.add_parser("status", help="show local context index counts")
    status.add_argument("--json", action="store_true")

    validate = commands.add_parser("validate", help="validate active and archived Brain records")
    validate.add_argument("--json", action="store_true")

    parity = commands.add_parser("parity", help="fail on canonical skill mirror drift")
    parity.add_argument("--json", action="store_true")

    compact_command = commands.add_parser(
        "compact", help="archive terminal and superseded Brain records"
    )
    compact_command.add_argument("--json", action="store_true")

    propose = commands.add_parser("promote-propose", help="propose durable-memory promotion")
    propose.add_argument("--source-id", action="append", required=True)
    propose.add_argument("--title", required=True)
    propose.add_argument("--content", required=True)
    propose.add_argument("--json", action="store_true")

    review = commands.add_parser("promote-review", help="human-review a promotion")
    review.add_argument("--promotion-id", required=True)
    review.add_argument("--reviewer", required=True)
    review.add_argument("--reject", action="store_true")
    review.add_argument("--json", action="store_true")

    apply = commands.add_parser("promote-apply", help="apply an approved promotion")
    apply.add_argument("--promotion-id", required=True)
    apply.add_argument("--json", action="store_true")
    return parser


def main() -> int:
    arguments = build_parser().parse_args()
    repository = arguments.root.resolve()

    try:
        if not repository.is_dir():
            raise ContextError(
                f"Repository root must be an existing directory: {repository}"
            )
        database = (arguments.db or default_database(repository)).resolve()
        mode = configured_mode(repository, arguments.mode)
        owner = arguments.owner or os.environ.get("PROJECT_BRAIN_OWNER", "local")
        connection = connect(database)
        try:
            if arguments.command == "index":
                result = index_repository(connection, repository)
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    print(
                        f"Context index: {result['documents']} documents "
                        f"({result['removed']} removed)."
                    )
                return 0

            if arguments.command == "record":
                episode_id = record_episode(
                    connection,
                    arguments.summary,
                    arguments.outcome,
                    arguments.file,
                    arguments.verification,
                    arguments.source,
                )
                result = {"episode_id": episode_id}
                if arguments.json:
                    print(json.dumps(result))
                else:
                    print(f"Context episode recorded: {episode_id}.")
                return 0

            if arguments.command == "start":
                if mode == "lightweight":
                    result = start_working_task(
                        connection,
                        arguments.task_id,
                        arguments.goal,
                        arguments.file,
                        arguments.source,
                    )
                else:
                    task_id = validate_task_id(arguments.task_id)
                    goal = arguments.goal.strip()
                    if not goal:
                        raise ContextError("Working task goal must not be empty")
                    files = validate_paths("Working task file", arguments.file)
                    sources = normalize_values("Working task source", arguments.source)
                    reject_secrets("working task", [task_id, goal, *files, *sources])
                    with mutation_lock(repository):
                        task = create_task(
                            repository, task_id, goal, files, sources, owner=owner
                        )
                        try:
                            bind_governed_task(
                                connection,
                                task_id,
                                str(task["id"]),
                                int(task["revision"]),
                            )
                        except Exception:
                            try:
                                compensate_governed_binding(
                                    connection, task_id, revision=None
                                )
                            finally:
                                rollback_created_record(
                                    repository, str(task["id"])
                                )
                            raise
                    result = governed_task_view(task)
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    print(f"Working task started: {result['task_id']}.")
                return 0

            if arguments.command == "update":
                if mode == "lightweight":
                    result = update_working_task(
                        connection,
                        arguments.task_id,
                        arguments.progress,
                        arguments.next_step,
                        arguments.file,
                        arguments.source,
                    )
                else:
                    binding = governed_binding(connection, arguments.task_id)
                    progress = arguments.progress
                    if progress is None and not (
                        arguments.next_step or arguments.file or arguments.source
                    ):
                        raise ContextError("Working task update requires a changed field")
                    if progress is not None and not progress.strip():
                        raise ContextError("Working task progress must not be empty")
                    next_steps = normalize_values(
                        "Working task next step", arguments.next_step
                    )
                    files = validate_paths("Working task file", arguments.file)
                    sources = normalize_values("Working task source", arguments.source)
                    reject_secrets(
                        "working task",
                        ([progress] if progress else []) + next_steps + files + sources,
                    )
                    with mutation_lock(repository):
                        snapshot = snapshot_record_state(
                            repository, binding["task_uuid"]
                        )
                        try:
                            task = update_task(
                                repository,
                                binding["task_uuid"],
                                expected_revision=arguments.revision,
                                progress=progress.strip() if progress else None,
                                next_steps=next_steps,
                                files=files,
                                sources=sources,
                                actor=owner,
                            )
                            refresh_governed_binding(
                                connection,
                                binding["external_id"],
                                int(task["revision"]),
                            )
                        except Exception:
                            restore_record_state(repository, snapshot)
                            compensate_governed_binding(
                                connection,
                                binding["external_id"],
                                revision=int(binding["revision"]),
                            )
                            raise
                    result = governed_task_view(task)
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    print(f"Working task updated: {result['task_id']}.")
                return 0

            if arguments.command == "get":
                if mode == "lightweight":
                    result = get_working_task(connection, arguments.task_id)
                else:
                    binding = governed_binding(connection, arguments.task_id)
                    result = governed_task_view(
                        get_task(repository, binding["task_uuid"])
                    )
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    print(f"Working task: {result['task_id']} — {result['goal']}")
                return 0

            if arguments.command == "clear":
                task_id = validate_task_id(arguments.task_id)
                if mode == "lightweight":
                    clear_working_task(connection, task_id)
                else:
                    binding = governed_binding(connection, task_id)
                    with mutation_lock(repository):
                        snapshot = snapshot_record_state(
                            repository, binding["task_uuid"]
                        )
                        try:
                            cancel_task(
                                repository, binding["task_uuid"], actor=owner
                            )
                            remove_governed_binding(
                                connection, binding["external_id"]
                            )
                        except Exception:
                            restore_record_state(repository, snapshot)
                            compensate_governed_binding(
                                connection,
                                binding["external_id"],
                                revision=int(binding["revision"]),
                            )
                            raise
                result = {"task_id": task_id}
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    print(f"Working task cleared: {task_id}.")
                return 0

            if arguments.command == "complete":
                if mode == "lightweight":
                    episode_id = complete_working_task(
                        connection,
                        arguments.task_id,
                        arguments.outcome,
                        arguments.summary,
                        arguments.file,
                        arguments.verification,
                        arguments.source,
                    )
                    result = {"episode_id": episode_id}
                else:
                    binding = governed_binding(connection, arguments.task_id)
                    task = get_task(repository, binding["task_uuid"])
                    files = validate_paths("Episode file", arguments.file)
                    verification = normalize_values(
                        "Episode verification", arguments.verification
                    )
                    sources = normalize_values("Episode source", arguments.source)
                    with mutation_lock(repository):
                        snapshot = snapshot_record_state(
                            repository, binding["task_uuid"]
                        )
                        try:
                            connection.execute("BEGIN IMMEDIATE")
                            episode_id = insert_episode(
                                connection,
                                (
                                    str(task["goal"])
                                    if arguments.summary is None
                                    else arguments.summary
                                ),
                                arguments.outcome,
                                merge_unique(list(task["files"]), files),
                                verification,
                                merge_unique(list(task["sources"]), sources),
                            )
                            deleted = connection.execute(
                                "DELETE FROM working_tasks WHERE task_id = ?",
                                (binding["external_id"],),
                            )
                            if deleted.rowcount != 1:
                                raise ContextError(
                                    f"Working task not found: {binding['external_id']}"
                                )
                            connection.execute(
                                "DELETE FROM task_bindings WHERE external_id = ?",
                                (binding["external_id"],),
                            )
                            completed_task = close_task(
                                repository,
                                binding["task_uuid"],
                                arguments.outcome,
                                verification,
                                expected_revision=arguments.revision,
                                actor=owner,
                            )
                            connection.commit()
                        except Exception:
                            connection.rollback()
                            restore_record_state(repository, snapshot)
                            raise
                    result = {
                        "episode_id": episode_id,
                        "task_uuid": completed_task["id"],
                        "revision": completed_task["revision"],
                        "status": completed_task["status"],
                    }
                if arguments.json:
                    print(json.dumps(result))
                else:
                    print(f"Working task completed as episode: {episode_id}.")
                return 0

            if arguments.command == "brain-create":
                external_id = validate_task_id(arguments.external_id)
                title = arguments.title.strip()
                if not title:
                    raise ContextError("Record title must not be empty")
                files = validate_paths("Record file", arguments.file)
                sources = normalize_values("Record source", arguments.source)
                conflicts = normalize_values("Record conflict", arguments.conflict)
                reject_secrets(
                    "Brain record",
                    [external_id, title, *files, *sources, *conflicts],
                )
                result = create_record(
                    repository,
                    arguments.type,
                    external_id,
                    title,
                    files,
                    sources,
                    owner=owner,
                    privacy=arguments.privacy,
                    authority=arguments.authority,
                    confidence=arguments.confidence,
                    goal=arguments.goal,
                    conflicts=conflicts,
                )
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    print(
                        f"Project Brain {result['type']} created: {result['id']}."
                    )
                return 0

            if arguments.command == "brain-update":
                progress = (
                    arguments.progress.strip()
                    if arguments.progress is not None
                    else None
                )
                if progress == "":
                    raise ContextError("Record progress must not be empty")
                next_steps = normalize_values("Record next step", arguments.next_step)
                files = validate_paths("Record file", arguments.file)
                sources = normalize_values("Record source", arguments.source)
                conflicts = normalize_values("Record conflict", arguments.conflict)
                if (
                    progress is None
                    and not next_steps
                    and not files
                    and not sources
                    and not conflicts
                    and arguments.transition is None
                ):
                    raise ContextError("Brain record update requires a changed field")
                reject_secrets(
                    "Brain record",
                    ([progress] if progress else [])
                    + next_steps
                    + files
                    + sources
                    + conflicts,
                )
                result = update_record(
                    repository,
                    arguments.record_id,
                    expected_revision=arguments.revision,
                    progress=progress,
                    next_steps=next_steps,
                    files=files,
                    sources=sources,
                    actor=owner,
                    conflicts=conflicts,
                    transition_to=arguments.transition,
                    reason=arguments.reason,
                )
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    print(
                        f"Project Brain {result['type']} updated: {result['id']}."
                    )
                return 0

            if arguments.command == "brain-get":
                result = get_record(repository, arguments.record_id)
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    print(
                        f"Project Brain {result['type']}: "
                        f"{result['external_id']} — {result['title']}"
                    )
                return 0

            if arguments.command == "status":
                layers = {layer: 0 for layer in DOCUMENT_LAYERS}
                for row in connection.execute(
                    "SELECT layer, COUNT(*) AS count FROM documents GROUP BY layer"
                ):
                    layers[row["layer"]] = row["count"]
                result = {
                    "documents": connection.execute(
                        "SELECT COUNT(*) FROM documents"
                    ).fetchone()[0],
                    "episodes": connection.execute(
                        "SELECT COUNT(*) FROM episodes"
                    ).fetchone()[0],
                    "working": connection.execute(
                        "SELECT COUNT(*) FROM "
                        + ("working_tasks" if mode == "lightweight" else "task_bindings")
                    ).fetchone()[0],
                    "mode": mode,
                    "authority": (
                        "local-sqlite" if mode == "lightweight" else "project-brain"
                    ),
                    "layers": layers,
                    "database": str(database),
                }
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    print(
                        f"Context index: {result['documents']} documents, "
                        f"{result['episodes']} episodes, {result['working']} working tasks "
                        f"({', '.join(f'{layer}: {count}' for layer, count in layers.items())}; "
                        f"{result['database']})."
                    )
                return 0

            if arguments.command == "search":
                if arguments.limit < 1:
                    raise ContextError("--limit must be a positive integer")
                documents = search_documents(
                    connection, arguments.query, arguments.limit, arguments.layer
                )
                episodes = (
                    search_episodes(connection, arguments.query, arguments.limit)
                    if arguments.layer in (None, "episodic")
                    else []
                )
                result = {
                    "query": arguments.query,
                    "documents": documents,
                    "episodes": episodes,
                }
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    for item in documents:
                        print(
                            f"{item['layer']} {item['kind']}: "
                            f"{item['path']} — {item['title']}"
                        )
                        print(f"  {item['snippet']}")
                    for item in episodes:
                        print(f"episode {item['id']}: {item['summary']}")
                        print(f"  {item['outcome']}")
                return 0

            if arguments.command in {"context", "retrieve"}:
                if mode == "lightweight":
                    result = build_context_packet(
                        connection, arguments.query, arguments.task_id, arguments.limit
                    )
                else:
                    binding = governed_binding(connection, arguments.task_id)
                    result = retrieve(
                        connection,
                        repository,
                        arguments.query,
                        binding["task_uuid"],
                        limit=arguments.limit,
                    )
                    result["episodic"] = (
                        search_documents(
                            connection, arguments.query, arguments.limit, "episodic"
                        )
                        + search_episodes(connection, arguments.query, arguments.limit)
                    )[: arguments.limit]
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    working = result["working"]
                    print(f"working: {working['task_id']} — {working['goal']}")
                    for layer in ("procedural", "semantic", "episodic"):
                        items = result[layer]
                        if not items:
                            continue
                        print(f"{layer}:")
                        for item in items:
                            label = item["path"] if "path" in item else f"episode {item['id']}"
                            title = item["title"] if "title" in item else item["summary"]
                            print(f"  {label} — {title}")
                return 0

            if arguments.command == "validate":
                errors = validate_repository(repository)
                result = {"valid": not errors, "errors": errors}
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                elif errors:
                    for error in errors:
                        print(error)
                else:
                    print("Project Brain validation passed.")
                return 0 if not errors else 1

            if arguments.command == "parity":
                canonical = str(load_config(repository)["canonical_edition"])
                assert_skill_mirror_parity(repository, canonical)
                result = {"valid": True, "canonical_edition": canonical}
                if arguments.json:
                    print(json.dumps(result))
                else:
                    print(f"Skill mirror parity passed ({canonical} canonical).")
                return 0

            if arguments.command == "compact":
                result = compact(repository)
                if arguments.json:
                    print(json.dumps(result))
                else:
                    print(f"Project Brain compacted: {result['moved']} record(s) archived.")
                return 0

            if arguments.command == "promote-propose":
                result = create_promotion(
                    repository,
                    arguments.source_id,
                    arguments.title,
                    arguments.content,
                    proposer=owner,
                )
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    print(f"Promotion proposed: {result['id']}.")
                return 0

            if arguments.command == "promote-review":
                result = review_promotion(
                    repository,
                    arguments.promotion_id,
                    arguments.reviewer,
                    not arguments.reject,
                )
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    print(f"Promotion review recorded: {result['status']}.")
                return 0

            if arguments.command == "promote-apply":
                result = apply_promotion(repository, arguments.promotion_id)
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    print(
                        f"Promotion applied: {result['destination_memory_id']}."
                    )
                return 0

            raise ContextError(f"Unsupported command: {arguments.command}")
        finally:
            connection.close()
    except (ContextError, BrainError, RetrievalError, OSError, sqlite3.Error) as error:
        print(f"context: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
