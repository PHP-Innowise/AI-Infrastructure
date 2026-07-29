#!/usr/bin/env python3
"""Repository-local context index and episodic memory."""

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
CAPSULE_LAYER_LIMITS = {
    "procedural": 2,
    "semantic": 3,
    "episodic": 1,
}
CAPSULE_QUERY_TOKEN_LIMIT = 32
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
    indexed_paths = {path for path, _, _, _, _ in documents}

    with connection:
        existing_paths = {
            row["path"]
            for row in connection.execute("SELECT path FROM documents").fetchall()
        }
        stale_paths = existing_paths - indexed_paths
        connection.execute("DELETE FROM documents")
        connection.executemany(
            "INSERT INTO documents(path, layer, kind, title, content) VALUES (?, ?, ?, ?, ?)",
            documents,
        )

    return {
        "documents": len(documents),
        "removed": len(stale_paths),
        "layers": {
            layer: sum(document[1] == layer for document in documents)
            for layer in DOCUMENT_LAYERS
        },
    }


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


def build_capsule_query(
    query: str, working: Optional[dict[str, object]]
) -> str:
    values = [query]
    if working is not None:
        values.extend(
            [
                str(working["goal"]),
                str(working["progress"]),
                *[str(item) for item in working["next_steps"]],
                *[Path(str(item)).stem for item in working["files"]],
            ]
        )
    tokens: list[str] = []
    seen_tokens: set[str] = set()
    for token in re.findall(r"\w+", " ".join(values), flags=re.UNICODE):
        normalized = token.casefold()
        if normalized in seen_tokens:
            continue
        seen_tokens.add(normalized)
        tokens.append(token)
        if len(tokens) == CAPSULE_QUERY_TOKEN_LIMIT:
            break
    if not tokens:
        raise ContextError("Search query must contain a word")
    return " ".join(tokens)


def deduplicate_context_items(
    items: list[dict[str, object]],
) -> list[dict[str, object]]:
    unique: list[dict[str, object]] = []
    seen: set[tuple[str, object]] = set()
    for item in items:
        key = ("path", item["path"]) if "path" in item else ("episode", item["id"])
        if key in seen:
            continue
        seen.add(key)
        unique.append(item)
    return unique


def build_context_packet(
    connection: sqlite3.Connection,
    query: str,
    task_id: Optional[str],
    limit: int,
    include_retrieval: bool = True,
    warnings: Optional[list[str]] = None,
) -> dict[str, object]:
    if limit < 1:
        raise ContextError("--limit must be a positive integer")
    reject_secrets("Task Capsule", [query])

    capsule_warnings = list(warnings or [])
    working = find_working_task(connection, task_id)
    if task_id is None:
        capsule_warnings.append("Working task unavailable: task ID was not supplied")
    elif working is None:
        capsule_warnings.append(f"Working task not found: {validate_task_id(task_id)}")

    packet: dict[str, object] = {
        "query": query,
        "task_id": task_id,
        "working": working,
        "procedural": [],
        "semantic": [],
        "episodic": [],
        "warnings": capsule_warnings,
        "omitted": {"working_files": 0},
    }
    if not include_retrieval:
        return packet

    retrieval_query = build_capsule_query(query, working)
    procedural_limit = min(limit, CAPSULE_LAYER_LIMITS["procedural"])
    semantic_limit = min(limit, CAPSULE_LAYER_LIMITS["semantic"])
    episodic_limit = min(limit, CAPSULE_LAYER_LIMITS["episodic"])
    packet["procedural"] = deduplicate_context_items(
        search_documents(connection, retrieval_query, procedural_limit, "procedural")
    )[:procedural_limit]
    packet["semantic"] = deduplicate_context_items(
        search_documents(connection, retrieval_query, semantic_limit, "semantic")
    )[:semantic_limit]
    packet["episodic"] = deduplicate_context_items(
        search_documents(connection, retrieval_query, episodic_limit, "episodic")
        + search_episodes(connection, retrieval_query, episodic_limit)
    )[:episodic_limit]
    return packet


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


def find_working_task(
    connection: sqlite3.Connection, task_id: Optional[str]
) -> Optional[dict[str, object]]:
    if task_id is None:
        return None
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
        return None
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


def get_working_task(
    connection: sqlite3.Connection, task_id: str
) -> dict[str, object]:
    task = find_working_task(connection, task_id)
    if task is None:
        raise ContextError(f"Working task not found: {validate_task_id(task_id)}")
    return task


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


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=default_root())
    parser.add_argument("--db", type=Path)
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
    context.add_argument("--task-id")
    context.add_argument("--limit", type=int, default=3)
    context.add_argument("--json", action="store_true")

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
    complete.add_argument("--json", action="store_true")

    status = commands.add_parser("status", help="show local context index counts")
    status.add_argument("--json", action="store_true")
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
                result = start_working_task(
                    connection,
                    arguments.task_id,
                    arguments.goal,
                    arguments.file,
                    arguments.source,
                )
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    print(f"Working task started: {result['task_id']}.")
                return 0

            if arguments.command == "update":
                result = update_working_task(
                    connection,
                    arguments.task_id,
                    arguments.progress,
                    arguments.next_step,
                    arguments.file,
                    arguments.source,
                )
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    print(f"Working task updated: {result['task_id']}.")
                return 0

            if arguments.command == "get":
                result = get_working_task(connection, arguments.task_id)
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    print(f"Working task: {result['task_id']} — {result['goal']}")
                return 0

            if arguments.command == "clear":
                task_id = validate_task_id(arguments.task_id)
                clear_working_task(connection, task_id)
                result = {"task_id": task_id}
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    print(f"Working task cleared: {task_id}.")
                return 0

            if arguments.command == "complete":
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
                if arguments.json:
                    print(json.dumps(result))
                else:
                    print(f"Working task completed as episode: {episode_id}.")
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
                        "SELECT COUNT(*) FROM working_tasks"
                    ).fetchone()[0],
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

            if arguments.command == "context":
                result = build_context_packet(
                    connection, arguments.query, arguments.task_id, arguments.limit
                )
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    working = result["working"]
                    if working is not None:
                        print(f"working: {working['task_id']} — {working['goal']}")
                    for warning in result["warnings"]:
                        print(f"warning: {warning}")
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

            raise ContextError(f"Unsupported command: {arguments.command}")
        finally:
            connection.close()
    except (ContextError, OSError, sqlite3.Error) as error:
        print(f"context: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
