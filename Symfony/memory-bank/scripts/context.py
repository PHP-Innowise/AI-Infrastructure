#!/usr/bin/env python3
"""Repository-local context index and episodic memory."""

from __future__ import annotations

import argparse
import json
import re
import sqlite3
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


def discover_documents(repository: Path) -> list[tuple[str, str, str, str, str]]:
    documents: list[tuple[str, str, str, str, str]] = []
    skill_keys: set[str] = set()
    for layer, kind, pattern in SOURCE_PATTERNS:
        for path in sorted(repository.glob(pattern)):
            if not path.is_file() or path.is_symlink():
                continue
            if kind == "memory":
                if not active_memory(path, repository):
                    continue
            else:
                try:
                    validate_secret_patterns(path)
                except (OSError, ValidationError):
                    continue
            relative_path = path.relative_to(repository).as_posix()
            if kind == "skill":
                skill_key = relative_path.split("/skills/", maxsplit=1)[1]
                if skill_key in skill_keys:
                    continue
                skill_keys.add(skill_key)
            content = path.read_text(encoding="utf-8")
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
            "summary": row["summary"],
            "outcome": row["outcome"],
            "files": json.loads(row["files"]),
            "verification": json.loads(row["verification"]),
            "sources": json.loads(row["sources"]),
            "created_at": row["created_at"],
        }
        for row in rows
    ]


def record_episode(
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
    cleaned: list[list[str]] = []
    for label, values in (
        ("file", files),
        ("verification", verification),
        ("source", sources),
    ):
        normalized = [value.strip() for value in values]
        if any(not value for value in normalized):
            raise ContextError(f"Episode {label} must not be empty")
        cleaned.append(normalized)
    files, verification, sources = cleaned
    candidate = "\n".join((summary, outcome, *files, *verification, *sources))
    for label, pattern in SECRET_PATTERNS.items():
        if pattern.search(candidate):
            raise ContextError(f"possible {label} detected; episode not stored")
    with connection:
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

    record = commands.add_parser("record", help="store a local completed-task episode")
    record.add_argument("--summary", required=True)
    record.add_argument("--outcome", required=True)
    record.add_argument("--file", action="append", default=[])
    record.add_argument("--verification", action="append", default=[])
    record.add_argument("--source", action="append", default=[])
    record.add_argument("--json", action="store_true")

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
                    "layers": layers,
                    "database": str(database),
                }
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    print(
                        f"Context index: {result['documents']} documents, "
                        f"{result['episodes']} episodes "
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
                episodes = search_episodes(connection, arguments.query, arguments.limit)
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

            raise ContextError(f"Unsupported command: {arguments.command}")
        finally:
            connection.close()
    except (ContextError, OSError, sqlite3.Error) as error:
        print(f"context: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
