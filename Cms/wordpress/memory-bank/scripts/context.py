#!/usr/bin/env python3
"""Canonical Project Brain + repository-local context CLI facade."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sqlite3
import stat as stat_module
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional

from brain_runtime import (
    BrainError,
    LIFECYCLES,
    MESSAGE_BODY_LIMIT,
    MESSAGE_TYPES,
    TASK_PHASE_INPUTS,
    TERMINAL_STATES,
    append_message,
    atomic_json,
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
    read_messages,
    reindex_bank,
    render_current_state,
    rollback_created_record,
    review_promotion,
    apply_promotion,
    restore_record_state,
    snapshot_record_state,
    update_record,
    update_task,
    validate_actor,
    validate_repository,
)
from context_retrieval import (
    DocumentRow,
    RetrievalError,
    STOPWORD_DOCUMENT_RATIO,
    SourceState,
    index_fingerprints,
    informative_tokens,
    token_document_frequencies,
    assert_skill_mirror_parity,
    cross_edition_drift,
    format_cross_edition_drift,
    format_full_mirror_drift,
    format_skill_mirror_drift,
    full_mirror_drift,
    skill_mirror_drift,
    ensure_metadata_tables,
    index_documents,
    load_index_state,
    store_index_state,
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
    # Task/ (client product specifications and design references) is
    # deliberately not a retrieval source: its prose is foreign to this
    # repository's own documentation and competed with it in BM25 ranking.
    ("episodic", "changelog", "CHANGELOG.md"),
)
DOCUMENT_LAYERS = ("procedural", "semantic", "episodic")
CAPSULE_LAYER_LIMITS = {
    "procedural": 2,
    "semantic": 3,
    "episodic": 1,
}
CAPSULE_QUERY_TOKEN_LIMIT = 32
# How many prompt terms survive distillation into the retrieval query. Rarity
# in the index decides which ones, so the cap bounds cost without deciding
# relevance by position the way the old first-N-words hook extraction did.
CAPSULE_PROMPT_TERM_LIMIT = 24
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
LAST_TURN_REPORT_FILENAME = "last-turn-report.json"
# The report describes one Stop-hook turn, so it is only meaningful for the
# session that produced it. A day bounds any plausible gap between two working
# sessions; anything older describes work the operator no longer remembers.
LAST_TURN_REPORT_MAX_AGE_SECONDS = 24 * 60 * 60
# The rendered section is telemetry, not retrieved knowledge, so it may never
# crowd the capsule: one compact line, problems first.
LAST_TURN_SECTION_CHARACTER_LIMIT = 600
LAST_TURN_SECTION_PATH_LIMIT = 3
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
                auto_checkpoint TEXT NOT NULL DEFAULT '',
                next_steps TEXT NOT NULL,
                files TEXT NOT NULL,
                sources TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )
            """
        )
        # Working state is user data, so a table written before the automatic
        # checkpoint column existed is migrated in place rather than rebuilt:
        # the empty default means "no checkpoint yet", exactly what an older
        # row asserts.
        working_columns = {
            row["name"]
            for row in connection.execute("PRAGMA table_info(working_tasks)")
        }
        if "auto_checkpoint" not in working_columns:
            connection.execute(
                "ALTER TABLE working_tasks "
                "ADD COLUMN auto_checkpoint TEXT NOT NULL DEFAULT ''"
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


# One discovered source file: layer, kind, absolute path, repository-relative
# path, and the (mtime_ns, size) stat pair every downstream consumer needs.
SourceCandidate = tuple[str, str, Path, str, tuple[int, int]]


def collect_source_candidates(repository: Path) -> list[SourceCandidate]:
    """Walk SOURCE_PATTERNS once, statting each candidate exactly once.

    Document discovery and the skill-tree fingerprint used to walk and stat
    the same trees independently — hundreds of mirrored skill files statted
    twice on every prompt. Everything a consumer needs from the filesystem is
    captured here, so both derive from a single pass. One lstat per file also
    answers regular-vs-symlink: a symlink is excluded as before, only without
    the extra stat calls `is_file()`/`is_symlink()` would each spend.
    """
    candidates: list[SourceCandidate] = []
    for layer, kind, pattern in SOURCE_PATTERNS:
        for path in sorted(repository.glob(pattern)):
            try:
                status = os.lstat(path)
            except OSError:
                continue
            if not stat_module.S_ISREG(status.st_mode):
                continue
            candidates.append(
                (
                    layer,
                    kind,
                    path,
                    path.relative_to(repository).as_posix(),
                    (status.st_mtime_ns, status.st_size),
                )
            )
    return candidates


def discover_documents(
    repository: Path,
    reusable: Optional[SourceState] = None,
    candidates: Optional[list[SourceCandidate]] = None,
) -> tuple[list[DocumentRow], SourceState, SourceState]:
    """Return changed documents, the retained cache subset, and the new cache.

    ``reusable`` maps already-indexed paths to the (mtime_ns, size) recorded by
    the last successful index. A candidate whose stat still matches is neither
    read nor re-validated, which is what makes per-request indexing affordable:
    secret scanning dominates a full pass. A modification that somehow reuses
    its predecessor's stat is still not served as truth — governed retrieval
    re-hashes every candidate and excludes the mismatch as stale.

    ``candidates`` accepts an already-collected source walk so a caller that
    also needs the stats elsewhere (the skill-tree fingerprint) pays for the
    filesystem pass once.
    """
    documents: list[DocumentRow] = []
    retained: SourceState = {}
    state: SourceState = {}
    skill_keys: set[str] = set()
    claimed: set[str] = set()
    if candidates is None:
        candidates = collect_source_candidates(repository)

    cache = reusable or {}
    ignored_paths = git_ignored_paths(
        repository, [relative_path for _, _, _, relative_path, _ in candidates]
    )
    for layer, kind, path, relative_path, current in candidates:
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
    """Index the repository, reporting how long each phase took.

    ``phase_seconds`` separates the filesystem walk (``stat``) from the
    database rebuild (``index``): the per-prompt hook runs under a hard
    budget, and a caller that only sees the total cannot tell which side is
    approaching it.
    """
    scan_seconds = 0.0
    phase_started = time.monotonic()
    candidates = collect_source_candidates(repository)
    # The candidate walk already statted every mirrored skill file, so the
    # skill-tree fingerprint is derived from it instead of a second tree walk.
    skill_stats = [
        (relative.split("/", 1)[0], relative.split("/skills/", 1)[1], mtime_ns, size)
        for _, kind, _, relative, (mtime_ns, size) in candidates
        if kind == "skill"
    ]
    fingerprints = index_fingerprints(repository, skill_stats)
    if incremental:
        reusable, fingerprints = reusable_source_state(
            connection, repository, fingerprints
        )
        documents, retained, state = discover_documents(
            repository, reusable, candidates=candidates
        )
        scan_seconds += time.monotonic() - phase_started
        index_started = time.monotonic()
        try:
            result = index_documents(
                connection,
                repository,
                documents,
                retained=retained,
                source_state=state,
                fingerprints=fingerprints,
            )
        except RetrievalError:
            # The cache disagreed with the index; fall through and rebuild.
            phase_started = time.monotonic()
        else:
            result["phase_seconds"] = {
                "stat": round(scan_seconds, 6),
                "index": round(time.monotonic() - index_started, 6),
            }
            return result
    documents, _, state = discover_documents(repository, candidates=candidates)
    scan_seconds += time.monotonic() - phase_started
    index_started = time.monotonic()
    result = index_documents(
        connection, repository, documents, source_state=state,
        fingerprints=fingerprints,
    )
    result["phase_seconds"] = {
        "stat": round(scan_seconds, 6),
        "index": round(time.monotonic() - index_started, 6),
    }
    return result


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


def distill_capsule_query(connection: sqlite3.Connection, prompt: str) -> str:
    """Reduce a raw prompt to its most informative retrieval terms.

    The read hook used to keep the first 24 words of the prompt, so a long
    request whose actual subject arrives at the end retrieved on its preamble.
    Instead the whole prompt is tokenized, terms the index has never seen or
    that match most of the corpus are dropped — the same adaptive stop-word
    rule retrieval itself applies — and the rarest terms win the slots. Rarity
    in this repository's own index decides relevance, not position.

    The fallback ladder mirrors ``informative_tokens``: rare terms first, any
    indexed term next, and the prompt head as a last resort, so a prompt made
    only of unknown words still produces the query it always did. Selected
    terms keep their prompt order — FTS ignores order, a human reading the
    manifest should not have to.
    """
    tokens: list[str] = []
    seen: set[str] = set()
    for token in re.findall(r"\w+", prompt, flags=re.UNICODE):
        folded = token.casefold()
        if folded in seen:
            continue
        seen.add(folded)
        tokens.append(token)
    if not tokens:
        # Empty and wordless prompts keep the canonical downstream error
        # ("Search query must contain a word") instead of inventing one here.
        return prompt
    try:
        total = connection.execute("SELECT COUNT(*) FROM documents").fetchone()[0]
    except sqlite3.Error:
        total = 0
    if not total:
        return " ".join(tokens[:CAPSULE_PROMPT_TERM_LIMIT])
    frequencies = token_document_frequencies(connection, tokens)
    ranked = [
        (position, token, frequencies.get(token))
        for position, token in enumerate(tokens)
    ]
    matched = [item for item in ranked if item[2]]
    informative = [
        item
        for item in matched
        if item[2] <= total * STOPWORD_DOCUMENT_RATIO  # type: ignore[operator]
    ]
    pool = informative or matched
    if not pool:
        return " ".join(tokens[:CAPSULE_PROMPT_TERM_LIMIT])
    rarest = sorted(pool, key=lambda item: (item[2], item[0]))
    kept = sorted(rarest[:CAPSULE_PROMPT_TERM_LIMIT])
    return " ".join(token for _, token, _ in kept)


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


def validate_direct_query_request(arguments: argparse.Namespace) -> None:
    """Reject unsafe direct queries before opening or mutating local state."""
    if arguments.command in {"context", "retrieve"}:
        query = arguments.query
    elif arguments.command == "refresh":
        query = arguments.query
    else:
        return
    if query is None:
        return
    reject_secrets("Task Capsule", [query])
    reject_capsule_privacy("Task Capsule request", [query])


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
    # Absent on records written before the field existed: an old row simply
    # has no checkpoint, which renders exactly as it always did.
    checkpoint = working.get("auto_checkpoint") or ""
    next_steps = working.get("next_steps")
    files = working.get("files")
    sources = working.get("sources")
    if (
        not isinstance(task_id, str)
        or not isinstance(goal, str)
        or not isinstance(progress, str)
        or not isinstance(checkpoint, str)
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

    values = [task_id, goal, progress, checkpoint, *next_steps, *files, *sources]
    reject_secrets("Task Capsule", values)
    reject_capsule_privacy(
        "Working task",
        [goal, progress, checkpoint, *next_steps],
        [task_id, *files, *sources],
    )
    # Manual progress leads and the checkpoint follows as a labelled
    # supplement, so the capsule keeps its shape while showing both.
    displayed = render_current_state(progress, checkpoint)

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
        "progress": " ".join(displayed.split()),
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
        "last_turn_characters",
    ):
        omitted.setdefault(key, 0)
    working = compacted["working"]

    while capsule_character_count(compacted) > CAPSULE_CHARACTER_LIMIT:
        # The previous turn's report is derived telemetry the next turn will
        # rewrite; every other section is either mandatory working state or
        # retrieved knowledge, so this one yields first.
        if compacted.get("last_turn"):
            omitted["last_turn_characters"] += len(compacted["last_turn"])
            compacted["last_turn"] = None
            continue
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


def enforce_governed_capsule_contract(
    capsule: dict[str, object],
) -> dict[str, object]:
    """Apply the shared 2/3/1 and 8,000-character contract to governed output."""
    compacted = json.loads(serialize_capsule(capsule))
    compacted["procedural"] = compacted.get("procedural", [])[
        :CAPSULE_LAYER_LIMITS["procedural"]
    ]
    compacted["semantic"] = compacted.get("semantic", [])[
        :CAPSULE_LAYER_LIMITS["semantic"]
    ]
    compacted["episodic"] = compacted.get("episodic", [])[
        :CAPSULE_LAYER_LIMITS["episodic"]
    ]
    compacted["omitted"] = {
        "procedural": max(
            0,
            len(capsule.get("procedural", []))
            - CAPSULE_LAYER_LIMITS["procedural"],
        ),
        "semantic": max(
            0,
            len(capsule.get("semantic", [])) - CAPSULE_LAYER_LIMITS["semantic"],
        ),
        "episodic": max(
            0,
            len(capsule.get("episodic", [])) - CAPSULE_LAYER_LIMITS["episodic"],
        ),
    }

    working = compacted.get("working")
    if isinstance(working, dict):
        working["next_steps"] = list(working.get("next_steps", []))[-1:]
        working["files"] = list(working.get("files", []))[
            :CAPSULE_WORKING_FILE_LIMIT
        ]
        working["sources"] = list(working.get("sources", []))[
            :CAPSULE_WORKING_SOURCE_LIMIT
        ]

    def synchronize_views() -> None:
        selected_paths = {
            item["path"]
            for layer in ("procedural", "semantic")
            for item in compacted[layer]
            if isinstance(item, dict) and isinstance(item.get("path"), str)
        }
        compacted["selected"] = [
            item
            for item in compacted.get("selected", [])
            if isinstance(item, dict) and item.get("path") in selected_paths
        ]
        categories = compacted.get("categories")
        if isinstance(categories, dict):
            for category, items in categories.items():
                categories[category] = [
                    item
                    for item in items
                    if isinstance(item, dict) and item.get("path") in selected_paths
                ]

    # Snippets are discovery aids. Bounding each rendered copy leaves the full
    # source and its hash in the auditable manifest while avoiding duplicated
    # aliases consuming the entire capsule.
    for collection in (
        compacted["procedural"],
        compacted["semantic"],
        compacted["episodic"],
        compacted.get("selected", []),
        *(
            compacted.get("categories", {}).values()
            if isinstance(compacted.get("categories"), dict)
            else []
        ),
    ):
        for item in collection:
            if isinstance(item, dict) and isinstance(item.get("snippet"), str):
                item["snippet"] = item["snippet"][:320]
    synchronize_views()

    while capsule_character_count(compacted) > CAPSULE_CHARACTER_LIMIT:
        if compacted.get("last_turn"):
            compacted["last_turn"] = None
            continue
        dropped = False
        for layer in ("episodic", "semantic", "procedural"):
            if compacted[layer]:
                compacted[layer].pop()
                compacted["omitted"][layer] += 1
                synchronize_views()
                dropped = True
                break
        if dropped:
            continue
        if isinstance(working, dict) and working.get("progress"):
            excess = capsule_character_count(compacted) - CAPSULE_CHARACTER_LIMIT
            keep = max(0, len(working["progress"]) - excess - 1)
            if keep:
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


# The mandatory sections of a delegation capsule - the payload an orchestrating
# flow hands each spawned agent. Matching is a case-insensitive substring so
# headings, bold list items, and plain labels all satisfy the contract.
CAPSULE_SECTIONS = (
    "objective",
    "output format",
    "tool and source guidance",
    "boundaries",
    "decisions and assumptions",
)


def validate_capsule_text(content: str) -> list[str]:
    """Return every problem that makes a delegation capsule under-specified."""
    if not content.strip():
        return ["capsule is empty"]
    problems: list[str] = []
    if len(content) > MESSAGE_BODY_LIMIT:
        problems.append(
            f"capsule exceeds {MESSAGE_BODY_LIMIT} characters ({len(content)})"
        )
    lowered = content.lower()
    for section in CAPSULE_SECTIONS:
        if section not in lowered:
            problems.append(f"missing mandatory section: {section}")
    try:
        reject_secrets("delegation capsule", [content])
    except ContextError as error:
        problems.append(str(error))
    return problems


def read_capsule_source(value: str) -> str:
    if value == "-":
        return sys.stdin.read()
    path = Path(value)
    if not path.is_file():
        raise ContextError(f"Capsule file not found: {value}")
    return path.read_text(encoding="utf-8")


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
        SELECT task_id, goal, progress, auto_checkpoint, next_steps, files, sources,
               created_at, updated_at
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
        "auto_checkpoint": row["auto_checkpoint"],
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
    *,
    auto_checkpoint: Optional[str] = None,
) -> dict[str, object]:
    """Merge an update into the lightweight working task.

    ``progress`` is the operator's narrative; ``auto_checkpoint`` is the
    automated turn flush's own field, replaced wholesale on every flush. They
    are separate parameters so automation can checkpoint without ever
    overwriting what the operator wrote.
    """
    task_id = validate_task_id(task_id)
    next_steps = normalize_values("Working task next step", next_steps)
    files = validate_paths("Working task file", files)
    sources = normalize_values("Working task source", sources)
    if progress is None and auto_checkpoint is None and not (
        next_steps or files or sources
    ):
        raise ContextError("Working task update requires a changed field")
    if progress is not None:
        progress = progress.strip()
        if not progress:
            raise ContextError("Working task progress must not be empty")
    if auto_checkpoint is not None:
        auto_checkpoint = auto_checkpoint.strip()
        if not auto_checkpoint:
            raise ContextError("Working task auto checkpoint must not be empty")
    prose = [
        value for value in (progress, auto_checkpoint) if value is not None
    ]
    reject_secrets(
        "working task",
        [task_id] + prose + next_steps + files + sources,
    )
    reject_capsule_privacy(
        "Working task",
        prose + next_steps,
        [task_id, *files, *sources],
    )
    try:
        connection.execute("BEGIN IMMEDIATE")
        row = connection.execute(
            "SELECT progress, auto_checkpoint, next_steps, files, sources "
            "FROM working_tasks WHERE task_id = ?",
            (task_id,),
        ).fetchone()
        if row is None:
            raise ContextError(f"Working task not found: {task_id}")
        connection.execute(
            """
            UPDATE working_tasks
            SET progress = ?, auto_checkpoint = ?, next_steps = ?, files = ?,
                sources = ?, updated_at = ?
            WHERE task_id = ?
            """,
            (
                row["progress"] if progress is None else progress,
                (
                    row["auto_checkpoint"]
                    if auto_checkpoint is None
                    else auto_checkpoint
                ),
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
        "auto_checkpoint": record.get("auto_checkpoint"),
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


def last_turn_report_path(repository: Path) -> Path:
    return repository / "memory-bank" / "local" / LAST_TURN_REPORT_FILENAME


def write_last_turn_report(repository: Path, report: dict[str, object]) -> None:
    """Persist the turn outcome where the next request's capsule can read it.

    The Stop hook discards this command's stdout, so a blocked promotion or a
    failed compaction printed there is never seen by anyone. The machine
    summary lands in ignored local state instead, and the next refresh folds
    it into the Task Capsule. A write failure degrades silently: continuity
    telemetry must never turn a checkpoint that already landed into a failed
    turn, and a repository whose ``local`` directory cannot be created simply
    goes without the report.
    """
    try:
        atomic_json(last_turn_report_path(repository), report)
    except OSError:
        pass


def load_last_turn_report(repository: Path) -> Optional[dict[str, object]]:
    """Return the previous turn's report, or None when it cannot be trusted.

    The file is advisory local state: missing, malformed, or stale content
    must never break a refresh. Staleness is judged by the timestamp inside
    the report rather than filesystem metadata, so a copied or restored file
    does not resurrect an old turn as if it had just happened. A timestamp
    from the future is equally untrustworthy and is treated as absent.
    """
    try:
        value = json.loads(
            last_turn_report_path(repository).read_text(encoding="utf-8")
        )
    except (OSError, UnicodeDecodeError, json.JSONDecodeError):
        return None
    if not isinstance(value, dict):
        return None
    timestamp = value.get("timestamp")
    if not isinstance(timestamp, str):
        return None
    try:
        written = datetime.fromisoformat(timestamp)
    except ValueError:
        return None
    if written.tzinfo is None:
        written = written.replace(tzinfo=timezone.utc)
    age = (datetime.now(timezone.utc) - written).total_seconds()
    if age < 0 or age > LAST_TURN_REPORT_MAX_AGE_SECONDS:
        return None
    return value


def summarize_last_turn(report: dict[str, object]) -> Optional[str]:
    """Render the previous turn's report as one compact capsule line.

    Only non-zero and problem items appear: a turn that merely buffered a
    delta produces no section at all, so the capsule does not narrate routine
    bookkeeping. Problems lead the line — a blocked promotion is the fact this
    section exists to surface — so the tail truncation can only ever cut
    routine counters, never a blocked-promotion reason. The report is local
    state a human may have edited, so every field is read defensively.
    """

    def entries(key: str) -> list[dict[str, object]]:
        value = report.get(key)
        if not isinstance(value, list):
            return []
        return [item for item in value if isinstance(item, dict)]

    def strings(key: str) -> list[str]:
        value = report.get(key)
        if not isinstance(value, list):
            return []
        return [item for item in value if isinstance(item, str) and item]

    def count(key: str) -> int:
        value = report.get(key)
        return value if isinstance(value, int) and value > 0 else 0

    parts: list[str] = []
    # A turn that failed outright leads everything else: its report carries
    # the error instead of counters, and without this line a broken flush
    # loop (for example an unresolvable binding) would stay invisible forever.
    error = report.get("error")
    if isinstance(error, str) and error:
        parts.append(f"turn failed: {error}")
    for label, key in (
        ("promotion blocked", "promotion_blocked"),
        ("promotion failed", "promotion_failed"),
    ):
        reasons = dict.fromkeys(
            str(item["reason"]) for item in entries(key) if item.get("reason")
        )
        parts.extend(f"{label}: {reason}" for reason in reasons)
    parts.extend(f"compaction failed: {error}" for error in strings("compaction_errors"))
    excluded = strings("excluded_paths")
    if excluded:
        shown = excluded[:LAST_TURN_SECTION_PATH_LIMIT]
        more = len(excluded) - len(shown)
        parts.append(
            "excluded paths: "
            + ", ".join(shown)
            + (f" (+{more} more)" if more else "")
        )
    completion_candidates = strings("completion_candidates")
    if completion_candidates:
        parts.append(
            "completion candidate (explicit complete required): "
            + ", ".join(completion_candidates)
        )
    promoted = entries("promoted")
    if promoted:
        parts.append(f"promoted: {len(promoted)} record(s)")
    if count("promotion_skipped"):
        parts.append(
            f"promotion deferred: {count('promotion_skipped')} record(s)"
        )
    if count("archived"):
        parts.append(f"archived: {count('archived')} record(s)")
    if report.get("flushed"):
        flushed = "buffer flushed"
        if count("files"):
            flushed += f": {count('files')} file(s) this turn"
        if count("files_omitted"):
            flushed += f" ({count('files_omitted')} beyond the per-flush limit)"
        if report.get("provisioned"):
            flushed += ", task auto-provisioned"
        if report.get("rebound"):
            # A second machine reconnected to the existing governed record
            # instead of failing on a duplicate; worth one clause because it
            # confirms the work landed in the task Git already knows.
            flushed += ", binding restored to the existing task"
        parts.append(flushed)
    if not parts:
        return None
    summary = "; ".join(parts)
    if len(summary) > LAST_TURN_SECTION_CHARACTER_LIMIT:
        summary = summary[: LAST_TURN_SECTION_CHARACTER_LIMIT - 1] + "…"
    return summary


def last_turn_section(repository: Path) -> Optional[str]:
    """The 'Last turn' capsule section, or None when there is nothing to say."""
    report = load_last_turn_report(repository)
    if report is None:
        return None
    return summarize_last_turn(report)


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
    # What the silenced Stop hook did last turn, folded into this request's
    # capsule. Rendered once here so both modes report it identically; the
    # summary itself is bounded, and the lightweight budget below may still
    # drop it first.
    last_turn = last_turn_section(repository)
    if mode == "lightweight":
        result = build_context_packet(
            connection,
            query,
            task_id,
            limit,
            include_retrieval=not warnings,
            warnings=warnings,
        )
        result["last_turn"] = last_turn
        return enforce_capsule_budget(result)
    if task_id is None:
        raise ContextError("Governed retrieval requires --task-id")
    # The governed path bypasses build_context_packet, so apply the same query
    # guards build_context_packet would have applied.
    reject_secrets("Task Capsule", [query])
    reject_capsule_privacy("Task Capsule request", [query])
    binding = governed_binding(connection, task_id)
    request_query = build_capsule_query(query, None)
    result = retrieve(
        connection,
        repository,
        request_query,
        binding["task_uuid"],
        limit=limit,
        manifest_scope="local" if ephemeral else "governed",
    )
    result["episodic"] = (
        search_documents(
            connection,
            request_query,
            CAPSULE_LAYER_LIMITS["episodic"],
            "episodic",
            relevant_only=True,
        )
        + search_episodes(
            connection, request_query, CAPSULE_LAYER_LIMITS["episodic"]
        )
    )[:CAPSULE_LAYER_LIMITS["episodic"]]
    # The print tail dereferences result["warnings"]; retrieve() has no such key.
    result["warnings"] = warnings
    result["last_turn"] = last_turn
    return enforce_governed_capsule_contract(result)


def assemble_hook_context(
    connection: sqlite3.Connection,
    repository: Path,
    *,
    mode: str,
    task_id: str,
) -> Optional[dict[str, object]]:
    """Return a governed capsule or a sanitized pre-provision warming capsule."""
    task_id = validate_task_id(task_id)
    reject_capsule_privacy("Task Capsule request", [], [task_id])
    if mode == "lightweight":
        if find_working_task(connection, task_id) is not None:
            return assemble_capsule(
                connection,
                repository,
                mode=mode,
                query=task_id,
                task_id=task_id,
                limit=3,
                ephemeral=True,
            )
    else:
        try:
            governed_binding(connection, task_id)
        except ContextError:
            pass
        else:
            return assemble_capsule(
                connection,
                repository,
                mode=mode,
                query=task_id,
                task_id=task_id,
                limit=3,
                ephemeral=True,
            )

    files, excluded = changed_paths(repository)
    if not files:
        return None
    pending = len(pending_turn_deltas(connection, task_id))
    return {
        "kind": "warming",
        "task_id": task_id,
        "working": None,
        "procedural": [],
        "semantic": [],
        "episodic": [],
        "changed_files": len(files),
        "excluded_files": len(excluded),
        "pending_turns": pending,
        "warnings": [
            "Governed task not provisioned yet; authoritative persistence "
            f"starts at the {DEFAULT_TURN_FLUSH_AFTER}-turn boundary."
        ],
    }


def print_capsule(capsule: dict[str, object]) -> None:
    # The first line is the render marker: the Cursor hooks accept a capsule
    # only if it starts with "working:", which is how a broken render is kept
    # from replacing a good rule now that they no longer parse JSON.
    working = capsule["working"]
    if working is None:
        print("working: unavailable")
    else:
        print(f"working: {working['task_id']} — {working['goal']}")
    if capsule.get("kind") == "warming":
        # Pre-provision progress is the one field the serialized form carried
        # that the warning text does not: how far along the boundary is.
        print(f"warming: {capsule['pending_turns']} turn(s) pending")
    for warning in capsule["warnings"]:
        print(f"warning: {warning}")
    if capsule.get("last_turn"):
        print(f"Last turn: {capsule['last_turn']}")
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
    auto_checkpoint: Optional[str] = None,
    allow_phase_regression: bool = False,
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
                auto_checkpoint=auto_checkpoint,
                allow_phase_regression=allow_phase_regression,
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


def rebind_governed_task(
    connection: sqlite3.Connection,
    repository: Path,
    task_id: str,
    record_id: Optional[str] = None,
) -> dict[str, object]:
    """Restore the machine-local binding for an existing governed task.

    The Brain record is authoritative and travels through Git; the binding
    that resolves compatibility commands lives in the ignored ``context.db``
    and does not. A second machine or a fresh clone therefore holds the task
    but cannot address it, and recreating it is rejected as a duplicate.
    Rebinding is a pointer repair, not a mutation: the record is never
    touched, so no revision is consumed and no owner check applies.

    ``record_id`` is an optional explicit target; the record's own
    ``external_id`` must equal ``task_id`` either way, so the command can only
    reconnect a task to its own identity, never repoint one task at another.
    An intact identical binding is reported rather than recreated; a binding
    that points elsewhere is refused, because silently overwriting local state
    an operator may be relying on is exactly the failure this repairs.
    """
    task_id = validate_task_id(task_id)
    with mutation_lock(repository):
        try:
            record = get_record(repository, record_id or task_id)
        except BrainError as error:
            raise ContextError(str(error)) from error
        if record.get("type") != "task":
            raise ContextError(
                f"Rebind requires a task record; {record['id']} is a "
                f"{record.get('type')} record"
            )
        if record.get("external_id") != task_id:
            raise ContextError(
                f"Rebind target mismatch: record {record['id']} belongs to "
                f"task id {record.get('external_id')!r}, not {task_id!r}"
            )
        if record.get("status") in TERMINAL_STATES:
            raise ContextError(
                f"Cannot rebind {task_id}: task {record['id']} is "
                f"{record['status']}; start new work under a new task id"
            )
        existing = connection.execute(
            "SELECT task_uuid FROM task_bindings WHERE external_id = ?",
            (task_id,),
        ).fetchone()
        if existing is not None:
            if existing["task_uuid"] == record["id"]:
                return {**governed_task_view(record), "already_bound": True}
            raise ContextError(
                f"Working task {task_id} is already bound to "
                f"{existing['task_uuid']}; refusing to repoint an existing "
                "binding"
            )
        bind_governed_task(
            connection, task_id, str(record["id"]), int(record["revision"])
        )
    return {**governed_task_view(record), "already_bound": False}


def ensure_working_task(
    connection: sqlite3.Connection,
    repository: Path,
    task_id: str,
    mode: str,
    owner: str,
) -> dict[str, object]:
    """Create or reconnect the working task when a turn has something to record.

    Automated continuity is worthless if it buffers into a task nobody created.
    Provisioning happens at flush time rather than at session start, so merely
    visiting a branch does not mint a record; only accumulated real work does.

    Returns ``{"provisioned": bool, "rebound": Optional[str]}``: whether a task
    was created, and — when the governed record already existed but the local
    binding did not (a second machine or fresh clone) — the UUID of the record
    the binding was restored to. Without that restoration the flush would fail
    on a duplicate-record error every turn, invisibly, and the second machine's
    work would never reach the authoritative task.
    """
    goal = derive_goal(task_id)
    reject_secrets("working task", [task_id, goal])
    reject_capsule_privacy("Working task", [goal], [task_id])
    if mode == "lightweight":
        if find_working_task(connection, task_id) is not None:
            return {"provisioned": False, "rebound": None}
        start_working_task(connection, task_id, goal, [], [])
        return {"provisioned": True, "rebound": None}
    try:
        governed_binding(connection, task_id)
        return {"provisioned": False, "rebound": None}
    except ContextError:
        pass
    try:
        task = rebind_governed_task(connection, repository, task_id)
    except ContextError as error:
        # Only a genuinely absent record means "first flush on this branch";
        # every other rebind refusal (terminal task, ambiguous identifier)
        # would also fail creation and is clearer stated as what it is.
        if "not found" not in str(error):
            raise
        create_governed_task(connection, repository, task_id, goal, [], [], owner)
        return {"provisioned": True, "rebound": None}
    return {"provisioned": False, "rebound": str(task["task_uuid"])}


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


DEFAULT_BRANCH_STATE_KEY = "default_branch"


def cached_default_branch(
    connection: sqlite3.Connection, repository: Path
) -> Optional[str]:
    """Resolve the merge target once and reuse it from the local store.

    Detection costs up to three git processes and its answer changes about as
    often as a repository renames its default branch, so the result is kept in
    `index_state` alongside the other index fingerprints. The cached value is
    still verified on every use — one probe instead of a full detect — because
    a branch that was deleted or renamed must not keep judging merges against
    a ref that no longer exists; a failed probe falls back to detection.

    `CONTEXT_DEFAULT_BRANCH` bypasses the cache entirely: an explicit override
    is authoritative for exactly as long as it is set, and caching it would
    let a removed override keep governing later runs.
    """
    configured = os.environ.get("CONTEXT_DEFAULT_BRANCH")
    if configured:
        return configured.strip() or None
    cached = load_index_state(connection).get(DEFAULT_BRANCH_STATE_KEY)
    if cached:
        # Detection only ever yields a local head ("main") or a
        # remote-tracking head ("origin/main"); the probes mirror that.
        for reference in (f"refs/heads/{cached}", f"refs/remotes/{cached}"):
            try:
                git_output(
                    repository,
                    ["rev-parse", "--verify", "--quiet", reference],
                    "Git branch probe",
                )
            except ContextError:
                continue
            return cached
    detected = default_branch(repository)
    if detected is not None:
        with connection:
            store_index_state(connection, {DEFAULT_BRANCH_STATE_KEY: detected})
    elif cached:
        # Nothing detected: drop the stale entry rather than probe a ref that
        # is known to be gone on every later turn.
        with connection:
            connection.execute(
                "DELETE FROM index_state WHERE key = ?",
                (DEFAULT_BRANCH_STATE_KEY,),
            )
    return detected


def repository_references(repository: Path) -> dict[str, str]:
    """Map every ref to the commit it points at, in one git process.

    Annotated tags are peeled to the commit they tag, so comparing these
    values against a branch tip compares commits with commits — the same
    objects `merge-base` and `rev-list` operated on when every branch was
    probed individually.
    """
    listing = os.fsdecode(
        git_output(
            repository,
            ["for-each-ref", "--format=%(objectname) %(*objectname) %(refname)"],
            "Git reference listing",
        )
    )
    references: dict[str, str] = {}
    for line in listing.splitlines():
        try:
            pointed, peeled, refname = line.split(" ", 2)
        except ValueError:
            continue
        references[refname] = peeled or pointed
    return references


def merged_reference_names(repository: Path, target: str) -> set[str]:
    """Name every ref that is an ancestor of `target`, in one git process."""
    listing = os.fsdecode(
        git_output(
            repository,
            ["for-each-ref", "--format=%(refname)", f"--merged={target}"],
            "Git merge probe",
        )
    )
    return {line for line in listing.splitlines() if line}


def resolve_task_reference(
    branch: str, references: dict[str, str]
) -> Optional[str]:
    """Pick the ref a task's branch name denotes, or None when it is gone.

    The order mirrors the per-branch probe this scan replaced: a local head
    wins, then the bare name falls through the gitrevisions disambiguation
    steps that `rev-parse` applied. A name that resolves to nothing is a
    deleted branch, which cannot be told apart from an abandoned one and is
    therefore never treated as merged.
    """
    for candidate in (
        f"refs/heads/{branch}",
        f"refs/{branch}",
        f"refs/tags/{branch}",
        f"refs/remotes/{branch}",
        f"refs/remotes/{branch}/HEAD",
    ):
        if candidate in references:
            return candidate
    return None


def merge_completion_candidates(
    connection: sqlite3.Connection, repository: Path
) -> list[dict[str, object]]:
    """Report active tasks whose branch has landed in the default branch.

    Merge evidence is advisory only. Completion remains an explicit,
    revision-checked user action after verification; this probe never mutates
    a task, handoff, episode, or binding.

    A branch counts as merged only when it is an ancestor of the target *and*
    the target has moved ahead of it, which is what a merge does. Ancestry
    alone is not enough: a branch created a moment ago and never committed to
    is already an ancestor of its target, so requiring only that would
    complete a task the instant it was provisioned. Merging the target *into*
    a branch does not make the branch an ancestor, so a long-running branch
    that merely keeps itself current stays open. A fast-forward merge leaves
    the two refs identical and is therefore indistinguishable from a branch
    that never diverged; it is not detected. Leaving a task open costs a
    stale record, while closing one wrongly ends its lifecycle and archives
    it.

    The whole scan costs a fixed number of git processes — one reference
    listing, one reachability listing, one target probe — instead of up to
    four processes per active task.
    """
    candidates = [
        record
        for _, record, _ in iter_records(repository)
        if record["type"] == "task"
        and record["status"] not in LIFECYCLES["task"]["terminal"]
    ]
    if not candidates:
        return []
    target = cached_default_branch(connection, repository)
    if target is None:
        return []
    try:
        target_commit = os.fsdecode(
            git_output(
                repository,
                ["rev-parse", "--verify", "--quiet", f"{target}^{{commit}}"],
                "Git merge probe",
            )
        ).strip()
    except ContextError:
        # A target this clone cannot resolve has merged nothing.
        return []
    references = repository_references(repository)
    reachable = merged_reference_names(repository, target)
    completion_candidates: list[dict[str, object]] = []
    for record in candidates:
        branch = str(record["external_id"])
        # A branch is its own ancestor, so the default branch would close
        # itself the moment a task were provisioned on it.
        if branch == target or branch == target.split("/")[-1]:
            continue
        reference = resolve_task_reference(branch, references)
        if reference is None or reference not in reachable:
            continue
        # An ancestor whose tip equals the target's means the target never
        # moved ahead: the never-diverged and fast-forward cases.
        if references[reference] == target_commit:
            continue
        completion_candidates.append(
            {
                "task_id": branch,
                "task_uuid": record["id"],
                "revision": record["revision"],
                "target": target,
                "evidence": "branch is an ancestor of the merge target",
            }
        )
    return completion_candidates


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

    The consolidated summary lands in the task's ``auto_checkpoint`` field and
    nowhere else: ``progress`` belongs to the operator, and an automated writer
    that replaced it would erase the one narrative a handoff cannot recover.
    """
    pending = pending_turn_deltas(connection, task_id)
    if not pending:
        return None
    files: list[str] = []
    for row in pending:
        files = merge_unique(files, json.loads(row["files"]))
    omitted = max(0, len(files) - file_limit)
    checkpoint = (
        f"Auto-checkpoint: {len(pending)} turn(s) since {pending[0]['created_at']}, "
        f"{len(files)} file(s) touched."
    )
    if omitted:
        checkpoint += f" {omitted} path(s) beyond the per-flush limit are not listed."
    files = files[:file_limit]
    ensured = ensure_working_task(connection, repository, task_id, mode, owner)
    if mode == "lightweight":
        result = update_working_task(
            connection, task_id, None, [], files, [],
            auto_checkpoint=checkpoint,
        )
    else:
        result = apply_governed_update(
            connection,
            repository,
            governed_binding(connection, task_id),
            expected_revision=AUTO_REVISION,
            progress=None,
            next_steps=[],
            files=files,
            sources=[],
            owner=owner,
            auto_checkpoint=checkpoint,
        )
    with connection:
        connection.executemany(
            "DELETE FROM turn_deltas WHERE id = ?",
            [(row["id"],) for row in pending],
        )
    return {**result, "files_omitted": omitted, **ensured}


def run_turn(
    connection: sqlite3.Connection,
    repository: Path,
    arguments: argparse.Namespace,
    mode: str,
    owner: str,
) -> int:
    """The `turn` command body: buffer, flush on the boundary, then report."""
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
    # Maintenance runs only on the boundary where a flush actually
    # happened. The Stop hook drives this command under a hard
    # timeout on every turn, so an ordinary turn must stay at one
    # git status probe plus a local buffer write — a SIGTERM there
    # interrupts nothing mid-mutation.
    completion_candidates: list[dict[str, object]] = []
    promotion = {
        "enabled": False, "promoted": [], "failed": [],
        "blocked": [], "skipped": 0,
    }
    compaction = {"enabled": False, "moved": 0, "pending": 0, "error": None}
    if flushed is not None and mode != "lightweight":
        # Merge evidence is advisory. It may suggest an explicit completion,
        # but this unattended path never changes task lifecycle state.
        try:
            completion_candidates = merge_completion_candidates(
                connection, repository
            )
        except (BrainError, OSError) as error:
            raise ContextError(
                f"Merge completion-candidate detection failed: {error}"
            ) from error
        # Durable memory is promoted on the same boundary as the
        # working-memory flush, so the whole pipeline runs
        # unattended.
        try:
            promotion = auto_promote(repository, owner=owner)
        except (BrainError, OSError) as error:
            raise ContextError(
                f"Automatic promotion failed: {error}"
            ) from error
        # Archiving runs last: a record must be promoted before it
        # is moved, and compaction batches so Git history is not
        # churned one terminal record at a time.
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
        # The UUID of the pre-existing record the flush restored the local
        # binding to — the second-machine signal. None on an ordinary flush.
        "rebound": flushed.get("rebound") if flushed else None,
        "completion_candidates": completion_candidates,
        "promoted": promotion["promoted"],
        "promotion_failed": promotion["failed"],
        "promotion_blocked": promotion["blocked"],
        "promotion_skipped": promotion["skipped"],
        "archived": compaction["moved"],
        "archivable_pending": compaction["pending"],
    }
    # The Stop hook silences this command's stdout, so the same
    # outcome is written to ignored local state for the next
    # refresh to fold into its Task Capsule.
    write_last_turn_report(
        repository,
        {
            "schema_version": 1,
            "timestamp": datetime.now(timezone.utc).isoformat(),
            "task_id": task_id,
            "flushed": result["flushed"],
            "files": result["files"],
            "files_omitted": result["files_omitted"],
            "pending": result["pending"],
            "provisioned": result["provisioned"],
            "rebound": result["rebound"],
            "completion_candidates": [
                str(item["task_id"]) for item in completion_candidates
            ],
            "promoted": promotion["promoted"],
            "promotion_failed": promotion["failed"],
            "promotion_blocked": promotion["blocked"],
            "promotion_skipped": promotion["skipped"],
            "excluded_paths": path_excluded,
            "archived": compaction["moved"],
            "compaction_errors": (
                [compaction["error"]] if compaction["error"] else []
            ),
        },
    )
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
        if result["rebound"]:
            print(f"binding restored to existing task: {result['rebound']}")
        for item in completion_candidates:
            print(
                "completion candidate (explicit complete required): "
                f"{item['task_id']} at revision {item['revision']}"
            )
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


EXPORT_SCHEMA_VERSION = 1
EXPORTABLE_CHUNK_STATUSES = ("active", "needs-review")

EXPORT_README = """# Context Bundle

A point-in-time, privacy-filtered copy of one installation's Project Brain
records and Memory Bank chunks. `MANIFEST.json` is the index: it lists every
item included, and every item excluded together with the reason.

This is a handoff artifact, not a second source of truth.

- **Records here are not authoritative.** Each one cites the sources it was
  derived from. Open the cited source before acting on a claim.
- **Chunks tagged `auto-promoted` were never read by a human.** Automatic
  promotion is the shipped default; see the origin repository's
  `docs/SECURITY.md`.
- **You are not an authorized owner.** Every Project Brain record carries the
  originating owner in `owner`/`authorized_owners`. A record copied into your
  installation cannot be mutated by you until it is re-created under your own
  identity. Treat this bundle as read-only history and start your own records
  for active work.
- **Revisions are frozen.** `revision` reflects the source installation at
  export time. It says nothing about what happened there afterwards.
"""


def _export_relative_path(repository: Path, path: Path) -> str:
    try:
        return path.relative_to(repository).as_posix()
    except ValueError:
        return path.name


def export_bundle(
    repository: Path,
    destination: Path,
    *,
    include_archive: bool = False,
    include_superseded: bool = False,
    force: bool = False,
) -> dict:
    """Write a privacy-filtered bundle of Brain records and Memory Bank chunks.

    Export moves content out of the installation that produced it, so it is
    fail-closed on both ends: a destination that already holds files is refused
    without ``force``, and a single secret match anywhere in the selected set
    aborts the whole bundle rather than writing a partial one. The manifest
    records every exclusion with its reason - a bundle that silently dropped
    records would read as "this is everything".
    """
    if destination.exists() and any(destination.iterdir()) and not force:
        raise ContextError(
            f"Export destination is not empty: {destination}. "
            "Use --force to overwrite its contents."
        )

    config = load_config(repository)
    allowed_privacy = set(config.get("allowed_privacy", ("public", "team")))

    included: list[dict] = []
    excluded: list[dict] = []
    payload: list[tuple[Path, str]] = []

    for path, record, body in iter_records(repository, include_archive=include_archive):
        relative = _export_relative_path(repository, path)
        privacy = record.get("privacy")
        if privacy not in allowed_privacy:
            excluded.append(
                {
                    "path": relative,
                    "id": record.get("id"),
                    "reason": f"privacy '{privacy}' is not in allowed_privacy",
                }
            )
            continue
        included.append(
            {
                "kind": "brain-record",
                "path": relative,
                "id": record.get("id"),
                "external_id": record.get("external_id"),
                "type": record.get("type"),
                "status": record.get("status"),
                "privacy": privacy,
                "authority": record.get("authority"),
                "owner": record.get("owner"),
                "revision": record.get("revision"),
                "sources": record.get("sources", []),
            }
        )
        payload.append((path, relative))

    chunks_root = repository / "memory-bank" / "chunks"
    if chunks_root.is_dir():
        for path in sorted(chunks_root.glob("*.md")):
            if path.is_symlink():
                continue
            relative = _export_relative_path(repository, path)
            try:
                metadata = parse_frontmatter(path)
            except ValidationError as error:
                excluded.append(
                    {"path": relative, "id": None, "reason": f"unreadable chunk: {error}"}
                )
                continue
            status = metadata.get("status")
            if not include_superseded and status not in EXPORTABLE_CHUNK_STATUSES:
                excluded.append(
                    {
                        "path": relative,
                        "id": metadata.get("id"),
                        "reason": f"status '{status}' is not exported without --include-superseded",
                    }
                )
                continue
            included.append(
                {
                    "kind": "memory-chunk",
                    "path": relative,
                    "id": metadata.get("id"),
                    "title": metadata.get("title"),
                    "type": metadata.get("type"),
                    "status": status,
                    "tags": metadata.get("tags", []),
                    "last_verified": metadata.get("last_verified"),
                    "sources": metadata.get("sources", []),
                    "auto_promoted": "auto-promoted" in (metadata.get("tags") or []),
                }
            )
            payload.append((path, relative))

    # Fail closed before anything is written. A bundle leaves the machine, so a
    # single match aborts the export; the offending path is named, its content
    # is not.
    for path, relative in payload:
        try:
            validate_secret_patterns(path)
        except ValidationError as error:
            raise ContextError(
                f"Export aborted: {relative} matches a secret pattern ({error}). "
                "Remove the secret from the record before exporting."
            ) from error

    destination.mkdir(parents=True, exist_ok=True)
    for path, relative in payload:
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(path.read_bytes())

    manifest = {
        "schema_version": EXPORT_SCHEMA_VERSION,
        "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "source": {
            "framework": config.get("framework"),
            "mode": config.get("mode"),
            "commit": _export_source_commit(repository),
            "automatic_promotion": config.get("automatic_promotion"),
        },
        "filters": {
            "allowed_privacy": sorted(allowed_privacy),
            "include_archive": include_archive,
            "include_superseded": include_superseded,
            "exported_chunk_statuses": (
                "all" if include_superseded else list(EXPORTABLE_CHUNK_STATUSES)
            ),
        },
        "counts": {
            "brain_records": sum(1 for item in included if item["kind"] == "brain-record"),
            "memory_chunks": sum(1 for item in included if item["kind"] == "memory-chunk"),
            "excluded": len(excluded),
        },
        "included": included,
        "excluded": excluded,
    }
    (destination / "MANIFEST.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    (destination / "README.md").write_text(EXPORT_README, encoding="utf-8")

    return {
        "destination": str(destination),
        "brain_records": manifest["counts"]["brain_records"],
        "memory_chunks": manifest["counts"]["memory_chunks"],
        "excluded": len(excluded),
        "auto_promoted_chunks": sum(
            1 for item in included if item.get("auto_promoted")
        ),
    }


def _export_source_commit(repository: Path) -> Optional[str]:
    try:
        completed = subprocess.run(
            ["git", "-C", str(repository), "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError:
        return None
    if completed.returncode != 0:
        return None
    return completed.stdout.strip() or None


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

    hook_context = commands.add_parser(
        "hook-context",
        help="render a governed or pre-provision warming capsule for host hooks",
    )
    hook_context.add_argument("--task-id", required=True)
    hook_context.add_argument("--json", action="store_true")

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
        help=(
            "also assemble a Task Capsule for this request (requires "
            "--task-id); accepts the raw prompt, which is distilled to its "
            "most informative terms against the index"
        ),
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

    rebind = commands.add_parser(
        "rebind",
        help="restore the local binding for an existing governed task",
    )
    rebind.add_argument("--task-id", required=True)
    rebind.add_argument(
        "--record",
        help="task record UUID to bind; defaults to resolving --task-id",
    )
    rebind.add_argument("--json", action="store_true")

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
    update.add_argument(
        "--phase",
        choices=TASK_PHASE_INPUTS,
        help=(
            "delivery phase; aliases implementing/execution/review are stored "
            "as implementation/implementation/verification"
        ),
    )
    update.add_argument("--revision", type=revision_argument)
    update.add_argument(
        "--actor",
        help=(
            "roster agent recording this update; the slug prefixes the "
            "progress line for attribution"
        ),
    )
    update.add_argument(
        "--allow-phase-regression",
        action="store_true",
        help="permit moving the task phase backward deliberately",
    )
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

    msg_send = commands.add_parser(
        "msg-send", help="append a message to the task's agent channel"
    )
    msg_send.add_argument("--task-id", required=True)
    msg_send.add_argument(
        "--from",
        dest="from_actor",
        required=True,
        help="sending agent slug, or 'main' for the orchestrating conversation",
    )
    msg_send.add_argument(
        "--to",
        dest="to_actor",
        required=True,
        help="receiving agent slug, 'main', or '*' to broadcast",
    )
    msg_send.add_argument(
        "--type", choices=("finding", "question", "handoff"), required=True
    )
    msg_send.add_argument("--body")
    msg_send.add_argument("--body-file")
    msg_send.add_argument("--ref", action="append", default=[])
    msg_send.add_argument("--json", action="store_true")

    msg_read = commands.add_parser(
        "msg-read", help="read the task's agent channel"
    )
    msg_read.add_argument("--task-id", required=True)
    msg_read.add_argument(
        "--for",
        dest="for_actor",
        help="return only messages addressed to this actor (or broadcast)",
    )
    msg_read.add_argument(
        "--since",
        type=int,
        default=0,
        help="return only messages with seq greater than this",
    )
    msg_read.add_argument("--type", choices=MESSAGE_TYPES)
    msg_read.add_argument("--json", action="store_true")

    msg_dispatch = commands.add_parser(
        "msg-dispatch",
        help="record an orchestration dispatch or completion event",
    )
    msg_dispatch.add_argument("--task-id", required=True)
    msg_dispatch.add_argument("--agent", required=True)
    msg_dispatch.add_argument(
        "--event", choices=("spawn", "complete"), required=True
    )
    msg_dispatch.add_argument("--note")
    msg_dispatch.add_argument(
        "--capsule-file",
        help=(
            "delegation capsule to fingerprint; a spawn refuses a capsule "
            "that fails validation"
        ),
    )
    msg_dispatch.add_argument("--json", action="store_true")

    capsule = commands.add_parser(
        "capsule",
        help="validate a delegation capsule against the mandatory sections",
    )
    capsule.add_argument("--validate", action="store_true", required=True)
    capsule.add_argument(
        "--file", required=True, help="capsule path, or '-' for stdin"
    )
    capsule.add_argument("--json", action="store_true")

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
    update_brain.add_argument(
        "--phase",
        choices=TASK_PHASE_INPUTS,
        help=(
            "task delivery phase; compatibility aliases are normalized before "
            "persistence"
        ),
    )
    update_brain.add_argument("--transition")
    update_brain.add_argument(
        "--authority",
        choices=("inferred", "observed", "verified"),
        help=(
            "promote the record's evidence level; only the observed -> "
            "verified transition is accepted"
        ),
    )
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

    parity = commands.add_parser(
        "parity",
        help="fail on canonical mirror drift (skills, hooks, commands, "
        "agents, governance docs)",
    )
    parity.add_argument("--json", action="store_true")
    parity.add_argument(
        "--skills-only",
        action="store_true",
        help="light mode: compare only mirrored skills (the indexing-path check)",
    )
    parity.add_argument(
        "--cross-edition",
        action="store_true",
        help="compare the byte-identical Python core against sibling "
        "editions of the monorepo; skips outside a monorepo",
    )

    compact_command = commands.add_parser(
        "compact", help="archive terminal and superseded Brain records"
    )
    compact_command.add_argument("--json", action="store_true")

    reindex_bank_command = commands.add_parser(
        "reindex-bank",
        help="regenerate memory-bank/INDEX.md deterministically from chunk "
        "frontmatter (the legacy .memory-counter is ignored)",
    )
    reindex_bank_command.add_argument("--json", action="store_true")

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

    export = commands.add_parser(
        "export",
        help="write a privacy-filtered bundle of Brain records and Memory Bank chunks",
    )
    export.add_argument("--destination", type=Path, required=True)
    export.add_argument(
        "--include-archive",
        action="store_true",
        help="also export archived Brain records",
    )
    export.add_argument(
        "--include-superseded",
        action="store_true",
        help="also export superseded and archived Memory Bank chunks",
    )
    export.add_argument(
        "--force",
        action="store_true",
        help="overwrite a destination that already contains files",
    )
    export.add_argument("--json", action="store_true")
    return parser


def main() -> int:
    arguments = build_parser().parse_args()
    repository = arguments.root.resolve()

    try:
        if not repository.is_dir():
            raise ContextError(
                f"Repository root must be an existing directory: {repository}"
            )
        # Direct query commands have a strict CLI contract: reject the original
        # value before connect() can create a database or an index/manifest can
        # be refreshed. Host hooks remain fail-safe by swallowing this nonzero
        # result outside the CLI.
        validate_direct_query_request(arguments)
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
                    progress = progress.strip() if progress else None
                    if arguments.actor:
                        actor_slug = validate_actor(arguments.actor)
                        if progress:
                            progress = f"[{actor_slug}] {progress}"
                    result = apply_governed_update(
                        connection,
                        repository,
                        binding,
                        expected_revision=arguments.revision,
                        progress=progress,
                        next_steps=next_steps,
                        files=files,
                        sources=sources,
                        owner=owner,
                        phase=arguments.phase,
                        allow_phase_regression=arguments.allow_phase_regression,
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
                    if arguments.revision in (None, AUTO_REVISION):
                        raise ContextError(
                            "Governed completion requires the current numeric "
                            "--revision"
                        )
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

            if arguments.command in {"msg-send", "msg-read", "msg-dispatch"}:
                if mode == "lightweight":
                    raise ContextError(
                        "Agent messages require governed mode"
                    )
                # The audit trail outlives the local binding: a completed or
                # archived task has no binding, but its journal must stay
                # readable (and refuse writes with the honest terminal error),
                # so fall back to resolving by the Brain identifier itself.
                try:
                    binding = governed_binding(connection, arguments.task_id)
                    task_reference = str(binding["task_uuid"])
                except ContextError:
                    task_reference = validate_task_id(arguments.task_id)
                if arguments.command == "msg-send":
                    body = arguments.body
                    if arguments.body_file:
                        if body is not None:
                            raise ContextError(
                                "Pass --body or --body-file, not both"
                            )
                        body_path = Path(arguments.body_file)
                        if not body_path.is_file():
                            raise ContextError(
                                f"Message body file not found: {arguments.body_file}"
                            )
                        body = body_path.read_text(encoding="utf-8")
                    if body is None or not body.strip():
                        raise ContextError(
                            "Message body must not be empty (--body or --body-file)"
                        )
                    refs = validate_paths("Message ref", arguments.ref)
                    reject_secrets("message", [body, *refs])
                    message = append_message(
                        repository,
                        task_reference,
                        from_actor=arguments.from_actor,
                        to_actor=arguments.to_actor,
                        message_type=arguments.type,
                        body=body.strip(),
                        refs=refs,
                    )
                    if arguments.json:
                        print(json.dumps(message, ensure_ascii=False))
                    else:
                        print(
                            f"Message {message['seq']} sent: "
                            f"{message['from_actor']} -> {message['to_actor']} "
                            f"[{message['type']}]."
                        )
                    return 0
                if arguments.command == "msg-read":
                    messages = read_messages(
                        repository,
                        task_reference,
                        for_actor=arguments.for_actor,
                        since_seq=arguments.since,
                        message_type=arguments.type,
                    )
                    if arguments.json:
                        print(json.dumps(messages, ensure_ascii=False))
                    else:
                        if not messages:
                            print("No messages.")
                        for message in messages:
                            summary = " ".join(message["body"].split())
                            if len(summary) > 100:
                                summary = summary[:97] + "..."
                            print(
                                f"{message['seq']:>4} {message['created_at']} "
                                f"[{message['type']}] {message['from_actor']} -> "
                                f"{message['to_actor']}: {summary}"
                            )
                    return 0
                agent = validate_actor(arguments.agent)
                capsule_digest = None
                if arguments.capsule_file:
                    content = read_capsule_source(arguments.capsule_file)
                    problems = validate_capsule_text(content)
                    if problems and arguments.event == "spawn":
                        raise ContextError(
                            "Delegation capsule is under-specified: "
                            + "; ".join(problems)
                        )
                    capsule_digest = hashlib.sha256(
                        content.encode("utf-8")
                    ).hexdigest()
                note = arguments.note or f"{arguments.event}: {agent}"
                reject_secrets("dispatch note", [note])
                if arguments.event == "spawn":
                    from_actor, to_actor, message_type = "main", agent, "dispatch"
                else:
                    from_actor, to_actor, message_type = agent, "main", "completion"
                message = append_message(
                    repository,
                    task_reference,
                    from_actor=from_actor,
                    to_actor=to_actor,
                    message_type=message_type,
                    body=note.strip(),
                    capsule_digest=capsule_digest,
                )
                if arguments.json:
                    print(json.dumps(message, ensure_ascii=False))
                else:
                    print(
                        f"Dispatch {message['seq']} recorded: "
                        f"{arguments.event} {agent}."
                    )
                return 0

            if arguments.command == "capsule":
                content = read_capsule_source(arguments.file)
                problems = validate_capsule_text(content)
                result = {
                    "valid": not problems,
                    "length": len(content),
                    "problems": problems,
                }
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                elif problems:
                    print("Delegation capsule is under-specified:")
                    for problem in problems:
                        print(f"  - {problem}")
                else:
                    print(f"Delegation capsule is valid ({len(content)} characters).")
                return 0 if not problems else 1

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
                    and arguments.authority is None
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
                        authority=arguments.authority,
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

            if arguments.command == "hook-context":
                result = assemble_hook_context(
                    connection,
                    repository,
                    mode=mode,
                    task_id=arguments.task_id,
                )
                if result is None:
                    # A valid current branch with no meaningful change has no
                    # context. Cursor uses this distinct code to remove a
                    # foreign branch's stale rule; real failures remain code 1.
                    return 3
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
                retrieval_seconds = 0.0
                if arguments.query is not None:
                    retrieval_started = time.monotonic()
                    try:
                        # --query accepts the raw prompt: the hook passes it
                        # as-is and distillation to informative terms happens
                        # here. The raw text is screened before any of it can
                        # reach a query or a manifest.
                        reject_secrets("Task Capsule", [arguments.query])
                        capsule = assemble_capsule(
                            connection,
                            repository,
                            mode=mode,
                            query=distill_capsule_query(
                                connection, arguments.query
                            ),
                            task_id=arguments.task_id,
                            limit=arguments.limit,
                            ephemeral=arguments.ephemeral,
                            refresh_index=False,
                        )
                    except (ContextError, BrainError, RetrievalError) as error:
                        # A capsule needs an active task; the layer refresh
                        # above stands on its own and is already done.
                        warnings.append(f"Capsule unavailable: {error}")
                    retrieval_seconds = time.monotonic() - retrieval_started
                codebase = codebase_map_status(connection, repository)
                phases = {
                    "stat": 0.0,
                    "index": 0.0,
                    **dict(index_result.get("phase_seconds") or {}),
                    "retrieval": round(retrieval_seconds, 6),
                }
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
                    # Wall-clock seconds per phase, so an operator can see
                    # which side of the work approaches the hook budget.
                    "phases": phases,
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
                # A failed turn writes a report too: the Stop hook discards
                # stderr along with stdout, so an error that only printed
                # would leave the operator believing the flush landed. The
                # raw --task-id argv is deliberately absent from the failure
                # report — validation may have rejected it as secret-bearing,
                # and the report must not preserve what the run refused.
                try:
                    return run_turn(connection, repository, arguments, mode, owner)
                except (ContextError, BrainError, OSError, sqlite3.Error) as error:
                    write_last_turn_report(
                        repository,
                        {
                            "schema_version": 1,
                            "timestamp": datetime.now(timezone.utc).isoformat(),
                            "flushed": False,
                            "error": str(error),
                        },
                    )
                    raise

            if arguments.command == "rebind":
                if mode == "lightweight":
                    raise ContextError(
                        "rebind requires governed mode: lightweight tasks are "
                        "machine-local and have no Brain record to rebind"
                    )
                result = rebind_governed_task(
                    connection, repository, arguments.task_id, arguments.record
                )
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    verb = (
                        "already bound" if result["already_bound"] else "rebound"
                    )
                    print(
                        f"Working task {verb}: {result['task_id']} -> "
                        f"{result['task_uuid']} (revision {result['revision']})."
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
                if arguments.cross_edition:
                    cross = cross_edition_drift(repository)
                    if cross is None:
                        # A standalone (copied-out) edition has no siblings to
                        # compare against; that is a normal deployment, not a
                        # failure.
                        result = {
                            "valid": True,
                            "skipped": True,
                            "reason": "standalone-edition",
                        }
                        if arguments.json:
                            print(json.dumps(result, ensure_ascii=False))
                        else:
                            print(
                                "Cross-edition parity skipped: standalone "
                                "edition (no monorepo siblings found)."
                            )
                        return 0
                    result = {"valid": not cross, "skipped": False, "drift": cross}
                    if arguments.json:
                        print(json.dumps(result, ensure_ascii=False))
                    elif cross:
                        print(format_cross_edition_drift(cross), file=sys.stderr)
                    else:
                        print("Cross-edition core parity passed.")
                    return 0 if not cross else 1
                canonical = str(load_config(repository)["canonical_edition"])
                # Report through the result path rather than an exception, so
                # --json produces a machine-readable drift list on failure too.
                # Raising first made the --json branch unreachable, and a caller
                # that could only see the first drifted path had to re-run once
                # per file to finish a repair.
                drift = skill_mirror_drift(repository, canonical)
                result = {
                    "valid": not drift,
                    "canonical_edition": canonical,
                    "drift": drift,
                }
                if not arguments.skills_only:
                    # Full mode adds the MIRROR_RULES contract: every mirrored
                    # class (skills including non-markdown files, hooks,
                    # commands, agents, governance docs), under its own key so
                    # the skills drift list keeps its historical shape.
                    mirror_drift = full_mirror_drift(repository)
                    result["mirror_drift"] = mirror_drift
                    result["valid"] = not (drift or mirror_drift)
                else:
                    mirror_drift = []
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    if drift:
                        print(format_skill_mirror_drift(drift), file=sys.stderr)
                    if mirror_drift:
                        print(format_full_mirror_drift(mirror_drift), file=sys.stderr)
                    if result["valid"]:
                        scope = "Skill mirror" if arguments.skills_only else "Mirror"
                        print(f"{scope} parity passed ({canonical} canonical).")
                return 0 if result["valid"] else 1

            if arguments.command == "compact":
                result = compact(repository)
                if arguments.json:
                    print(json.dumps(result))
                else:
                    print(f"Project Brain compacted: {result['moved']} record(s) archived.")
                return 0

            if arguments.command == "reindex-bank":
                result = reindex_bank(repository)
                if arguments.json:
                    print(json.dumps(result))
                else:
                    state = "regenerated" if result["changed"] else "already up to date"
                    print(
                        f"Memory bank index {state}: "
                        f"{result['chunks']} chunk row(s)."
                    )
                return 0

            if arguments.command == "export":
                result = export_bundle(
                    repository,
                    arguments.destination.resolve(),
                    include_archive=arguments.include_archive,
                    include_superseded=arguments.include_superseded,
                    force=arguments.force,
                )
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    print(
                        f"Exported {result['brain_records']} Brain record(s) and "
                        f"{result['memory_chunks']} Memory Bank chunk(s) to "
                        f"{result['destination']}."
                    )
                    if result["excluded"]:
                        print(
                            f"  {result['excluded']} item(s) excluded; see MANIFEST.json "
                            "for each reason."
                        )
                    if result["auto_promoted_chunks"]:
                        print(
                            f"  {result['auto_promoted_chunks']} chunk(s) were promoted "
                            "automatically and never human-reviewed."
                        )
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
