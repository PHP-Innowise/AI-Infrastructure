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
    LIFECYCLES,
    TASK_PHASES,
    auto_compact,
    auto_promote,
    cancel_task,
    iter_records,
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
    DocumentRow,
    RetrievalError,
    SourceState,
    informative_tokens,
    assert_skill_mirror_parity,
    ensure_metadata_tables,
    index_documents,
    codebase_map_drift,
    is_relevant,
    query_tokens,
    required_coverage,
    retrieve,
    reusable_source_state,
    token_coverage,
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
    ("semantic", "codebase", "codebase/*.md"),
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
CAPSULE_CHARACTER_LIMIT = 8000
CAPSULE_WORKING_FILE_LIMIT = 8
CAPSULE_WORKING_SOURCE_LIMIT = 4
CAPSULE_PRIVATE_PATTERNS = (
    re.compile(r"\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b", re.IGNORECASE),
    re.compile(
        r"(?<!\w)(?:\+\d(?:[\d ().-]{6,}\d)|\(?\d{3}\)?[ .-]\d{3}[ .-]\d{4})(?!\w)"
    ),
    re.compile(
        r"\b(?:(?:customer|patient)\s+(?:name|address|id)|"
        r"client\s+(?:name|address))\s*[:=]\s*\S+",
        re.IGNORECASE,
    ),
)
CAPSULE_RAW_TEXT_PATTERN = re.compile(
    r"^\s*(?:user|assistant|system|developer|tool|prompt|response|reasoning|"
    r"stdout|stderr|log)\s*:",
    re.IGNORECASE | re.MULTILINE,
)
TASK_ID_PATTERN = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._/-]{0,127}$")
# Porter wraps unicode61 so "rounding" matches "round" and "review" matches
# "reviewer". Without stemming the correct skill is simply missed: a security
# question did not retrieve the security skill because its title says
# "Reviewer". Non-English tokens pass through the stemmer unchanged.
TOKENIZER = "porter unicode61"
# A skill's identity lives in its declared description, not in its body prose,
# which reads much alike across skills. Indexing that separately lets ranking
# weight what a document is *about* over what it happens to mention.
DESCRIPTION_PATTERN = re.compile(r"^description:\s*(.+)$", re.MULTILINE)
SUMMARY_CHARACTERS = 400
AUTO_REVISION = "auto"
DEFAULT_TURN_FLUSH_AFTER = 5
DEFAULT_TURN_FILE_LIMIT = 20
TURN_PATH_DENYLIST = re.compile(
    r"(^|/)(\.env(\..+)?|secrets?|id_[a-z0-9]+|[^/]+\.(pem|key|p12|pfx|jks|keystore))$",
    re.IGNORECASE,
)
TURN_RUNTIME_DIRECTORIES = ("dynamic", "control", "indexes", "archive", "local")
BRANCH_PREFIXES = (
    "feature/", "feat/", "fix/", "bugfix/", "hotfix/", "chore/", "release/",
    "refactor/", "docs/", "test/", "merge/",
)
TICKET_PATTERN = re.compile(r"[A-Za-z][A-Za-z0-9]*-\d+")


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
        document_schema = connection.execute(
            "SELECT sql FROM sqlite_master WHERE name = 'documents'"
        ).fetchone()
        stale_tokenizer = document_schema is not None and TOKENIZER not in (
            document_schema["sql"] or ""
        )
        if document_columns and (
            "layer" not in document_columns
            or "summary" not in document_columns
            or stale_tokenizer
        ):
            # The index is derived from repository sources, so a tokenizer
            # change is a rebuild rather than a migration.
            connection.execute("DROP TABLE documents")
            connection.execute("DROP TABLE IF EXISTS document_source_state")
            document_columns = set()
        if not document_columns:
            create_document_table(connection)
        episode_schema = connection.execute(
            "SELECT sql FROM sqlite_master WHERE name = 'episodes'"
        ).fetchone()
        if episode_schema is None:
            create_episode_table(connection)
        elif TOKENIZER not in (episode_schema["sql"] or "") or any(
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
        connection.execute(
            """
            CREATE TABLE IF NOT EXISTS turn_deltas(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                files TEXT NOT NULL
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
        f"""
        CREATE VIRTUAL TABLE documents USING fts5(
            path UNINDEXED,
            layer UNINDEXED,
            kind UNINDEXED,
            title,
            summary,
            content,
            tokenize = '{TOKENIZER}'
        )
        """
    )


def create_episode_table(connection: sqlite3.Connection) -> None:
    connection.execute(
        f"""
        CREATE VIRTUAL TABLE episodes USING fts5(
            summary,
            outcome,
            files,
            verification,
            sources,
            created_at UNINDEXED,
            tokenize = '{TOKENIZER}'
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


def document_summary(content: str) -> str:
    """Return what a document declares itself to be about.

    Prefers a frontmatter `description`, and otherwise the opening prose, so a
    document without one still contributes something more specific than its
    whole body.
    """
    described = DESCRIPTION_PATTERN.search(content)
    if described:
        return " ".join(described.group(1).split())[:SUMMARY_CHARACTERS]
    body = re.sub(r"^---.*?^---", "", content, flags=re.S | re.M)
    lead = [line for line in body.splitlines() if line.strip() and not line.startswith("#")]
    return " ".join(" ".join(lead).split())[:SUMMARY_CHARACTERS]


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
    except OSError as error:
        raise ContextError("Git ignore probe failed") from error
    if result.returncode not in (0, 1):
        raise ContextError(
            f"Git ignore probe failed with exit status {result.returncode}"
        )
    return set(result.stdout.split(b"\0")) - {b""}


def discover_documents(
    repository: Path, reusable: Optional[SourceState] = None
) -> tuple[list[DocumentRow], SourceState, SourceState]:
    """Return changed documents, the retained cache subset, and the new cache.

    ``reusable`` maps already-indexed paths to the (mtime_ns, size) recorded by
    the last successful index. A candidate whose stat still matches is neither
    read nor re-validated, which is what makes per-request indexing affordable:
    secret scanning dominates a full pass. A modification that somehow reuses
    its predecessor's stat is still not served as truth — governed retrieval
    re-hashes every candidate and excludes the mismatch as stale.
    """
    documents: list[DocumentRow] = []
    retained: SourceState = {}
    state: SourceState = {}
    skill_keys: set[str] = set()
    claimed: set[str] = set()
    candidates: list[tuple[str, str, Path, str]] = []
    for layer, kind, pattern in SOURCE_PATTERNS:
        for path in sorted(repository.glob(pattern)):
            if not path.is_file() or path.is_symlink():
                continue
            candidates.append(
                (layer, kind, path, path.relative_to(repository).as_posix())
            )

    cache = reusable or {}
    ignored_paths = git_ignored_paths(
        repository, [relative_path for _, _, _, relative_path in candidates]
    )
    for layer, kind, path, relative_path in candidates:
        if os.fsencode(relative_path) in ignored_paths:
            continue
        if relative_path in claimed:
            # Two patterns can match one file, and the earlier one owns its
            # layer and kind. Without this the document is inserted twice and
            # the metadata primary key aborts the whole refresh.
            continue
        claimed.add(relative_path)
        skill_key = (
            relative_path.split("/skills/", maxsplit=1)[1] if kind == "skill" else None
        )
        if skill_key is not None and skill_key in skill_keys:
            # A mirrored copy of an already-indexed skill. Only a copy that
            # passes validation claims the key, so a later copy can never win
            # it and never needs to be read or scanned.
            continue
        try:
            status = path.stat()
        except OSError:
            continue
        current = (status.st_mtime_ns, status.st_size)
        if cache.get(relative_path) == current:
            # Unchanged since the last successful index, so it already passed
            # secret and active-memory validation; keep the existing row.
            if skill_key is not None:
                skill_keys.add(skill_key)
            retained[relative_path] = current
            state[relative_path] = current
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
        if skill_key is not None:
            skill_keys.add(skill_key)
        documents.append(
            (
                relative_path,
                layer,
                kind,
                document_title(path, content),
                document_summary(content),
                content,
            )
        )
        # Stat was taken before the read, so a write that races this pass is
        # detected next time instead of being cached away.
        state[relative_path] = current
    return documents, retained, state


def index_repository(
    connection: sqlite3.Connection, repository: Path, *, incremental: bool = False
) -> dict[str, object]:
    if incremental:
        reusable, fingerprints = reusable_source_state(connection, repository)
        documents, retained, state = discover_documents(repository, reusable)
        try:
            return index_documents(
                connection,
                repository,
                documents,
                retained=retained,
                source_state=state,
                fingerprints=fingerprints,
            )
        except RetrievalError:
            pass  # The cache disagreed with the index; fall through and rebuild.
    documents, _, state = discover_documents(repository)
    return index_documents(connection, repository, documents, source_state=state)


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
    relevant_only: bool = False,
) -> list[dict[str, object]]:
    """Search the index.

    ``relevant_only`` drops corpus-common terms and requires a document to
    contain more than one query term. Capsule assembly uses it, because a
    document that shares a single common word with the request is noise
    presented as context. Explicit `search` stays broad by design.
    """
    coverage: dict[str, int] = {}
    distinctive: set[str] = set()
    minimum = 0
    if relevant_only:
        tokens = informative_tokens(connection, query_tokens(query))
        coverage, distinctive = token_coverage(connection, tokens)
        minimum = required_coverage(tokens)
        match = " OR ".join(f'"{token}"' for token in tokens)
    else:
        match = fts_query(query)
    conditions = ["documents MATCH ?"]
    parameters: list[object] = [match]
    if layer is not None:
        conditions.append("layer = ?")
        parameters.append(layer)
    parameters.append(limit if not relevant_only else limit * 10)
    rows = connection.execute(
        f"""
        SELECT
            path,
            layer,
            kind,
            title,
            snippet(documents, 5, '[', ']', ' … ', 18) AS snippet
        FROM documents
        WHERE {' AND '.join(conditions)}
        ORDER BY bm25(documents), path
        LIMIT ?
        """,
        parameters,
    ).fetchall()
    selected = [
        row
        for row in rows
        if is_relevant(row["path"], coverage, distinctive, minimum)
    ] if relevant_only else rows
    return [dict(row) for row in selected[:limit]]


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


def reject_capsule_privacy(
    label: str,
    prose: list[str],
    identifiers: Optional[list[str]] = None,
) -> None:
    candidate = "\n".join((*prose, *(identifiers or [])))
    if (
        any(pattern.search(candidate) for pattern in CAPSULE_PRIVATE_PATTERNS)
        or CAPSULE_RAW_TEXT_PATTERN.search(candidate)
    ):
        raise ContextError(
            f"{label} contains private or raw data; "
            "replace it with a sanitized summary"
        )


def project_working_task(
    working: Optional[dict[str, object]],
) -> tuple[Optional[dict[str, object]], dict[str, int]]:
    omitted = {
        "working_files": 0,
        "working_next_steps": 0,
        "working_sources": 0,
        "working_progress_characters": 0,
    }
    if working is None:
        return None, omitted

    task_id = working.get("task_id")
    goal = working.get("goal")
    progress = working.get("progress")
    next_steps = working.get("next_steps")
    files = working.get("files")
    sources = working.get("sources")
    if (
        not isinstance(task_id, str)
        or not isinstance(goal, str)
        or not isinstance(progress, str)
        or not isinstance(next_steps, list)
        or not isinstance(files, list)
        or not isinstance(sources, list)
        or any(not isinstance(item, str) or not item.strip() for item in next_steps)
        or any(not isinstance(item, str) or not item.strip() for item in files)
        or any(not isinstance(item, str) or not item.strip() for item in sources)
    ):
        raise ContextError(
            "Working task cannot be projected; replace it with a sanitized summary"
        )

    values = [task_id, goal, progress, *next_steps, *files, *sources]
    reject_secrets("Task Capsule", values)
    reject_capsule_privacy(
        "Working task",
        [goal, progress, *next_steps],
        [task_id, *files, *sources],
    )

    normalized_steps = [" ".join(item.split()) for item in next_steps]
    omitted["working_files"] = max(
        0, len(files) - CAPSULE_WORKING_FILE_LIMIT
    )
    omitted["working_next_steps"] = max(0, len(normalized_steps) - 1)
    omitted["working_sources"] = max(
        0, len(sources) - CAPSULE_WORKING_SOURCE_LIMIT
    )
    return {
        "task_id": validate_task_id(task_id),
        "goal": " ".join(goal.split()),
        "progress": " ".join(progress.split()),
        "next_steps": normalized_steps[-1:],
        "files": files[:CAPSULE_WORKING_FILE_LIMIT],
        "sources": sources[:CAPSULE_WORKING_SOURCE_LIMIT],
    }, omitted


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
    reject_capsule_privacy("Task Capsule request", [query])

    capsule_warnings = list(warnings or [])
    working, omitted = project_working_task(
        find_working_task(connection, task_id)
    )
    if task_id is None:
        capsule_warnings.append("Working task unavailable: task ID was not supplied")
    elif working is None:
        capsule_warnings.append(f"Working task not found: {validate_task_id(task_id)}")

    request_query = build_capsule_query(query, None)
    packet: dict[str, object] = {
        "query": request_query,
        "task_id": task_id,
        "working": working,
        "procedural": [],
        "semantic": [],
        "episodic": [],
        "warnings": capsule_warnings,
        "omitted": omitted,
    }
    if not include_retrieval:
        return packet

    retrieval_query = build_capsule_query(request_query, working)
    procedural_limit = min(limit, CAPSULE_LAYER_LIMITS["procedural"])
    semantic_limit = min(limit, CAPSULE_LAYER_LIMITS["semantic"])
    episodic_limit = min(limit, CAPSULE_LAYER_LIMITS["episodic"])
    procedural = search_documents(
        connection, request_query, procedural_limit, "procedural",
        relevant_only=True,
    )
    semantic = search_documents(
        connection, request_query, semantic_limit, "semantic",
        relevant_only=True,
    )
    episodic = search_documents(
        connection, request_query, episodic_limit, "episodic",
        relevant_only=True,
    ) + search_episodes(connection, request_query, episodic_limit)
    if retrieval_query != request_query:
        if len(deduplicate_context_items(procedural)) < procedural_limit:
            procedural += search_documents(
                connection, retrieval_query, procedural_limit, "procedural",
                relevant_only=True,
            )
        if len(deduplicate_context_items(semantic)) < semantic_limit:
            semantic += search_documents(
                connection, retrieval_query, semantic_limit, "semantic",
                relevant_only=True,
            )
        if len(deduplicate_context_items(episodic)) < episodic_limit:
            episodic += search_documents(
                connection, retrieval_query, episodic_limit, "episodic",
                relevant_only=True,
            ) + search_episodes(connection, retrieval_query, episodic_limit)
    packet["procedural"] = deduplicate_context_items(procedural)[:procedural_limit]
    packet["semantic"] = deduplicate_context_items(semantic)[:semantic_limit]
    packet["episodic"] = deduplicate_context_items(episodic)[:episodic_limit]
    return packet


def serialize_capsule(capsule: dict[str, object]) -> str:
    return json.dumps(capsule, ensure_ascii=False, separators=(",", ":"))


def capsule_character_count(capsule: dict[str, object]) -> int:
    return len(serialize_capsule(capsule))


def enforce_capsule_budget(capsule: dict[str, object]) -> dict[str, object]:
    compacted = json.loads(serialize_capsule(capsule))
    omitted = compacted["omitted"]
    for key in (
        "working_files",
        "working_next_steps",
        "working_sources",
        "working_progress_characters",
    ):
        omitted.setdefault(key, 0)
    working = compacted["working"]

    while capsule_character_count(compacted) > CAPSULE_CHARACTER_LIMIT:
        if compacted["episodic"]:
            compacted["episodic"].pop()
            continue
        if compacted["semantic"]:
            compacted["semantic"].pop()
            continue
        if working is not None and len(working["sources"]) > 1:
            working["sources"].pop()
            omitted["working_sources"] += 1
            continue
        if working is not None and len(working["files"]) > 1:
            working["files"].pop()
            omitted["working_files"] += 1
            continue
        if working is not None and working["progress"]:
            excess = capsule_character_count(compacted) - CAPSULE_CHARACTER_LIMIT
            keep = max(0, len(working["progress"]) - excess - 1)
            if keep == 0:
                raise ContextError(
                    "mandatory Task Capsule content exceeds "
                    f"{CAPSULE_CHARACTER_LIMIT} characters"
                )
            omitted["working_progress_characters"] += (
                len(working["progress"]) - keep
            )
            working["progress"] = f"…{working['progress'][-keep:]}"
            continue
        raise ContextError(
            "mandatory Task Capsule content exceeds "
            f"{CAPSULE_CHARACTER_LIMIT} characters"
        )

    return compacted


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
    task = {
        "task_id": row["task_id"],
        "goal": row["goal"],
        "progress": row["progress"],
        "next_steps": json.loads(row["next_steps"]),
        "files": json.loads(row["files"]),
        "sources": json.loads(row["sources"]),
        "created_at": row["created_at"],
        "updated_at": row["updated_at"],
    }
    project_working_task(task)
    return task


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
    reject_capsule_privacy(
        "Working task",
        [goal],
        [task_id, *files, *sources],
    )
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
    reject_capsule_privacy(
        "Working task",
        ([progress] if progress is not None else []) + next_steps,
        [task_id, *files, *sources],
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
        "phase": record.get("phase"),
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


def codebase_map_status(
    connection: sqlite3.Connection, repository: Path
) -> list[dict[str, object]]:
    """Report how far each codebase map has drifted from the code it describes.

    Retrieval excludes a drifted map, but silently: an operator who never sees
    the number cannot know the map needs regenerating.
    """
    try:
        rows = connection.execute(
            "SELECT path FROM documents WHERE kind = 'codebase' ORDER BY path"
        ).fetchall()
    except sqlite3.Error:
        return []
    status: list[dict[str, object]] = []
    for row in rows:
        path = repository / row["path"]
        try:
            content = path.read_text(encoding="utf-8")
        except OSError:
            continue
        drift = codebase_map_drift(repository, content)
        status.append({"path": row["path"], "commits_behind": drift})
    return status


def refresh_layers(
    connection: sqlite3.Connection, repository: Path
) -> tuple[dict[str, str], dict[str, object], list[str]]:
    """Re-index every memory layer and report each one separately.

    Procedural (policy and skills), semantic (overview, specs, docs, active
    Memory Bank chunks, task documents, eligible Brain records), and episodic
    (changelog) all come from one indexing pass, so they succeed or fail
    together. They are still reported per layer: a caller that is told only
    "refresh failed" cannot tell which part of its context went stale.
    """
    try:
        result = index_repository(connection, repository, incremental=True)
    except (ContextError, RetrievalError, OSError, sqlite3.Error) as error:
        return (
            {layer: "failed" for layer in DOCUMENT_LAYERS},
            {},
            [f"Index refresh failed: {error}"],
        )
    return {layer: "updated" for layer in DOCUMENT_LAYERS}, result, []


def assemble_capsule(
    connection: sqlite3.Connection,
    repository: Path,
    *,
    mode: str,
    query: str,
    task_id: Optional[str],
    limit: int,
    ephemeral: bool,
    refresh_index: bool = True,
) -> dict[str, object]:
    """Build the layered capsule for one request.

    ``refresh_index`` exists so a caller that already refreshed does not index
    twice; retrieval reads the index rather than the sources, so the refresh
    has to happen somewhere before this runs.
    """
    warnings: list[str] = []
    if refresh_index:
        _, _, warnings = refresh_layers(connection, repository)
    if mode == "lightweight":
        result = build_context_packet(
            connection,
            query,
            task_id,
            limit,
            include_retrieval=not warnings,
            warnings=warnings,
        )
        # Character budget applies only here: retrieve() below runs its own
        # token budget (TARGET_BUDGET/HARD_BUDGET) and its payload legitimately
        # exceeds CAPSULE_CHARACTER_LIMIT.
        return enforce_capsule_budget(result)
    if task_id is None:
        raise ContextError("Governed retrieval requires --task-id")
    # The governed path bypasses build_context_packet, so apply the same query
    # guards build_context_packet would have applied.
    reject_secrets("Task Capsule", [query])
    reject_capsule_privacy("Task Capsule request", [query])
    binding = governed_binding(connection, task_id)
    result = retrieve(
        connection,
        repository,
        query,
        binding["task_uuid"],
        limit=limit,
        manifest_scope="local" if ephemeral else "governed",
    )
    result["episodic"] = (
        search_documents(connection, query, limit, "episodic", relevant_only=True)
        + search_episodes(connection, query, limit)
    )[:limit]
    # The print tail dereferences result["warnings"]; retrieve() has no such key.
    result["warnings"] = warnings
    return result


def print_capsule(capsule: dict[str, object]) -> None:
    working = capsule["working"]
    if working is None:
        print("working: unavailable")
    else:
        print(f"working: {working['task_id']} — {working['goal']}")
    for warning in capsule["warnings"]:
        print(f"warning: {warning}")
    for layer in DOCUMENT_LAYERS:
        items = capsule[layer]
        if not items:
            continue
        print(f"{layer}:")
        for item in items:
            label = item["path"] if "path" in item else f"episode {item['id']}"
            title = item["title"] if "title" in item else item["summary"]
            print(f"  {label} — {title}")


def revision_argument(value: str) -> object:
    """Accept an explicit revision or 'auto' for a locked read-modify-write."""
    if value == AUTO_REVISION:
        return AUTO_REVISION
    try:
        revision = int(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError(
            f"Revision must be a positive integer or '{AUTO_REVISION}'"
        ) from error
    if revision < 1:
        raise argparse.ArgumentTypeError(
            f"Revision must be a positive integer or '{AUTO_REVISION}'"
        )
    return revision


def resolve_revision(
    repository: Path, identifier: str, requested: object
) -> Optional[int]:
    """Resolve 'auto' against the record on disk.

    Call this inside the caller's ``mutation_lock`` so the read and the write
    it feeds form one compare-and-swap. Automated writers cannot know the
    revision in advance, and passing no revision at all would skip the check
    entirely rather than perform it.
    """
    if requested != AUTO_REVISION:
        return requested  # type: ignore[return-value]
    return int(get_record(repository, identifier)["revision"])


def apply_governed_update(
    connection: sqlite3.Connection,
    repository: Path,
    binding: sqlite3.Row,
    *,
    expected_revision: object,
    progress: Optional[str],
    next_steps: list[str],
    files: list[str],
    sources: list[str],
    owner: str,
    phase: Optional[str] = None,
) -> dict[str, object]:
    """Update the authoritative Brain task and its local compatibility binding."""
    with mutation_lock(repository):
        revision = resolve_revision(
            repository, binding["task_uuid"], expected_revision
        )
        snapshot = snapshot_record_state(repository, binding["task_uuid"])
        try:
            task = update_task(
                repository,
                binding["task_uuid"],
                expected_revision=revision,
                progress=progress,
                next_steps=next_steps,
                files=files,
                sources=sources,
                actor=owner,
                phase=phase,
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
    return governed_task_view(task)


def git_output(repository: Path, arguments: list[str], label: str) -> bytes:
    try:
        result = subprocess.run(
            ["git", "-C", str(repository), *arguments],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            check=False,
        )
    except OSError as error:
        raise ContextError(f"{label} failed") from error
    if result.returncode != 0:
        raise ContextError(f"{label} failed with exit status {result.returncode}")
    return result.stdout


def derive_goal(task_id: str) -> str:
    """Build a task goal from a branch name.

    A task goal cannot be edited after creation, so an automatically created
    one states its own provenance instead of pretending someone wrote it.
    """
    label = task_id
    for prefix in BRANCH_PREFIXES:
        if label.lower().startswith(prefix):
            label = label[len(prefix):]
            break
    # A ticket identifier spells its own hyphen, so protect it before treating
    # the remaining hyphens as word separators: BAUMAS-133 is a name, not two.
    tickets = TICKET_PATTERN.findall(label)
    for index, ticket in enumerate(tickets):
        label = label.replace(ticket, f"\x00{index}\x00", 1)
    label = " ".join(part for part in re.split(r"[-_/]+", label) if part)
    for index, ticket in enumerate(tickets):
        label = label.replace(f"\x00{index}\x00", ticket)
    label = f"{label[:1].upper()}{label[1:]}" if label.strip() else task_id
    return f"{label} (auto-provisioned from {task_id})"


def create_governed_task(
    connection: sqlite3.Connection,
    repository: Path,
    task_id: str,
    goal: str,
    files: list[str],
    sources: list[str],
    owner: str,
) -> dict[str, object]:
    """Create the Brain task and its local binding, or leave neither behind."""
    with mutation_lock(repository):
        task = create_task(repository, task_id, goal, files, sources, owner=owner)
        try:
            bind_governed_task(
                connection, task_id, str(task["id"]), int(task["revision"])
            )
        except Exception:
            try:
                compensate_governed_binding(connection, task_id, revision=None)
            finally:
                rollback_created_record(repository, str(task["id"]))
            raise
    return task


def ensure_working_task(
    connection: sqlite3.Connection,
    repository: Path,
    task_id: str,
    mode: str,
    owner: str,
) -> bool:
    """Create the working task the first time a turn has something to record.

    Automated continuity is worthless if it buffers into a task nobody created.
    Provisioning happens at flush time rather than at session start, so merely
    visiting a branch does not mint a record; only accumulated real work does.

    Returns whether a task was created.
    """
    goal = derive_goal(task_id)
    reject_secrets("working task", [task_id, goal])
    reject_capsule_privacy("Working task", [goal], [task_id])
    if mode == "lightweight":
        if find_working_task(connection, task_id) is not None:
            return False
        start_working_task(connection, task_id, goal, [], [])
        return True
    try:
        governed_binding(connection, task_id)
        return False
    except ContextError:
        create_governed_task(connection, repository, task_id, goal, [], [], owner)
        return True


def complete_governed_task(
    connection: sqlite3.Connection,
    repository: Path,
    binding: sqlite3.Row,
    *,
    outcome: str,
    summary: Optional[str],
    files: list[str],
    verification: list[str],
    sources: list[str],
    expected_revision: object,
    owner: str,
) -> dict[str, object]:
    """Close the Brain task, record the episode, and drop the local binding."""
    task = get_task(repository, binding["task_uuid"])
    with mutation_lock(repository):
        snapshot = snapshot_record_state(repository, binding["task_uuid"])
        try:
            connection.execute("BEGIN IMMEDIATE")
            episode_id = insert_episode(
                connection,
                str(task["goal"]) if summary is None else summary,
                outcome,
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
            completed = close_task(
                repository,
                binding["task_uuid"],
                outcome,
                verification,
                expected_revision=resolve_revision(
                    repository, binding["task_uuid"], expected_revision
                ),
                actor=owner,
            )
            connection.commit()
        except Exception:
            connection.rollback()
            restore_record_state(repository, snapshot)
            raise
    return {
        "episode_id": episode_id,
        "task_uuid": completed["id"],
        "revision": completed["revision"],
        "status": completed["status"],
    }


def default_branch(repository: Path) -> Optional[str]:
    """Resolve the branch a merge would land in."""
    configured = os.environ.get("CONTEXT_DEFAULT_BRANCH")
    if configured:
        return configured.strip() or None
    try:
        head = os.fsdecode(
            git_output(
                repository,
                ["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"],
                "Git default-branch probe",
            )
        ).strip()
    except ContextError:
        head = ""
    if head:
        return head
    for candidate in ("main", "master"):
        try:
            git_output(
                repository,
                ["rev-parse", "--verify", "--quiet", f"refs/heads/{candidate}"],
                "Git branch probe",
            )
        except ContextError:
            continue
        return candidate
    return None


def branch_is_merged(repository: Path, branch: str, target: str) -> bool:
    """Report whether `branch` has landed in `target`.

    Ancestry alone is not enough. A branch created a moment ago and never
    committed to is also an ancestor of its target, so requiring only that
    would complete a task the instant it was provisioned. The target must also
    have moved ahead of the branch, which is what a merge does.

    Merging the target *into* a branch does not make the branch an ancestor, so
    a long-running branch that merely keeps itself current stays open.

    A fast-forward merge leaves the two refs identical and is therefore
    indistinguishable from a branch that never diverged; it is not detected.
    Leaving a task open costs a stale record, while closing one wrongly ends
    its lifecycle and archives it.
    """
    for reference in (f"refs/heads/{branch}", branch):
        try:
            git_output(
                repository,
                ["rev-parse", "--verify", "--quiet", reference],
                "Git branch probe",
            )
        except ContextError:
            continue
        try:
            git_output(
                repository,
                ["merge-base", "--is-ancestor", reference, target],
                "Git merge probe",
            )
        except ContextError:
            return False
        ahead = os.fsdecode(
            git_output(
                repository,
                ["rev-list", "--count", f"{reference}..{target}"],
                "Git divergence probe",
            )
        ).strip()
        return ahead.isdigit() and int(ahead) > 0
    # A deleted branch cannot be told apart from an abandoned one, so it is
    # never treated as merged.
    return False


def close_merged_tasks(
    connection: sqlite3.Connection, repository: Path, owner: str
) -> list[dict[str, object]]:
    """Complete governed tasks whose branch has landed in the default branch.

    The scan covers every active task rather than the current one: a merge is
    observed after the branch is left, not while it is being worked on.
    """
    config = load_config(repository)
    if not config.get("automatic_completion"):
        return []
    target = default_branch(repository)
    if target is None:
        return []
    closed: list[dict[str, object]] = []
    for _, record, _ in iter_records(repository):
        if record["type"] != "task" or record["status"] in LIFECYCLES["task"]["terminal"]:
            continue
        branch = str(record["external_id"])
        # A branch is its own ancestor, so the default branch would close
        # itself the moment a task were provisioned on it.
        if branch == target or branch == target.split("/")[-1]:
            continue
        if not branch_is_merged(repository, branch, target):
            continue
        # State the fact that was actually verified — the merge — rather than
        # a claim about the work being correct, which nothing here checked.
        outcome = (
            f"Branch {branch} merged into {target}. "
            f"Last recorded progress: {record['progress'] or 'none recorded'}"
        )
        verification = [f"{branch} is an ancestor of {target}"]
        try:
            binding = governed_binding(connection, branch)
        except ContextError:
            # Bound on another machine: close the shared record without
            # inventing a local episode for work this machine did not do.
            with mutation_lock(repository):
                completed = close_task(
                    repository,
                    str(record["id"]),
                    outcome,
                    verification,
                    expected_revision=record["revision"],
                    actor=owner,
                )
            closed.append(
                {"task_id": branch, "task_uuid": completed["id"], "episode_id": None}
            )
            continue
        result = complete_governed_task(
            connection,
            repository,
            binding,
            outcome=outcome,
            summary=None,
            files=[],
            verification=verification,
            sources=[],
            expected_revision=AUTO_REVISION,
            owner=owner,
        )
        closed.append(
            {
                "task_id": branch,
                "task_uuid": result["task_uuid"],
                "episode_id": result["episode_id"],
            }
        )
    return closed


def changed_paths(repository: Path) -> tuple[list[str], list[str]]:
    """Return sanitized changed paths from Git porcelain metadata alone.

    Working-tree contents are never read here. A path list is everything the
    short-term buffer needs, and reading the files would bypass the secret and
    privacy gates that the indexer applies to sources it does read.

    Paths the runtime writes itself are dropped rather than reported: a task
    recording its own record, handoff, and index churn as user work would grow
    without ever describing anything the user changed.
    """
    prefix = os.fsdecode(
        git_output(repository, ["rev-parse", "--show-prefix"], "Git prefix probe")
    ).strip()
    runtime_owned = re.compile(
        rf"^{re.escape(prefix)}project-brain/"
        rf"({'|'.join(TURN_RUNTIME_DIRECTORIES)})/"
    )
    fields = git_output(
        repository,
        [
            "status", "--porcelain=v1", "-z", "--untracked-files=all", "--", ".",
        ],
        "Git status probe",
    ).split(b"\0")
    paths: list[str] = []
    position = 0
    while position < len(fields):
        entry = fields[position]
        position += 1
        if len(entry) < 4:
            continue
        if entry[:1] in b"RC":
            # Rename and copy entries carry their origin in the next field.
            position += 1
        path = os.fsdecode(entry[3:])
        if not runtime_owned.match(path):
            paths.append(path)
    excluded = sorted({item for item in paths if TURN_PATH_DENYLIST.search(item)})
    allowed = [item for item in dict.fromkeys(paths) if item not in set(excluded)]
    return allowed, excluded


def append_turn_delta(
    connection: sqlite3.Connection, task_id: str, files: list[str]
) -> int:
    with connection:
        cursor = connection.execute(
            "INSERT INTO turn_deltas(task_id, created_at, files) VALUES (?, ?, ?)",
            (
                task_id,
                datetime.now(timezone.utc).isoformat(),
                json.dumps(files, ensure_ascii=False),
            ),
        )
    return int(cursor.lastrowid)


def pending_turn_deltas(
    connection: sqlite3.Connection, task_id: str
) -> list[sqlite3.Row]:
    return connection.execute(
        "SELECT id, created_at, files FROM turn_deltas WHERE task_id = ? ORDER BY id",
        (task_id,),
    ).fetchall()


def flush_turn_deltas(
    connection: sqlite3.Connection,
    repository: Path,
    task_id: str,
    mode: str,
    owner: str,
    file_limit: int = DEFAULT_TURN_FILE_LIMIT,
) -> Optional[dict[str, object]]:
    """Consolidate buffered turns into one authoritative working-memory write.

    Buffering exists so that per-turn continuity does not cost one governed
    revision and one rewritten handoff per turn. Deltas are removed only after
    the authoritative write lands, so an interrupted flush replays rather than
    loses the buffer.

    ``file_limit`` bounds how many paths one flush contributes. Task files are
    merge-only, so an unbounded automated writer would grow the record and its
    handoff indefinitely. The count that did not fit is reported, never dropped
    silently.
    """
    pending = pending_turn_deltas(connection, task_id)
    if not pending:
        return None
    files: list[str] = []
    for row in pending:
        files = merge_unique(files, json.loads(row["files"]))
    omitted = max(0, len(files) - file_limit)
    progress = (
        f"Auto-checkpoint: {len(pending)} turn(s) since {pending[0]['created_at']}, "
        f"{len(files)} file(s) touched."
    )
    if omitted:
        progress += f" {omitted} path(s) beyond the per-flush limit are not listed."
    files = files[:file_limit]
    provisioned = ensure_working_task(connection, repository, task_id, mode, owner)
    if mode == "lightweight":
        result = update_working_task(connection, task_id, progress, [], files, [])
    else:
        result = apply_governed_update(
            connection,
            repository,
            governed_binding(connection, task_id),
            expected_revision=AUTO_REVISION,
            progress=progress,
            next_steps=[],
            files=files,
            sources=[],
            owner=owner,
        )
    with connection:
        connection.executemany(
            "DELETE FROM turn_deltas WHERE id = ?",
            [(row["id"],) for row in pending],
        )
    return {**result, "files_omitted": omitted, "provisioned": provisioned}


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
    index.add_argument(
        "--incremental",
        action="store_true",
        help="reuse rows whose source stat is unchanged instead of rebuilding",
    )
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

    retrieve_command = commands.add_parser(
        "retrieve", help="assemble governed, budgeted task context"
    )
    retrieve_command.add_argument("query")
    retrieve_command.add_argument("--task-id", required=True)
    retrieve_command.add_argument("--limit", type=int, default=3)
    retrieve_command.add_argument("--json", action="store_true")

    for retrieval_parser in (context, retrieve_command):
        retrieval_parser.add_argument(
            "--ephemeral",
            action="store_true",
            help=(
                "write the governed retrieval manifest to ignored local state; "
                "use for automated retrieval that would otherwise flood shared "
                "history (no effect in lightweight mode, which writes none)"
            ),
        )

    refresh = commands.add_parser(
        "refresh",
        help="refresh every indexed memory layer and report each one",
    )
    refresh.add_argument(
        "--query",
        help="also assemble a Task Capsule for this request (requires --task-id)",
    )
    refresh.add_argument("--task-id")
    refresh.add_argument("--limit", type=int, default=3)
    refresh.add_argument(
        "--ephemeral",
        action="store_true",
        help="write the capsule's retrieval manifest to ignored local state",
    )
    refresh.add_argument(
        "--validate",
        action="store_true",
        help="also report Project Brain validation (reads every record)",
    )
    refresh.add_argument("--json", action="store_true")

    turn = commands.add_parser(
        "turn", help="buffer a per-turn working-memory delta and flush on a boundary"
    )
    turn.add_argument("--task-id", required=True)
    turn.add_argument(
        "--flush", action="store_true", help="flush regardless of the threshold"
    )
    turn.add_argument(
        "--flush-after",
        type=int,
        default=DEFAULT_TURN_FLUSH_AFTER,
        help="buffered turns to accumulate before one authoritative write",
    )
    turn.add_argument(
        "--max-files",
        type=int,
        default=DEFAULT_TURN_FILE_LIMIT,
        help="paths one flush may add to the task; the remainder is reported",
    )
    turn.add_argument("--json", action="store_true")

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
    update.add_argument("--phase", choices=TASK_PHASES)
    update.add_argument("--revision", type=revision_argument)
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
    complete.add_argument("--revision", type=revision_argument)
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
    update_brain.add_argument("--revision", type=revision_argument, required=True)
    update_brain.add_argument("--progress")
    update_brain.add_argument("--next-step", action="append", default=[])
    update_brain.add_argument("--file", action="append", default=[])
    update_brain.add_argument("--source", action="append", default=[])
    update_brain.add_argument("--conflict", action="append", default=[])
    update_brain.add_argument("--phase", choices=TASK_PHASES)
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
                result = index_repository(
                    connection, repository, incremental=arguments.incremental
                )
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    print(
                        f"Context index: {result['documents']} documents "
                        f"({result['removed']} removed, {result['reused']} reused)."
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
                    result = governed_task_view(
                        create_governed_task(
                            connection, repository, task_id, goal, files, sources, owner
                        )
                    )
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
                        arguments.next_step
                        or arguments.file
                        or arguments.source
                        or arguments.phase
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
                    result = apply_governed_update(
                        connection,
                        repository,
                        binding,
                        expected_revision=arguments.revision,
                        progress=progress.strip() if progress else None,
                        next_steps=next_steps,
                        files=files,
                        sources=sources,
                        owner=owner,
                        phase=arguments.phase,
                    )
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
                    result = complete_governed_task(
                        connection,
                        repository,
                        governed_binding(connection, arguments.task_id),
                        outcome=arguments.outcome,
                        summary=arguments.summary,
                        files=validate_paths("Episode file", arguments.file),
                        verification=normalize_values(
                            "Episode verification", arguments.verification
                        ),
                        sources=normalize_values("Episode source", arguments.source),
                        expected_revision=arguments.revision,
                        owner=owner,
                    )
                    episode_id = result["episode_id"]
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
                    and arguments.phase is None
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
                with mutation_lock(repository):
                    result = update_record(
                        repository,
                        arguments.record_id,
                        expected_revision=resolve_revision(
                            repository, arguments.record_id, arguments.revision
                        ),
                        progress=progress,
                        next_steps=next_steps,
                        files=files,
                        sources=sources,
                        actor=owner,
                        conflicts=conflicts,
                        transition_to=arguments.transition,
                        phase=arguments.phase,
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
                result = assemble_capsule(
                    connection,
                    repository,
                    mode=mode,
                    query=arguments.query,
                    task_id=arguments.task_id,
                    limit=arguments.limit,
                    ephemeral=arguments.ephemeral,
                )
                if arguments.json:
                    print(serialize_capsule(result))
                else:
                    print_capsule(result)
                return 0

            if arguments.command == "refresh":
                if arguments.limit < 1:
                    raise ContextError("--limit must be a positive integer")
                if arguments.query is not None and arguments.task_id is None:
                    raise ContextError("--query requires --task-id")
                layers, index_result, warnings = refresh_layers(
                    connection, repository
                )
                capsule = None
                if arguments.query is not None:
                    try:
                        capsule = assemble_capsule(
                            connection,
                            repository,
                            mode=mode,
                            query=arguments.query,
                            task_id=arguments.task_id,
                            limit=arguments.limit,
                            ephemeral=arguments.ephemeral,
                            refresh_index=False,
                        )
                    except (ContextError, BrainError, RetrievalError) as error:
                        # A capsule needs an active task; the layer refresh
                        # above stands on its own and is already done.
                        warnings.append(f"Capsule unavailable: {error}")
                codebase = codebase_map_status(connection, repository)
                result = {
                    "mode": mode,
                    **layers,
                    "codebase": codebase,
                    "documents": index_result.get(
                        "documents",
                        connection.execute(
                            "SELECT COUNT(*) FROM documents"
                        ).fetchone()[0],
                    ),
                    "reused": index_result.get("reused", 0),
                    "counts": index_result.get("layers", {}),
                    "parity_drift": index_result.get("parity_drift", []),
                    "warnings": warnings,
                    "capsule": capsule,
                }
                if arguments.validate:
                    errors = validate_repository(repository)
                    result["brain_validation"] = "valid" if not errors else "invalid"
                    warnings.extend(errors)
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    counts = result["counts"]
                    for layer in DOCUMENT_LAYERS:
                        suffix = (
                            f" ({counts[layer]} documents)" if layer in counts else ""
                        )
                        print(f"{layer}: {result[layer]}{suffix}")
                    for item in codebase:
                        behind = item["commits_behind"]
                        print(
                            "codebase map: %s — %s"
                            % (
                                item["path"],
                                "unverifiable"
                                if behind is None
                                else f"{behind} commit(s) behind",
                            )
                        )
                    if "brain_validation" in result:
                        print(f"brain-validation: {result['brain_validation']}")
                    for warning in warnings:
                        print(f"warning: {warning}")
                    if capsule is not None:
                        print_capsule(capsule)
                return 0 if all(
                    layers[layer] == "updated" for layer in DOCUMENT_LAYERS
                ) else 1

            if arguments.command == "turn":
                if arguments.flush_after < 1:
                    raise ContextError("--flush-after must be a positive integer")
                if arguments.max_files < 1:
                    raise ContextError("--max-files must be a positive integer")
                task_id = validate_task_id(arguments.task_id)
                files, path_excluded = changed_paths(repository)
                delta_id = (
                    append_turn_delta(connection, task_id, files) if files else None
                )
                buffered = len(pending_turn_deltas(connection, task_id))
                flushed = None
                if buffered and (arguments.flush or buffered >= arguments.flush_after):
                    flushed = flush_turn_deltas(
                        connection,
                        repository,
                        task_id,
                        mode,
                        owner,
                        file_limit=arguments.max_files,
                    )
                # A merge is observed after the branch is left, so this scans
                # every active task rather than only the current one.
                closed: list[dict[str, object]] = []
                if mode != "lightweight":
                    try:
                        closed = close_merged_tasks(connection, repository, owner)
                    except (BrainError, OSError) as error:
                        raise ContextError(
                            f"Automatic completion failed: {error}"
                        ) from error
                # Durable memory is promoted on the same boundary as the
                # working-memory flush, so the whole pipeline runs unattended.
                promotion = {
                    "enabled": False, "promoted": [], "failed": [],
                    "blocked": [], "skipped": 0,
                }
                if mode != "lightweight":
                    try:
                        promotion = auto_promote(repository, owner=owner)
                    except (BrainError, OSError) as error:
                        raise ContextError(
                            f"Automatic promotion failed: {error}"
                        ) from error
                # Archiving runs last: a record must be promoted before it is
                # moved, and compaction batches so Git history is not churned
                # one terminal record at a time.
                compaction = {"enabled": False, "moved": 0, "pending": 0, "error": None}
                if mode != "lightweight":
                    compaction = auto_compact(repository, owner=owner)
                result = {
                    "task_id": task_id,
                    "delta_id": delta_id,
                    "files": len(files),
                    "excluded": path_excluded,
                    "pending": 0 if flushed else buffered,
                    "flushed": flushed is not None,
                    "revision": flushed.get("revision") if flushed else None,
                    "files_omitted": flushed.get("files_omitted") if flushed else 0,
                    "provisioned": bool(flushed and flushed.get("provisioned")),
                    "closed": closed,
                    "promoted": promotion["promoted"],
                    "promotion_failed": promotion["failed"],
                    "promotion_blocked": promotion["blocked"],
                    "promotion_skipped": promotion["skipped"],
                    "archived": compaction["moved"],
                    "archivable_pending": compaction["pending"],
                }
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                elif flushed:
                    print(
                        f"Turn buffer flushed to {task_id}: "
                        f"{result['files']} file(s) in this turn."
                    )
                else:
                    print(
                        f"Turn buffered for {task_id}: {result['pending']} pending, "
                        f"{result['files']} file(s) in this turn."
                    )
                if not arguments.json:
                    for item in closed:
                        print(f"task completed on merge: {item['task_id']}")
                    for item in promotion["promoted"]:
                        print(
                            f"promoted without review: {item['type']} -> "
                            f"{item['memory_id']}"
                        )
                    for item in promotion["failed"]:
                        print(f"promotion failed: {item['reason']}")
                    for item in promotion["blocked"]:
                        print(f"promotion blocked: {item['reason']}")
                    if compaction["moved"]:
                        print(f"archived: {compaction['moved']} terminal record(s)")
                    if compaction["error"]:
                        print(f"compaction failed: {compaction['error']}")
                    if promotion["skipped"]:
                        print(
                            f"promotion deferred: {promotion['skipped']} more "
                            "eligible record(s) this run"
                        )
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
