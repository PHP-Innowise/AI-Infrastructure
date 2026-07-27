#!/usr/bin/env python3
"""Repository-local context index and episodic memory."""

from __future__ import annotations

import argparse
import json
import re
import sqlite3
import sys
from datetime import UTC, datetime
from pathlib import Path

from validate import SECRET_PATTERNS


class ContextError(Exception):
    """A safe, user-facing context-engine error."""


SOURCE_PATTERNS = (
    ("policy", "AGENTS.md"),
    ("spec", "specs/**/*.md"),
    ("memory", "memory-bank/chunks/*.md"),
    ("task", "tasks/**/*.md"),
    ("capability", "Task/Epics/**/*.md"),
    ("changelog", "CHANGELOG.md"),
)


def default_root() -> Path:
    return Path(__file__).resolve().parents[2]


def default_database(repository: Path) -> Path:
    return repository / "memory-bank" / "local" / "context.db"


def connect(database: Path) -> sqlite3.Connection:
    database.parent.mkdir(parents=True, exist_ok=True)
    connection = sqlite3.connect(database)
    connection.row_factory = sqlite3.Row
    try:
        connection.executescript(
            """
            CREATE VIRTUAL TABLE IF NOT EXISTS documents USING fts5(
                path UNINDEXED,
                kind UNINDEXED,
                title,
                content,
                tokenize = 'unicode61'
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS episodes USING fts5(
                summary,
                outcome,
                files UNINDEXED,
                verification UNINDEXED,
                sources UNINDEXED,
                created_at UNINDEXED,
                tokenize = 'unicode61'
            );
            """
        )
    except sqlite3.OperationalError as error:
        connection.close()
        if "fts5" in str(error).lower():
            raise ContextError("SQLite FTS5 support is required") from error
        raise
    return connection


def document_title(path: Path, content: str) -> str:
    for line in content.splitlines():
        if line.startswith("# "):
            return line[2:].strip()
    return path.stem.replace("-", " ").replace("_", " ")


def active_memory(content: str) -> bool:
    if not content.startswith("---\n"):
        return False
    try:
        raw_metadata, _ = content[4:].split("\n---\n", 1)
        metadata = json.loads(raw_metadata)
    except (ValueError, json.JSONDecodeError):
        return False
    return isinstance(metadata, dict) and metadata.get("status") == "active"


def discover_documents(repository: Path) -> list[tuple[str, str, str, str]]:
    documents: list[tuple[str, str, str, str]] = []
    for kind, pattern in SOURCE_PATTERNS:
        for path in sorted(repository.glob(pattern)):
            if not path.is_file() or path.is_symlink():
                continue
            content = path.read_text(encoding="utf-8")
            if kind == "memory" and not active_memory(content):
                continue
            documents.append(
                (
                    path.relative_to(repository).as_posix(),
                    kind,
                    document_title(path, content),
                    content,
                )
            )
    return documents


def index_repository(connection: sqlite3.Connection, repository: Path) -> dict[str, int]:
    documents = discover_documents(repository)
    indexed_paths = {path for path, _, _, _ in documents}

    with connection:
        existing_paths = {
            row["path"]
            for row in connection.execute("SELECT path FROM documents").fetchall()
        }
        stale_paths = existing_paths - indexed_paths
        connection.execute("DELETE FROM documents")
        connection.executemany(
            "INSERT INTO documents(path, kind, title, content) VALUES (?, ?, ?, ?)",
            documents,
        )

    return {"documents": len(documents), "removed": len(stale_paths)}


def fts_query(query: str) -> str:
    # ponytail: FTS5-only retrieval; add local embeddings only after a
    # golden-query evaluation shows lexical recall is insufficient.
    tokens = re.findall(r"\w+", query, flags=re.UNICODE)
    if not tokens:
        raise ContextError("Search query must contain a word")
    return " OR ".join(f'"{token.replace(chr(34), chr(34) * 2)}"' for token in tokens)


def search_documents(
    connection: sqlite3.Connection,
    query: str,
    limit: int,
) -> list[dict[str, object]]:
    rows = connection.execute(
        """
        SELECT
            path,
            kind,
            title,
            snippet(documents, 3, '[', ']', ' … ', 18) AS snippet
        FROM documents
        WHERE documents MATCH ?
        ORDER BY bm25(documents), path
        LIMIT ?
        """,
        (fts_query(query), limit),
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
    if not summary.strip() or not outcome.strip():
        raise ContextError("Episode summary and outcome must not be empty")
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
                summary.strip(),
                outcome.strip(),
                json.dumps(files, ensure_ascii=False),
                json.dumps(verification, ensure_ascii=False),
                json.dumps(sources, ensure_ascii=False),
                datetime.now(UTC).isoformat(),
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
    database = (arguments.db or default_database(repository)).resolve()

    try:
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
                result = {
                    "documents": connection.execute(
                        "SELECT COUNT(*) FROM documents"
                    ).fetchone()[0],
                    "episodes": connection.execute(
                        "SELECT COUNT(*) FROM episodes"
                    ).fetchone()[0],
                    "database": str(database),
                }
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    print(
                        f"Context index: {result['documents']} documents, "
                        f"{result['episodes']} episodes ({result['database']})."
                    )
                return 0

            if arguments.command == "search":
                if arguments.limit < 1:
                    raise ContextError("--limit must be a positive integer")
                documents = search_documents(connection, arguments.query, arguments.limit)
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
                        print(f"{item['kind']}: {item['path']} — {item['title']}")
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
