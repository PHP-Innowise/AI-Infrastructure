#!/usr/bin/env python3
"""Canonical Project Brain + repository-local context CLI facade."""

from __future__ import annotations

SOURCE_HINT_PROFILE = "source-hints-v1"
SOURCE_PROVENANCE_PROFILE = "local-source-provenance-v1"

import argparse
import hashlib
import io
from contextlib import nullcontext
import math
import json
import os
import re
import shlex
import sqlite3
import stat as stat_module
import subprocess
import sys
import time
from datetime import date, datetime, timezone
from pathlib import Path, PurePosixPath
from typing import Callable, Optional

import brain_runtime
from brain_runtime import (
    BrainError,
    BrainRecordNotFound,
    LIFECYCLES,
    MESSAGE_BODY_LIMIT,
    MESSAGE_TYPES,
    PROMOTABLE_STATES,
    TASK_PHASE_INPUTS,
    TERMINAL_STATES,
    append_message,
    atomic_json,
    auto_compact,
    auto_consolidate,
    auto_promote,
    retry_pending_auto_promotion,
    cancel_task,
    ensure_attached_state,
    iter_records,
    close_task,
    compact,
    consolidate,
    configured_capsule_profile,
    configured_mode,
    configured_retrieval_gate,
    create_promotion,
    create_record,
    create_task,
    dynamic_path,
    get_record,
    get_task,
    find_record,
    find_task,
    validate_record,
    load_config,
    mutation_lock,
    read_messages,
    record_is_eligible,
    sources_are_fresh,
    audit_bank,
    brain_root,
    utc_now,
    validate_schema_file,
    iter_promotions,
    render_bank_index,
    promotable_records,
    reindex_bank,
    retire_chunk,
    reverify_chunk,
    render_current_state,
    rollback_created_record,
    review_promotion,
    apply_promotion,
    stale_record_states,
    restore_record_state,
    snapshot_record_state,
    update_record,
    update_task,
    validate_actor,
    validate_repository,
)
from context_retrieval import (
    AUTOMATIC_LINK_SELECTIONS,
    TASK_TWINS_KEY,
    _CurrentSourcePolicy,
    _capsule_index_images,
    _carry_index_images,
    _with_index_images,
    TOUCH_SEED_LIMIT,
    prompt_path_seeds,
    CAPSULE_EPISODIC_LIMIT,
    CAPSULE_EVENT_LIMIT,
    CAPSULE_PROMPT_TERM_LIMIT,
    DocumentRow,
    _content_hash,
    episode_item,
    EVIDENCE_STOPWORDS,
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
    effective_canonical_edition,
    format_skill_mirror_drift,
    full_mirror_drift,
    skill_mirror_drift,
    ensure_metadata_tables,
    index_documents,
    load_index_state,
    store_index_state,
    codebase_map_drift,
    evidence_tokens,
    expand_query_tokens,
    fts_query_tokens,
    excerpt_weights,
    excerpt_section,
    excerpt_window,
    fts_query,
    _unmarked,
    is_relevant,
    linked_documents,
    source_link_candidates,
    marked_document,
    match_strength,
    quoted_section,
    refresh_health_retention,
    RETRIEVAL_GATE_DEFAULT,
    RETRIEVAL_GATE_MODES,
    RETRIEVAL_HOSTS,
    TOKENIZER,
    query_tokens,
    required_coverage,
    retrieve,
    record_render_trace,
    diagnose_funnel,
    valid_funnel_diagnosis,
    unknown_funnel_diagnosis,
    unknown_ordinary_receipt,
    normalize_ordinary_receipt,
    reusable_source_state,
    token_coverage,
)
import workspace_roots
# One pattern for paths a turn may not record and sources a learning may not
# cite: secrets, keys, environment files.
from memory_results import SOURCE_PATH_DENYLIST as TURN_PATH_DENYLIST, record_result, prepare_result
from automatic_query import source_path_problem
from validate import (
    PRIVATE_PATTERNS,
    SECRET_PATTERNS,
    ValidationError,
    mask_secrets,
    parse_frontmatter,
    source_fact_memo,
    validate_metadata,
    validate_secret_patterns,
    sanitize_automatic_query as _sanitize_automatic_query,
)
from context_handoff import HandoffError, load_handoff, render_body, save_handoff


ContextError = workspace_roots.ContextError


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
    # Recursive, like every other document tree here. A codebase map big
    # enough to be worth writing is filed into subdirectories, and a
    # single-level glob silently indexed the top of it and nothing else.
    ("semantic", "codebase", "codebase/**/*.md"),
    ("semantic", "memory", "memory-bank/chunks/*.md"),
    ("semantic", "task", "tasks/**/*.md"),
    # Task/ (client product specifications and design references) is
    # deliberately not a retrieval source: its prose is foreign to this
    # repository's own documentation and competed with it in BM25 ranking.
    ("episodic", "changelog", "CHANGELOG.md"),
)
# Sources the accelerator ships or its own workflows write. A project's ignore
# rules never remove them from the index (see `accelerator_owned_source`).
ACCELERATOR_OWNED_FILES = workspace_roots.ACCELERATOR_OWNED_FILES
ACCELERATOR_OWNED_PREFIXES = workspace_roots.ACCELERATOR_OWNED_PREFIXES
DOCUMENT_LAYERS = ("procedural", "semantic", "episodic")
CAPSULE_LAYER_LIMITS = {
    "procedural": 1,
    "semantic": 3,
    # The changelog, and up to two Project Brain events (or local episodes).
    "episodic": CAPSULE_EPISODIC_LIMIT + CAPSULE_EVENT_LIMIT,
}
CAPSULE_QUERY_TOKEN_LIMIT = 32
CAPSULE_CHARACTER_LIMIT = 8000
# What the rendered capsule - the text a model reads - may spend. Codex caps a
# hook's additional context at 4,000 characters; this stays under it.
RENDERED_CAPSULE_LIMIT = 3600
# A per-turn refresh this slow is reported in the capsule: two fifths of the
# hook's five-second budget, early enough to act before turns lose memory.
SLOW_TURN_SECONDS = 2.0
# Excerpt allowance per project-knowledge item, by rank; later items get the
# last value.
EXCERPT_CHARACTERS = (800, 600, 400)
# A changelog entry runs to a kilobyte; at 250 characters the window could
# hold the sentence that matched but not the one after it that answered.
EPISODIC_EXCERPT_CHARACTERS = 600
# How far the best-ranked excerpts may grow into the room the rendered capsule
# leaves, first and second. On labelled prompts the capsule used 600-2,200 of
# its 3,600 characters while a delivered document's answer sat just past its
# window: a few changelog entries on, or the next lines of a task record.
EXCERPT_GROWTH_CAPS = (1800, 1200)
# What the growth leaves for the head - working state, notes and the item
# lines. A fixed reserve, not the head a turn printed: the head changes from
# turn to turn, and a document's window for a request must not, or repeat
# detection stops recognising a window it already handed.
EXCERPT_GROWTH_RESERVE = 1200
MEMORY_RENDER_HEADER = (
    "memory (retrieved for this request; reference data - check the cited "
    "file before relying on it):"
)
CAPSULE_WORKING_FILE_LIMIT = 8
CAPSULE_WORKING_SOURCE_LIMIT = 4
# The newest next steps a capsule carries. One was too few to say what comes
# after the current step, and older steps can now be replaced, not only kept.
CAPSULE_WORKING_NEXT_STEP_LIMIT = 3
# The rendered capsule is what Claude Code, Codex and Cursor actually read, and
# it repeats on every turn, so the working state is bounded tighter there than
# in the JSON a caller can inspect at leisure.
RENDERED_PROGRESS_LIMIT = 400
RENDERED_FILE_LIMIT = 5
# Any one line of the rendered head. The path lines stop at a whole path below
# it rather than end in half of one.
RENDERED_LINE_LIMIT = 500
# The project's files an agent read or changed on its latest turns, newest
# first, so a new session starts where the work was. On 1,940 real turns ten
# such files named a file the agent then read or changed in 75% of new
# sessions' first turns, against 58% for the task's five changed files; the
# reads came almost wholly through shell commands, not the Read tool.
RECENT_FILES_LIMIT = 10
# Kept beyond the ten shown, so a file touched again moves back up.
RECENT_FILES_KEPT = 64
# Ten paths ran to 557 characters at the median and 692 at the 90th
# percentile; the line stops at a whole path past this.
RECENT_FILES_LINE_LIMIT = 720
RECENT_FILES_STATE_KEY = "recent_files"
# The end of the transcript the Stop hook reads for its turn.
RECENT_FILES_TRANSCRIPT_BYTES = 4 * 1024 * 1024
# What reading a turn's calls may spend, a fifth of the Stop hook's budget:
# the files found by then are kept. 400 commands of 43 words took 0.24 s on
# Linux; a slow disk pays for every path a word could name.
RECENT_FILES_PARSE_SECONDS = 1.0
# Dependencies, caches and the runtime's own state are not where the work is,
# and prose reaches the capsule through the memory layers.
RECENT_FILES_SKIPPED_PREFIXES = ("vendor/", "node_modules/", "var/", ".git/", "memory-bank/local/")
RECENT_FILES_SKIPPED_SUFFIXES = (".md", ".mdx", ".rst", ".txt", ".adoc")
# Defined beside SECRET_PATTERNS in validate.py, so the Project Brain write
# path (brain_runtime) refuses the same personal data the capsule does.
CAPSULE_PRIVATE_PATTERNS = tuple(PRIVATE_PATTERNS.values())
CAPSULE_RAW_TEXT_PATTERN = re.compile(
    r"^\s*(?:user|assistant|system|developer|tool|prompt|response|reasoning|"
    r"stdout|stderr|log)\s*:",
    re.IGNORECASE | re.MULTILINE,
)
# Letters and digits of any script: a task ID is usually the branch name, and
# a branch called feature/счёт-экспорт had no memory at all - its prompts got
# "Capsule unavailable" and its turns were never recorded.
TASK_ID_PATTERN = re.compile(r"^[^\W_][\w\u0300-\u036f./#+-]{0,127}$")
# A skill's identity lives in its declared description, not in its body prose,
# which reads much alike across skills. Indexing that separately lets ranking
# weight what a document is *about* over what it happens to mention.
DESCRIPTION_PATTERN = re.compile(r"^description:\s*(.+)$", re.MULTILINE)
SUMMARY_CHARACTERS = 400
AUTO_REVISION = "auto"
DEFAULT_TURN_FLUSH_AFTER = 5
DEFAULT_TURN_FILE_LIMIT = 20
# What a checkpoint says about committed work: the branch's newest commit
# subjects. A subject is already shared history and already a person's summary
# of a change, which a list of touched paths never is.
CHECKPOINT_COMMIT_LIMIT = 5
CHECKPOINT_SUBJECT_CHARACTERS = 80
CHECKPOINT_LOG_LIMIT = 500
# The HEAD each task last saw, so a turn that ends in a commit - and so leaves
# a clean tree - still counts as work. One map under one key, bounded, because
# nothing prunes index_state.
TURN_HEADS_KEY = "turn-heads"
TURN_HEADS_RETENTION = 200
LAST_TURN_REPORT_FILENAME = "last-turn-report.json"
# The report describes one Stop-hook turn, so it is only meaningful for the
# session that produced it. A day bounds any plausible gap between two working
# sessions; anything older describes work the operator no longer remembers.
LAST_TURN_REPORT_MAX_AGE_SECONDS = 24 * 60 * 60
# The rendered section is telemetry, not retrieved knowledge, so it may never
# crowd the capsule: one compact line, problems first.
LAST_TURN_SECTION_CHARACTER_LIMIT = 600
LAST_TURN_SECTION_PATH_LIMIT = 3
TURN_RUNTIME_DIRECTORIES = ("dynamic", "control", "indexes", "archive", "local")
BRANCH_PREFIXES = (
    "feature/", "feat/", "fix/", "bugfix/", "hotfix/", "chore/", "release/",
    "refactor/", "docs/", "test/", "merge/",
)
TICKET_PATTERN = re.compile(r"[A-Za-z][A-Za-z0-9]*-\d+")


def default_root() -> Path:
    # The state root: this install, or ACCELERATOR_STATE_DIR when the
    # accelerator is attached to a project from its own clone.
    return workspace_roots.default_state_root()


def default_database(repository: Path) -> Path:
    return repository / "memory-bank" / "local" / "context.db"


def connect(database: Path, *, timeout: float = 5.0, require_write: bool = False,
            busy_seconds: Optional[float] = None) -> sqlite3.Connection:
    database.parent.mkdir(parents=True, exist_ok=True)
    connection: Optional[sqlite3.Connection] = None
    try:
        connection = sqlite3.connect(database, timeout=timeout)
        connection.row_factory = sqlite3.Row
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
        # `discover_documents` returns only *changed* documents, so a link
        # table created beside an existing index would be populated for the
        # handful of files that happen to change next and silently claim to be
        # complete. Dropping the stat cache forces one full re-read, the same
        # remedy the tokenizer change above uses. A new database has no cache
        # to drop, so this costs nothing there.
        if connection.execute(
            "SELECT name FROM sqlite_master WHERE name = 'document_links'"
        ).fetchone() is None:
            connection.execute("DROP TABLE IF EXISTS document_source_state")
        # The same for a metadata column added later: retained rows would
        # otherwise keep the column's default until their file changes.
        metadata_columns = {
            row["name"]
            for row in connection.execute("PRAGMA table_info(document_metadata)")
        }
        if metadata_columns and "attestation" not in metadata_columns:
            connection.execute("DROP TABLE IF EXISTS document_source_state")
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
        if require_write:
            # BEGIN plus already-existing IF NOT EXISTS schema can succeed on
            # a read-only image. Require SQLite's actual write admission, with
            # no row changes, before choosing the normal refresh writer path.
            connection.execute('UPDATE index_state SET value=value WHERE 0')
        connection.commit()
        # The startup wait decides only whether this refresh may write at
        # all. Once admitted, a concurrent session's commit is waited for as
        # any other connection waits, instead of failing the turn mid-refresh.
        busy = CONNECTION_BUSY_SECONDS if busy_seconds is None else busy_seconds
        connection.execute(f'PRAGMA busy_timeout = {max(1, int(busy * 1000))}')
        return connection
    except BaseException as error:
        if connection is None:
            raise
        # This handle is owned until initialization returns. Cleanup failures
        # must neither skip the other attempt nor replace the original error.
        try:
            connection.rollback()
        except BaseException:
            pass
        try:
            connection.close()
        except BaseException:
            pass
        if isinstance(error, sqlite3.OperationalError) and "fts5" in str(error).lower():
            raise ContextError("SQLite FTS5 support is required") from error
        raise


# A refresh waits this long for write admission before falling back to the
# read-only working-state path. Another session's refresh holds the lock for
# a fraction of a second; 50 ms lost a parallel session's knowledge in about
# a third of turns. A read-only database is refused at once, without waiting.
# Under a hook the waits are shares of the hook's own budget, so a locked
# database still leaves time for the working-state answer (_refresh_waits).
REFRESH_STARTUP_WAIT = 2.0
CONNECTION_BUSY_SECONDS = 5.0


def _refresh_waits(arguments: argparse.Namespace) -> tuple[float, float]:
    """(write-admission wait, busy wait) for this refresh."""
    budget = None
    raw = os.environ.get("CONTEXT_HOOK_BUDGET")
    if raw is not None:
        try:
            budget = float(raw)
        except ValueError:
            budget = None
        if budget is not None and not (budget > 0 and budget != float("inf")):
            budget = None
    if budget is None and getattr(arguments, "host", None):
        budget = 5.0  # the hooks' default budget
    if budget is None:
        return REFRESH_STARTUP_WAIT, CONNECTION_BUSY_SECONDS
    return (min(REFRESH_STARTUP_WAIT, max(0.05, budget * 0.25)),
            min(CONNECTION_BUSY_SECONDS, max(0.05, budget * 0.5)))
READ_ONLY_COLUMNS = {
    'documents': ('path', 'layer', 'kind', 'title', 'summary', 'content'),
    'episodes': ('summary', 'outcome', 'files', 'verification', 'sources', 'created_at'),
    'working_tasks': ('task_id', 'goal', 'progress', 'auto_checkpoint', 'next_steps', 'files', 'sources', 'created_at', 'updated_at'),
    'task_bindings': ('external_id', 'task_uuid', 'revision', 'updated_at'),
    'document_metadata': ('path', 'category', 'privacy', 'owner', 'authority', 'lifecycle', 'source_hash', 'record_id', 'conflicts', 'source_fingerprints', 'updated_at', 'confidence', 'attestation'),
    'document_source_state': ('path', 'mtime_ns', 'size', 'eligible_until'),
    'document_links': ('path', 'ref_path', 'ref_kind'),
    'index_state': ('key', 'value'),
    'turn_deltas': ('id', 'task_id', 'created_at', 'files'),
}


def connect_read_only(database: Path, *, timeout: float = REFRESH_STARTUP_WAIT) -> sqlite3.Connection:
    """Existing compatible local state; no initialization or migrations.

    This handle is admitted for working-only delivery, never cached retrieval.
    SQLite still takes normal read locks: an exclusive writer is unavailable.
    """
    # A read-only WAL open can create its shared-memory sidecar. This finite
    # fallback supports rollback-journal images only; immutable=1 would ignore
    # live writer locks and is not safe for this mutable state.
    with database.open('rb') as handle:
        header = handle.read(20)
    if header[:16] != b'SQLite format 3\x00' or header[18:20] != b'\x01\x01':
        raise ContextError('Unsupported read-only local state profile')
    connection = sqlite3.connect(database.resolve().as_uri() + '?mode=ro',
                                 uri=True, timeout=timeout)
    connection.row_factory = sqlite3.Row
    try:
        connection.execute('PRAGMA query_only=ON')
        keys = {'working_tasks': ('task_id',), 'task_bindings': ('external_id',),
                'document_metadata': ('path',), 'document_source_state': ('path',),
                'document_links': ('path', 'ref_path', 'ref_kind'),
                'index_state': ('key',), 'turn_deltas': ('id',)}
        for table, expected in READ_ONLY_COLUMNS.items():
            schema = connection.execute(
                'SELECT type, sql FROM sqlite_master WHERE name=?', (table,)
            ).fetchone()
            columns = connection.execute('PRAGMA table_info(' + table + ')').fetchall()
            if (schema is None or schema['type'] != 'table'
                    or {row['name'] for row in columns} != set(expected)):
                raise ContextError('Unsupported read-only local state profile')
            sql = (schema['sql'] or '').casefold()
            if table in ('documents', 'episodes'):
                unindexed = ('path', 'layer', 'kind') if table == 'documents' else ('created_at',)
                fields = ','.join(column + (' UNINDEXED' if column in unindexed else '')
                                  for column in expected)
                canonical = "CREATE VIRTUAL TABLE " + table + " USING fts5(" + fields + ",tokenize='" + TOKENIZER + "')"
                if re.sub(r'\s+', '', sql) != re.sub(r'\s+', '', canonical.casefold()):
                    raise ContextError('Unsupported read-only local state profile')
            elif ('create virtual table' in sql or tuple(
                    row['name'] for row in sorted(columns, key=lambda row: row['pk'])
                    if row['pk']) != keys[table]):
                raise ContextError('Unsupported read-only local state profile')
        unique_uuid = False
        for row in connection.execute('PRAGMA index_list(task_bindings)'):
            if row['unique'] and not row['partial']:
                indexed = tuple(item['name'] for item in connection.execute(
                    'SELECT name FROM pragma_index_info(?) ORDER BY seqno', (row['name'],)))
                unique_uuid = unique_uuid or indexed == ('task_uuid',)
        if not unique_uuid:
            raise ContextError('Unsupported read-only local state profile')
        for name, table, expected in (
                ('document_metadata_conflicts', 'document_metadata', ('conflicts',)),
                ('document_links_ref', 'document_links', ('ref_path',)),
                ('document_links_source_path', 'document_links', ('ref_path', 'ref_kind', 'path'))):
            schema = connection.execute(
                'SELECT type, tbl_name FROM sqlite_master WHERE name=?', (name,)
            ).fetchone()
            indexed = tuple(row['name'] for row in connection.execute(
                'SELECT name FROM pragma_index_info(?) ORDER BY seqno', (name,)))
            if schema is None or tuple(schema) != ('index', table) or indexed != expected:
                raise ContextError('Unsupported read-only local state profile')
        return connection
    except BaseException:
        connection.close()
        raise


def degraded_refresh(
    connection: Optional[sqlite3.Connection], repository: Path,
    arguments: argparse.Namespace, mode: str, delivery_profile: str,
    delivery_admission: Optional[Callable[[], None]] = None,
) -> int:
    """Fresh working state only; no index, history, receipts or filesystem writes."""
    # Not the Brain's carried record images either.
    brain_runtime.hold_persisted_images()
    warning = ('Memory refresh incomplete: refresh budget exhausted; retrieval was not consulted.'
               if getattr(arguments, 'read_only_reason', None) == 'budget' else
               'Memory refresh incomplete: SQLite writes unavailable; retrieval was not consulted.'
               if connection is not None else
               'Memory refresh incomplete: local state unavailable; retrieval was not consulted.')
    warnings = [warning]
    capsule = None
    if connection is not None and arguments.query is not None:
        try:
            query = arguments.query
            reject_secrets('Task Capsule', [query])
            reject_capsule_privacy('Task Capsule request', [query])
            task_record = None
            task = None
            if mode == 'governed':
                binding = governed_binding(connection, arguments.task_id)
                task_record, task, _ = find_task(repository, arguments.task_id)
                validate_record(task)
                if (task['id'] != binding['task_uuid']
                        or task['external_id'] != binding['external_id']
                        or arguments.task_id not in (task['id'], task['external_id'])):
                    raise ContextError('Working task identity unavailable')
                eligible, _ = record_is_eligible(repository, task, load_config(repository))
                # A cited file that changed since the checkpoint, or one the
                # project ignores, does not hide the task here either: the
                # full refresh delivers this same working state.
                if (not eligible or task['conflicts']
                        or any(source_path_problem(source) for source in task['sources'])):
                    raise ContextError('Working task eligibility unavailable')
                raw_working = governed_task_view(task)
            else:
                raw_working = find_working_task(connection, arguments.task_id)
                if raw_working is None or raw_working['task_id'] != arguments.task_id:
                    raise ContextError('Working task unavailable')
            working, omitted = project_working_task(raw_working, delivery_profile=delivery_profile)
            capsule = {
                'mode': mode, 'query': build_capsule_query(query, None), 'query_source': 'prompt',
                'task_id': task['external_id'] if task else arguments.task_id,
                'working': working, 'omitted': omitted,
                'procedural': [], 'semantic': [], 'episodic': [], 'selected': [],
                'categories': {}, 'no_match': [], 'manifest': None, 'last_turn': None,
                'warnings': warnings,
            }
            if task is not None:
                capsule.update(task_uuid=task['id'], task_revision=task['revision'],
                               task_record=task_record.relative_to(repository).as_posix())
            capsule = (enforce_governed_capsule_contract(capsule) if mode == 'governed'
                       else enforce_capsule_budget(capsule))
        except SourceGitError:
            raise
        except (ContextError, BrainError, OSError, sqlite3.Error, ValueError, TypeError, KeyError):
            capsule = None
            warnings.append('Working state unavailable: current identity or eligibility could not be verified.')
    result = {'mode': mode, **{layer: 'failed' for layer in DOCUMENT_LAYERS},
              'startup_profile': 'read-only-working-v1', 'documents': None,
              'reused': None, 'counts': {}, 'warnings': warnings,
              'capsule': capsule, 'persisted': False}
    if capsule is not None:
        result['capsule_text'] = '\n'.join(render_capsule_lines(
            capsule, {}, delivery_profile=delivery_profile))
    elif delivery_profile == 'cursor-rule':
        result['capsule_text'] = 'working: unavailable\nwarning: ' + warning
    if delivery_admission is not None:
        delivery_admission()
    if arguments.json:
        print(json.dumps(result, ensure_ascii=False))
    elif capsule is not None:
        print(result['capsule_text'])
    else:
        for item in warnings:
            print('warning: ' + item)
    return 1


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


def memory_eligible_until(metadata: dict) -> Optional[str]:
    """The date a chunk stops being servable: ``min(review_after, valid_to)``.

    Eligibility is a calendar fact, not a filesystem one, and the incremental
    cache keys on ``(mtime_ns, size)``. Recording the boundary alongside the
    stat pair is what lets a cached row expire on its own date without the
    file being touched. ``review_after`` is required of every chunk, so an
    eligible chunk always has a boundary; ``None`` means the frontmatter
    carries no parseable date, which validation rejects anyway.
    """
    bounds = []
    for key in ("review_after", "valid_to"):
        value = metadata.get(key)
        if not isinstance(value, str):
            continue
        try:
            bounds.append(date.fromisoformat(value))
        except ValueError:
            return None
    return min(bounds).isoformat() if bounds else None


def _lapsed_reason(metadata: dict, eligible_until: Optional[str]) -> str:
    """Name the boundary a chunk crossed, or 'invalid' if it crossed none.

    A chunk that reached its own end date is not malformed - it is retired,
    which is the outcome the lifecycle is for. Reporting that as `invalid`
    tells a reader to go fix a file that is doing exactly what it promised.
    """
    if metadata.get("status") != "active" or eligible_until is None:
        return "invalid"
    if date.today() <= date.fromisoformat(eligible_until):
        return "invalid"
    valid_to = metadata.get("valid_to")
    if isinstance(valid_to, str):
        try:
            if date.today() > date.fromisoformat(valid_to):
                return "retired"
        except ValueError:
            return "invalid"
    return "overdue-review"


def memory_eligibility(
    path: Path, repository: Path, *, screened_hash: Optional[str] = None
) -> tuple[Optional[dict], Optional[str], Optional[str]]:
    """Frontmatter if the chunk may be served, else why not, plus its boundary.

    Returns ``(metadata, None, eligible_until)`` for a servable chunk and
    ``(None, reason, eligible_until)`` otherwise. The reason vocabulary is the
    one the retrieval manifest already speaks - `retired`, `overdue-review`,
    `invalid`, `secret`, plus the non-active statuses - so a dropped chunk
    reaches the `excluded` channel instead of vanishing on a bare `continue`.

    ``screened_hash`` is the content hash an index that admitted the chunk
    stored for it, under the same secret and provenance policies (a changed
    policy rebuilds the index). Text that still hashes to it is not scanned
    again: the secret scan read the whole chunk against two dozen patterns on
    every prompt, 0.4 ms a chunk - most of a prompt's cost once the Memory
    Bank holds thousands - and the provenance scan of an auto-promoted chunk
    read it a second time (1,000 such chunks took a retrieval from 141 to
    186 ms). Other text, even behind an unchanged stat, is scanned.
    Citations are always checked: files go away.
    """
    try:
        metadata = parse_frontmatter(path)
    except (OSError, ValidationError):
        return None, "invalid", None
    eligible_until = memory_eligible_until(metadata)
    try:
        validate_metadata(path, metadata, repository)
    except OSError:
        return None, "invalid", eligible_until
    except ValidationError:
        return None, _lapsed_reason(metadata, eligible_until), eligible_until
    try:
        text: Optional[str] = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError):
        text = None
    screened = screened_hash is not None and text is not None and _content_hash(text) == screened_hash
    if not screened:
        try:
            validate_secret_patterns(path)
        except (OSError, ValidationError):
            return None, "secret", eligible_until
    status = metadata["status"]
    if status != "active":
        return None, status, eligible_until
    tags = metadata.get("tags")
    if not screened and isinstance(tags, list) and "auto-promoted" in tags:
        # Automatic promotion wrote this chunk without a person reading it.
        # Instruction-like text is withheld automatically; a remote citation
        # is not, because a pinned source URL is legitimate evidence and
        # withholding every one of them lost a measured useful chunk. The
        # remote citation stays visible to `bank-audit --provenance`.
        if text is None or brain_runtime.instruction_like(metadata.get("title"), text):
            return None, "provenance", eligible_until
    return metadata, None, eligible_until


def active_memory(path: Path, repository: Path) -> Optional[dict]:
    """The chunk's frontmatter when it may be served, else ``None``.

    Returning the metadata rather than a bare bool keeps every caller's
    truthiness test working while giving the ones that need provenance
    something to read.
    """
    return memory_eligibility(path, repository)[0]


def accelerator_owned_source(path: str) -> bool:
    """Whether a source key is the accelerator's own, not the project's.

    The ignore probe exists for the project's documents: a team that ignores
    `docs/private/` has said those files are not shared knowledge. Installs
    routinely ignore the accelerator itself to keep it out of the client's
    history, and the probe then dropped every skill, AGENTS.md and the Memory
    Bank from the index - on real installations 12 and 18 documents were left
    of 120, retrieval routed 0 of 14 requests, and nothing said so.
    """
    return path in ACCELERATOR_OWNED_FILES or path.startswith(
        ACCELERATOR_OWNED_PREFIXES
    )


SourceGitError = workspace_roots.SourceGitError
_SOURCE_GIT_ERROR = workspace_roots._SOURCE_GIT_ERROR
_source_git_environment = workspace_roots._source_git_environment
_source_git_checkout = workspace_roots._source_git_checkout
_source_git_run = workspace_roots._source_git_run


def _source_git_frame(project, environment, deadline):
    return workspace_roots._source_git_frame(project, environment, deadline,
        _checkout=_source_git_checkout, _run=_source_git_run)


def _source_git_ignored(project, paths, timeout):
    return workspace_roots._source_git_ignored(project, paths, timeout,
        _environment=_source_git_environment, _frame=_source_git_frame, _run=_source_git_run)


def _rebased_branch(project: Path) -> Optional[str]:
    """The branch a rebase in progress replays onto a detached HEAD.

    Git keeps its full name in ``rebase-merge/head-name`` (``rebase-apply/``
    for the apply backend) of the worktree's own Git directory, and removes
    the directory when the rebase ends or is aborted. The directory is found
    as Git finds it - ``.git`` itself, or the ``gitdir:`` a linked worktree's
    or submodule's ``.git`` file names - without starting Git again.
    """
    try:
        checkout = _source_git_checkout(project)
        if checkout is None:
            return None
        directory = checkout / ".git"
        if not directory.is_dir():
            with directory.open("rb") as handle:
                pointer = handle.read(4097)
            line = pointer.decode("utf-8", "strict").splitlines()[0] if pointer else ""
            if len(pointer) > 4096 or not line.startswith("gitdir: "):
                return None
            directory = checkout / line[len("gitdir: "):].strip()
        for state in ("rebase-merge", "rebase-apply"):
            try:
                with (directory / state / "head-name").open("rb") as handle:
                    raw = handle.read(4097)
            except OSError:
                continue
            name = raw.decode("utf-8", "strict").rstrip("\r\n")
            if (len(raw) <= 4096 and name.startswith("refs/heads/") and len(name) > len("refs/heads/")
                    and not any(character in name for character in ("\0", "\r", "\n"))):
                return name[len("refs/heads/"):]
    except (SourceGitError, OSError, UnicodeError, ValueError):
        return None
    return None


def _admit_inferred_task_branch(repository: Path, expected: Optional[str]) -> None:
    """Recheck a hook-inferred identity; explicit task IDs add no branch probe."""
    if expected is None:
        return
    message = "Inferred task branch changed or unavailable"
    try:
        if (type(expected) is not str or not expected or len(expected.encode("utf-8")) > 4000
                or any(character in expected for character in ("\0", "\r", "\n"))):
            raise SourceGitError(message)
        project = workspace_roots.project_root(repository)
        if _source_git_checkout(project) is None:
            raise SourceGitError(message)
        # One probe: `git -C <project>` with the command environment's Git
        # selectors stripped finds the project's own repository. A prompt
        # asks once; the checks before delivery ask again only when the
        # checkout's HEAD changed (_branch_admission). A pinned before/after
        # frame around it tripled the Git starts and failed outright whenever
        # Git was slow.
        observed = _source_git_run(project, ["symbolic-ref", "--quiet", "HEAD"],
                                   _source_git_environment(), time.monotonic() + 2.0)
        # A rebase detaches HEAD; the branch it rewrites stays the task.
        if observed.returncode == 1 and _rebased_branch(project) == expected:
            return
        if (observed.returncode != 0
                or not isinstance(observed.stdout, bytes) or len(observed.stdout) > 4096
                or observed.stdout != ("refs/heads/" + expected + "\n").encode("utf-8")):
            raise SourceGitError(message)
    except (SourceGitError, OSError, UnicodeError, ValueError) as error:
        raise SourceGitError(message) from error


def _branch_admission(repository: Path, expected: str) -> Callable[[], None]:
    """Admit the inferred branch now; return the check to repeat before
    delivery. That check asks Git again only when the checkout's HEAD, or its
    reftable list, changed since the last answer - each snapshot is taken
    before Git is asked, so a switch during the question still shows."""
    project = workspace_roots.project_root(repository)
    held = [workspace_roots.branch_snapshot(project)]
    _admit_inferred_task_branch(repository, expected)

    def readmit() -> None:
        current = workspace_roots.branch_snapshot(project)
        if held[0] is not None and current == held[0]:
            return
        _admit_inferred_task_branch(repository, expected)
        held[0] = current

    return readmit


def inside_git_checkout(directory: Path) -> bool:
    return _source_git_checkout(directory) is not None


def git_ignored_paths(repository: Path, paths: list[str]) -> set[bytes]:
    """Apply project ignore policy, keeping ordinary accelerator exemptions."""
    if workspace_roots.is_attached(repository):
        paths = [path for path in paths
                 if not Path(path).is_absolute() and not workspace_roots.is_state_key(path)]
    paths = [path for path in paths if not accelerator_owned_source(path)]
    return _source_git_ignored(workspace_roots.project_root(repository), paths, 2.0)


# One discovered source file: layer, kind, absolute path, repository-relative
# path, and the (mtime_ns, size) stat pair every downstream consumer needs.
# The originating glob travels with each candidate so discovery can report a
# pattern that matched files on disk and lost every one of them to .gitignore.
SourceCandidate = tuple[str, str, Path, str, tuple[int, int], str]


def _invalid_source_key(key: str) -> Optional[dict[str, str]]:
    try:
        key.encode('utf-8')
    except UnicodeError:
        digest = hashlib.sha256(key.encode('utf-8', 'surrogatepass')).hexdigest()[:16]
        return {'path': '<invalid-utf8-path:'+digest+'>', 'reason': 'invalid-utf8-path'}
    return None


def collect_source_candidates(repository: Path, *, excluded=None) -> list[SourceCandidate]:
    """Walk SOURCE_PATTERNS once, statting each candidate exactly once.

    Document discovery and the skill-tree fingerprint used to walk and stat
    the same trees independently — hundreds of mirrored skill files statted
    twice on every prompt. Everything a consumer needs from the filesystem is
    captured here, so both derive from a single pass. One lstat per file also
    answers regular-vs-symlink: a symlink is excluded as before, only without
    the extra stat calls `is_file()`/`is_symlink()` would each spend.
    """
    candidates: list[SourceCandidate] = []
    rejected = set()
    for layer, kind, pattern, base in source_scan_plan(repository):
        for path in sorted(base.glob(pattern)):
            try:
                status = os.lstat(path)
            except OSError:
                continue
            if not stat_module.S_ISREG(status.st_mode):
                continue
            key = workspace_roots.key_for(repository, base, path)
            problem = _invalid_source_key(key)
            if problem is not None:
                if kind == 'skill':
                    # Policy inventory is strict; never turn a malformed
                    # skill into a clean cached parity decision by skipping it.
                    raise RetrievalError('Skill source invalid-utf8-path')
                if excluded is not None and problem['path'] not in rejected:
                    excluded.append(problem)
                rejected.add(problem['path'])
                continue
            candidates.append(
                (
                    layer,
                    kind,
                    path,
                    key,
                    (status.st_mtime_ns, status.st_size),
                    pattern,
                )
            )
    return candidates


SKILL_TREE_DIRECTORIES = (".agents", ".claude", ".cursor", ".codex")


def skill_key_parts(key: str) -> tuple[str, str]:
    """A skill document's tool directory and its path inside that skills tree.

    `.claude/skills/review/SKILL.md` gives `(".claude", "review/SKILL.md")`.
    An attached tooling key is an absolute path, so the tree is found by its
    last occurrence rather than by the first path segment.
    """
    found = ("", key)
    best = -1
    for directory in SKILL_TREE_DIRECTORIES:
        marker = f"{directory}/skills/"
        position = key.rfind("/" + marker)
        start = position + 1 if position >= 0 else (0 if key.startswith(marker) else -1)
        if start > best:
            best = start
            found = (directory, key[start + len(marker):])
    return found


def source_scan_plan(repository: Path) -> list[tuple[str, str, str, Path]]:
    """Each source pattern with the root it is read from.

    Installed, every root is the repository and the plan is SOURCE_PATTERNS
    as written. Attached, policy and skills come from the accelerator clone
    and also from the project (its own AGENTS.md or skills, if any), durable
    memory from the state directory, and every other document from the
    project, because the accelerator's own README, specs and changelog
    describe the accelerator rather than the work.
    """
    layout = workspace_roots.roots(repository)
    plan: list[tuple[str, str, str, Path]] = []
    for layer, kind, pattern in SOURCE_PATTERNS:
        if kind == "memory":
            bases = (layout.state,)
        elif layer == "procedural":
            bases = (layout.tooling, layout.project)
        else:
            bases = (layout.project,)
        for base in dict.fromkeys(bases):
            plan.append((layer, kind, pattern, base))
    return plan


def _cache_entry_is_current(
    kind: str, cached: tuple, current: tuple[int, int]
) -> bool:
    """Whether a cached row may be reused without re-reading the file.

    The stat pair answers "has the file changed"; for durable memory that is
    only half the question, because a chunk also stops being servable on a
    date nobody has to touch it for. The recorded boundary answers the other
    half. An entry from a build that predates the boundary column cannot
    express expiry, so it is re-validated once rather than trusted.
    """
    if tuple(cached[:2]) != current:
        return False
    if kind != "memory":
        return True
    boundary = cached[2] if len(cached) > 2 else None
    if boundary is None:
        return False
    try:
        return date.today() <= date.fromisoformat(boundary)
    except (TypeError, ValueError):
        return False


def discover_documents(
    repository: Path,
    reusable: Optional[SourceState] = None,
    candidates: Optional[list[SourceCandidate]] = None,
    screened_hashes: Optional[dict[str, str]] = None,
) -> tuple[
    list[DocumentRow], SourceState, SourceState, list[dict[str, str]], dict[str, tuple[str, int]]
]:
    # Thousands of chunks citing the same files: each is looked at once.
    with source_fact_memo():
        return _discover_documents(repository, reusable, candidates, screened_hashes)


def _discover_documents(
    repository: Path,
    reusable: Optional[SourceState] = None,
    candidates: Optional[list[SourceCandidate]] = None,
    screened_hashes: Optional[dict[str, str]] = None,
) -> tuple[
    list[DocumentRow], SourceState, SourceState, list[dict[str, str]], dict[str, tuple[str, int]]
]:
    """Return changed documents, the retained cache subset, the new cache,
    every candidate dropped along the way with the reason it was dropped, and
    the documents indexed with masked values: path -> (digest of the file as
    read, values masked).

    ``reusable`` maps already-indexed paths to the (mtime_ns, size, boundary)
    recorded by the last successful index. Ordinary documents whose stat still
    matches are not re-read. Memory chunks still revalidate current eligibility:
    their cited paths can disappear or escape while the chunk stays unchanged.
    This costs chunk parsing and secret scanning, but preserves the existing
    citation contract without inventing historical source hashes. A modification
    that somehow reuses its predecessor's stat is still not served as truth — governed retrieval
    re-hashes every candidate and excludes the mismatch as stale.

    Only files a reader could reasonably expect to find in the index are
    reported as excluded. The three structural skips below - git-ignored
    files, a second pattern claiming a file the first already owns, and a
    mirrored copy of an indexed skill - are how discovery is defined rather
    than something that went wrong, and Symfony alone would contribute 279 of
    the third.

    ``candidates`` accepts an already-collected source walk so a caller that
    also needs the stats elsewhere (the skill-tree fingerprint) pays for the
    filesystem pass once.
    """
    documents: list[DocumentRow] = []
    retained: SourceState = {}
    state: SourceState = {}
    excluded: list[dict[str, str]] = []
    masked: dict[str, tuple[str, int]] = {}
    skill_keys: set[str] = set()
    claimed: set[str] = set()
    if candidates is None:
        candidates = collect_source_candidates(repository, excluded=excluded)
    else:
        admitted = []
        rejected = set()
        for candidate in candidates:
            problem = _invalid_source_key(candidate[3])
            if problem is None:
                admitted.append(candidate)
            elif candidate[1] == 'skill':
                raise RetrievalError('Skill source invalid-utf8-path')
            elif problem['path'] not in rejected:
                excluded.append(problem)
                rejected.add(problem['path'])
        candidates = admitted

    cache = reusable or {}
    ignored_paths = git_ignored_paths(
        repository, [relative_path for _, _, _, relative_path, _, _ in candidates]
    )
    seen_by_pattern: dict[str, int] = {}
    ignored_by_pattern: dict[str, int] = {}
    for layer, kind, path, relative_path, current, pattern in candidates:
        seen_by_pattern[pattern] = seen_by_pattern.get(pattern, 0) + 1
        if os.fsencode(relative_path) in ignored_paths:
            ignored_by_pattern[pattern] = ignored_by_pattern.get(pattern, 0) + 1
            continue
        if relative_path in claimed:
            # Two patterns can match one file, and the earlier one owns its
            # layer and kind. Without this the document is inserted twice and
            # the metadata primary key aborts the whole refresh.
            continue
        claimed.add(relative_path)
        skill_key = skill_key_parts(relative_path)[1] if kind == "skill" else None
        if skill_key is not None and skill_key in skill_keys:
            # A mirrored copy of an already-indexed skill. Only an indexed copy
            # claims the key, so a later copy can never win it and never needs
            # to be read or scanned.
            continue
        cached = cache.get(relative_path)
        if cached is not None and _cache_entry_is_current(kind, cached, current):
            if kind == "memory":
                try:
                    metadata, reason, boundary = memory_eligibility(
                        path, repository, screened_hash=(screened_hashes or {}).get(relative_path))
                except UnicodeDecodeError:
                    excluded.append({"path": relative_path, "reason": "invalid-utf8"})
                    continue
                if metadata is None:
                    excluded.append({"path": relative_path, "reason": reason or "invalid"})
                    continue
                cached = (current[0], current[1], boundary)
            # Unchanged since the last successful index and still inside its
            # calendar boundary; memory passed current citation eligibility
            # above. Keep the existing indexed row and its original digests.
            if skill_key is not None:
                skill_keys.add(skill_key)
            retained[relative_path] = cached
            state[relative_path] = cached
            continue
        boundary: Optional[str] = None
        try:
            if kind == "memory":
                # A chunk with a likely secret stays out whole: it is shared
                # knowledge the team wrote, not a file the project ships.
                metadata, reason, boundary = memory_eligibility(path, repository)
                if metadata is None:
                    excluded.append(
                        {"path": relative_path, "reason": reason or "invalid"}
                    )
                    continue
                content = path.read_text(encoding="utf-8")
            else:
                # Masked, not dropped: one placeholder-shaped development
                # credential in a code block used to take a whole README - the
                # only useful document of seven graded prompts - out of the
                # index. The value never reaches the index; the rest does.
                try:
                    raw = path.read_text(encoding="utf-8")
                    content, values = mask_secrets(raw)
                except (OSError, ValidationError):
                    excluded.append({"path": relative_path, "reason": "secret"})
                    continue
                if values:
                    masked[relative_path] = (_content_hash(raw), values)
        except UnicodeDecodeError:
            # A bad document must not disable every otherwise valid source.
            # It is absent from retained/state, so its previous indexed row is
            # removed by the ordinary refresh instead of silently reused.
            excluded.append({"path": relative_path, "reason": "invalid-utf8"})
            continue
        # Portable snapshots may contain an explicitly exported conversation.
        # Only context-load may return them: general retrieval must never ingest
        # their historical instructions, even after a snapshot is renamed.
        if content.startswith("---\n"):
            try:
                snapshot = json.loads(content[4:].split("\n---\n", 1)[0])
            except (ValueError, json.JSONDecodeError):
                snapshot = None
            if isinstance(snapshot, dict) and snapshot.get("type") == "context-handoff":
                excluded.append({"path": relative_path, "reason": "explicit-context-only"})
                continue
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
        # detected next time instead of being cached away. The boundary rides
        # along so the next incremental pass can retire the row on its date
        # without opening the file.
        state[relative_path] = (current[0], current[1], boundary)
    # A whole pattern going dark is the one git-ignore case worth reporting.
    # Individual ignored files are deliberately silent — Symfony alone would
    # contribute hundreds — but a glob that matched files on disk and lost
    # every one of them means a documented source of truth is invisible and
    # nothing else would say so. Measured on a real installation: a project
    # whose own .gitignore carried a bare `docs` entry indexed 96 accelerator
    # skills, one README, and none of its own design documents.
    for pattern, total in sorted(seen_by_pattern.items()):
        if total and ignored_by_pattern.get(pattern, 0) == total:
            excluded.append(
                {"path": pattern, "reason": "pattern-all-git-ignored"}
            )
    return documents, retained, state, excluded, masked


def _report_masked(result: dict[str, object], masked: dict[str, tuple[str, int]]) -> None:
    """Which documents read on this pass were indexed with masked values, and
    how many: never a value, never its label."""
    result["redacted"] = [
        {"path": path, "values": values} for path, (_, values) in sorted(masked.items())
    ]


def _merge_excluded(result: dict[str, object], dropped: list[dict[str, str]]) -> None:
    """Fold discovery's exclusions into the one channel the index reports.

    Project Brain records are excluded inside ``index_documents``; repository
    documents are excluded during discovery, one call earlier. Merging here
    rather than threading the list through ``index_documents`` keeps that
    function's signature and every existing caller untouched, and it has to
    happen on both of ``index_documents``' return paths - including the
    "nothing observable changed" early return, which is the one the per-prompt
    hook takes.
    """
    if not dropped:
        return
    existing = result.get("excluded")
    result["excluded"] = [
        *(existing if isinstance(existing, list) else []),
        *dropped,
    ]


SOURCE_NOTICE_LABELS = {'invalid-utf8': 'invalid UTF-8 content',
                        'invalid-utf8-path': 'invalid UTF-8 paths'}


def _normalized_source_notices(value):
    return {key: value[key] for key in SOURCE_NOTICE_LABELS
            if isinstance(value, dict) and type(value.get(key)) is int and value[key] > 0}


def _record_source_notices(result, excluded):
    # Only ordinary discovery contributes: protected Brain-record exclusions
    # and arbitrary reasons never become prompt-visible aggregate counts.
    counts = {key: sum(item.get('reason') == key for item in excluded)
              for key in SOURCE_NOTICE_LABELS}
    counts = _normalized_source_notices(counts)
    if counts:
        result['source_notices'] = counts


def _source_notice_text(value):
    counts = _normalized_source_notices(value)
    return ('sources omitted: '+ '; '.join(SOURCE_NOTICE_LABELS[key]+' '+str(count)
            for key, count in counts.items())) if counts else ''


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
    candidate_dropped = []
    candidates = collect_source_candidates(repository, excluded=candidate_dropped)
    # The candidate walk already statted every mirrored skill file, so the
    # skill-tree fingerprint is derived from it instead of a second tree walk.
    skill_stats = [
        (*skill_key_parts(relative), mtime_ns, size)
        for _, kind, _, relative, (mtime_ns, size), _ in candidates
        if kind == "skill"
    ]
    fingerprints = index_fingerprints(repository, skill_stats)
    if incremental:
        reusable, fingerprints = reusable_source_state(
            connection, repository, fingerprints
        )
        # The hash each retained chunk was screened and indexed with.
        screened = {
            str(row[0]): str(row[1])
            for row in connection.execute("SELECT path, source_hash FROM document_metadata")
            if row[0] in reusable and row[1]
        }
        documents, retained, state, dropped, masked = discover_documents(
            repository, reusable, candidates=candidates, screened_hashes=screened
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
                source_hashes={path: digest for path, (digest, _) in masked.items()},
            )
        except RetrievalError:
            # The cache disagreed with the index; fall through and rebuild.
            phase_started = time.monotonic()
        else:
            _merge_excluded(result, [*candidate_dropped, *dropped])
            _record_source_notices(result, [*candidate_dropped, *dropped])
            _report_masked(result, masked)
            result["phase_seconds"] = {
                "stat": round(scan_seconds, 6),
                "index": round(time.monotonic() - index_started, 6),
            }
            return result
    documents, _, state, dropped, masked = discover_documents(
        repository, candidates=candidates
    )
    scan_seconds += time.monotonic() - phase_started
    index_started = time.monotonic()
    result = index_documents(
        connection, repository, documents, source_state=state,
        fingerprints=fingerprints,
        source_hashes={path: digest for path, (digest, _) in masked.items()},
    )
    _merge_excluded(result, [*candidate_dropped, *dropped])
    _record_source_notices(result, [*candidate_dropped, *dropped])
    _report_masked(result, masked)
    result["phase_seconds"] = {
        "stat": round(scan_seconds, 6),
        "index": round(time.monotonic() - index_started, 6),
    }
    return result


def search_documents(
    connection: sqlite3.Connection,
    query: str,
    limit: int,
    layer: Optional[str] = None,
    relevant_only: bool = False,
    strong_only: bool = False,
    *, repository: Optional[Path] = None, _image_sink=None,
) -> list[dict[str, object]]:
    """Search the index.

    ``relevant_only`` drops corpus-common terms and requires a document to
    contain more than one query term. Capsule assembly uses it, because a
    document that shares a single common word with the request is noise
    presented as context. Explicit `search` stays broad by design.

    On the ``relevant_only`` path each returned item also carries how it
    qualified, via `match_strength`: a document admitted only by a single rare
    term is a far weaker answer than one that covered the query, and until the
    capsule says so the two are indistinguishable inside a turn.
    """
    coverage: dict[str, int] = {}
    distinctive: dict[str, list[str]] = {}
    minimum = 0
    if relevant_only:
        tokens = informative_tokens(
            connection, expand_query_tokens(connection, query_tokens(query))
        )
        # Broad skill discovery keeps its routing contract; automatic capsules
        # use strong_only and must ignore acknowledgements/intent-only terms.
        if layer != "procedural" or strong_only:
            tokens = evidence_tokens(tokens)
        if not tokens:
            return []
        coverage, distinctive = token_coverage(connection, tokens)
        minimum = required_coverage(tokens)
        match_expression = " OR ".join(f'"{token}"' for token in tokens)
    else:
        match_expression = fts_query_tokens(
            expand_query_tokens(connection, query_tokens(query))
        )
    conditions = ["documents MATCH ?"]
    parameters: list[object] = [match_expression]
    if layer is not None:
        conditions.append("layer = ?")
        parameters.append(layer)
    parameters.append(limit * 10 if relevant_only or repository is not None else limit)
    markers = ("", "") if relevant_only else ("[", "]")
    policy_columns = ', summary AS _policy_summary, content AS _policy_content' if repository is not None else ''
    rows = connection.execute(
        f"""
        SELECT
            path,
            layer,
            kind,
            title,
            snippet(documents, 5, ?, ?, ' … ', 18) AS snippet
            {policy_columns}
        FROM documents
        WHERE {' AND '.join(conditions)}
        ORDER BY bm25(documents), path
        LIMIT ?
        """,
        (*markers, *parameters),
    ).fetchall()
    selected = [
        row
        for row in rows
        if is_relevant(row["path"], coverage, distinctive, minimum, str(row["title"] or ""),
                       anchored=row["layer"] != "procedural")
        and not (strong_only and coverage.get(row["path"], 0) < minimum)
    ] if relevant_only else rows
    if repository is not None:
        policy = _CurrentSourcePolicy(connection, repository)
        # These full unhighlighted images come from the same statement as the
        # returned snippet. FTS highlighting may split a newly sensitive token.
        policy.prime(selected, indexed_images={row['path']: tuple(str(row[key] or '')
                     for key in ('title','_policy_summary','_policy_content')) for row in selected})
        selected = [row for row in selected if policy.reason(row) is None]
        if _image_sink is not None:
            for row in selected[:limit]:
                _image_sink.setdefault(row['path'], policy.indexed_images[row['path']])
    # Extra admission columns are private and never extend the public schema.
    items = [{key: row[key] for key in ('path','layer','kind','title','snippet')} for row in selected[:limit]]
    if relevant_only:
        for item in items:
            strength = match_strength(str(item["path"]), coverage, minimum)
            if strength != "covered":
                item["match"] = strength
    return items


def search_episodes(
    connection: sqlite3.Connection,
    query: str,
    limit: int,
) -> list[dict[str, object]]:
    if not evidence_tokens(query_tokens(query)):
        return []
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
        (fts_query_tokens(expand_query_tokens(connection, query_tokens(query))), limit),
    ).fetchall()
    return [episode_item(row) for row in rows]


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
    Instead the whole prompt is tokenized, terms the searchable corpus has
    never seen or that match most of it are dropped — the same adaptive
    stop-word rule retrieval itself applies — and the rarest terms win the
    slots. Rarity in this repository's own index decides relevance, not
    position.

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
        document_total, episode_total = connection.execute(
            "SELECT (SELECT COUNT(*) FROM documents), "
            "(SELECT COUNT(*) FROM episodes)"
        ).fetchone()
        total = document_total + episode_total
    except sqlite3.Error:
        total = 0
    if not total:
        return " ".join(tokens[:CAPSULE_PROMPT_TERM_LIMIT])
    frequencies = token_document_frequencies(connection, tokens)
    if episode_total:
        for token in tokens:
            episode_count = connection.execute(
                "SELECT COUNT(*) FROM episodes WHERE episodes MATCH ?",
                (f'"{token}"',),
            ).fetchone()[0]
            frequencies[token] = (frequencies.get(token) or 0) + episode_count
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


def sanitize_automatic_query(text: str) -> str:
    """The prompt an automatic path searches with (`automatic_query`)."""
    try:
        return _sanitize_automatic_query(text)
    except ValidationError as error:
        raise ContextError(str(error)) from error


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


# Words a reply made only of is an acknowledgement, never a new subject.
# Closed on purpose: one content word ("okay quartz") makes it a real prompt.
_CONFIRMATION_WORDS = frozenset({
    "yes", "yeah", "yep", "yup", "ok", "okay", "k", "sure", "please", "thanks", "thank",
    "you", "go", "on", "ahead", "continue", "proceed", "do", "it", "that", "this",
    "next", "keep", "going", "carry", "sounds", "good", "great", "fine", "perfect",
    "lgtm", "alright", "right", "cool", "nice", "and", "then", "now", "so", "let's",
    "lets", "why", "fix",
    "да", "ок", "окей", "ага", "угу", "хорошо", "отлично", "давай", "давайте", "дальше",
    "продолжай", "продолжи", "продолжайте", "делай", "сделай", "делайте", "так", "и",
    "спасибо", "верно", "согласен", "согласна", "поехали", "вперёд", "вперед", "можно",
    "ну", "пожалуйста", "это", "ладно", "годится", "норм", "супер", "почему", "исправь",
})


_TASK_FALLBACK_ACKNOWLEDGEMENTS = frozenset({
    "continue", "go on", "ok", "okay", "thanks", "thank you",
    "ok thanks", "okay thanks", "ok go on",
    "дальше", "продолжай", "продолжай дальше", "хорошо", "спасибо",
})
# A short continuation may borrow the task only when it names an action. The
# acknowledgement set alone is not enough: `_CONFIRMATION_WORDS` also accepts
# "sure", "go ahead", "please continue" and "yes", none of which is a request
# to do anything. The action verbs are deliberately small and explicit; an
# unknown short phrase gets no memory rather than unsolicited context.
_TASK_FALLBACK_ACTIONS = frozenset({
    "fix", "do", "implement", "update", "change", "add", "remove", "refactor",
    "test", "run", "check", "review", "apply", "finish",
    "исправь", "сделай", "делай", "добавь", "обнови", "измени", "проверь",
    "запусти", "реализуй", "поправь", "заверши",
})


def _task_fallback_prompt(text: str) -> bool:
    """Whether a short prompt may borrow the task's terms for retrieval.

    A bare acknowledgement is not a new request: the governed capsule has
    never injected memory for "continue", and a touch link admitted only by
    the task's words would be exactly the unsolicited context that rule
    excludes. An imperative continuation ("fix it", "do it") does name an
    action, so the task it continues is the only place its subject can be
    read.
    """
    phrase = re.sub(r"[\s,.!?;]+", " ", text.casefold()).strip()
    if phrase in _TASK_FALLBACK_ACKNOWLEDGEMENTS:
        return False
    words = phrase.split()
    if not words or len(words) > 6:
        return False
    if any(
        word not in _TASK_FALLBACK_ACTIONS and word not in _CONFIRMATION_WORDS
        for word in words
    ):
        # A phrase with a subject of its own ("fix the bug", "update the
        # rule") is not a continuation; retrieval already has its words.
        return False
    return any(word in _TASK_FALLBACK_ACTIONS for word in words)


def _cursor_continuation_prompt(text: str, *, persistent: bool = False) -> bool:
    """A bounded explicit continuation, never a generic lack-of-evidence test."""
    if len(text) > 128:
        return False
    phrase = re.sub(r"[\s,.!?;]+", " ", text.casefold()).strip()
    ordinary = {
        "go on", "continue", "ok", "okay", "thanks", "thank you",
        "ok thanks", "okay thanks", "ok go on",
        "дальше", "продолжай", "продолжай дальше", "хорошо", "спасибо",
    }
    if phrase in ordinary:
        return True
    if not persistent:
        return False
    # The Cursor rule is re-sent with every request, so a short reply that
    # only acknowledges ("sure", "go ahead", "давай", "ок, делай") keeps the
    # topic it answers instead of replacing the rule's knowledge with none.
    words = phrase.split()
    return 0 < len(words) <= 6 and all(word in _CONFIRMATION_WORDS for word in words)


def validate_direct_query_request(arguments: argparse.Namespace) -> None:
    """Reject unsafe direct queries before opening or mutating local state.

    A query a person typed is refused whole, so they see what was wrong. The
    automatic paths pass `refresh --sanitize`, and their prompt is cleaned
    instead (`sanitize_automatic_query`); what remains is held to the same
    checks. A prompt with nothing left to search is withheld, not replaced
    by the task goal or the branch name.
    """
    if arguments.command in {"context", "retrieve"}:
        query = arguments.query
    elif arguments.command == "refresh":
        query = arguments.query
        if query is not None and getattr(arguments, "sanitize", False):
            original = query
            query = arguments.query = sanitize_automatic_query(query)
            # Capture eligibility before replacing the raw prompt. Redaction
            # must never turn a sensitive request into a task-goal lookup.
            arguments.cursor_continuation = (
                getattr(arguments, "host", None) == "cursor"
                and query == original.strip(" \t\r\n,;")
                and _cursor_continuation_prompt(original,
                    persistent=getattr(arguments, 'delivery_profile', 'ordinary') == 'cursor-rule')
            )
            arguments.cursor_topic_eligible = (
                getattr(arguments, 'delivery_profile', 'ordinary') == 'cursor-rule'
                and query == original.strip(" \t\r\n,;") and len(original) <= 1024
                and original.casefold().strip(' \t\r\n,.!?;') not in
                    {'do not continue', 'не продолжай', 'стоп', 'stop'}
            )
            # Nothing left, or nothing a search can use: a reply of "?" or
            # an emoji passed the sanitizer, failed the word check later, and
            # put the operator's layer report and "Capsule unavailable" into
            # the prompt of the turn that approves a plan.
            if re.search(r"\w", query) is None:
                arguments.query = None
                arguments.query_withheld = True
                return
    else:
        return
    if query is None:
        return
    reject_secrets("Task Capsule", [query])
    reject_capsule_privacy("Task Capsule request", [query])


def project_working_task(
    working: Optional[dict[str, object]],
    *, delivery_profile: str = 'ordinary',
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
    displayed = progress if delivery_profile == 'cursor-rule' else render_current_state(progress, checkpoint)

    normalized_steps = [" ".join(item.split()) for item in next_steps]
    omitted["working_files"] = max(
        0, len(files) - CAPSULE_WORKING_FILE_LIMIT
    )
    omitted["working_next_steps"] = max(
        0, len(normalized_steps) - CAPSULE_WORKING_NEXT_STEP_LIMIT
    )
    omitted["working_sources"] = max(
        0, len(sources) - CAPSULE_WORKING_SOURCE_LIMIT
    )
    return {
        "task_id": validate_task_id(task_id),
        "goal": " ".join(goal.split()),
        "progress": " ".join(displayed.split()),
        "next_steps": normalized_steps[-CAPSULE_WORKING_NEXT_STEP_LIMIT:],
        # Newest last, so the tail is the work in hand.
        "files": files[-CAPSULE_WORKING_FILE_LIMIT:],
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


def deduplicate_capsule_layers(capsule: dict[str, object]) -> None:
    """Drop any item that a second layer of the same capsule already holds.

    The governed capsule is assembled from two different taxonomies - the
    semantic layer from retrieval categories, the episodic layer from the
    layer column - which is how one document can occupy a slot in each. The
    layer split has to survive, so this is a layer-preserving pass rather than
    `deduplicate_context_items` over the concatenation: the governed contract
    enforces per-layer limits afterwards and needs three lists, not one.

    A document is claimed by the capsule layer its own ``layer`` field names,
    whichever list happened to see it first; only an item with no such home -
    a recorded episode, which has an id rather than a path and no layer -
    falls back to first-come priority. Resolving by ownership rather than by
    order is what keeps the episodic slot from being emptied by a semantic
    copy of the one document that belongs in it.

    Today the retrieval side already excludes episodic documents from the
    semantic ranking, which is where the freed slot gets reallocated to the
    next candidate, so this pass removes nothing. It exists so that a future
    layer source cannot reintroduce the collision unnoticed.
    """
    layers = ("procedural", "semantic", "episodic")

    def key(item: dict[str, object]) -> tuple[str, object]:
        return ("path", item["path"]) if "path" in item else ("episode", item["id"])

    holder: dict[tuple[str, object], str] = {}
    for layer in layers:
        for item in capsule.get(layer) or []:
            if item.get("layer") == layer:
                holder[key(item)] = layer
    for layer in layers:
        for item in capsule.get(layer) or []:
            holder.setdefault(key(item), layer)
    for layer in layers:
        items = capsule.get(layer)
        if isinstance(items, list):
            capsule[layer] = [item for item in items if holder[key(item)] == layer]


def build_context_packet(
    connection: sqlite3.Connection,
    repository: Path,
    query: str,
    task_id: Optional[str],
    limit: int,
    include_retrieval: bool = True,
    warnings: Optional[list[str]] = None,
    *, delivery_profile: str = 'ordinary', query_source: str = 'explicit',
) -> dict[str, object]:
    if limit < 1:
        raise ContextError("--limit must be a positive integer")
    reject_secrets("Task Capsule", [query])
    reject_capsule_privacy("Task Capsule request", [query])

    capsule_warnings = list(warnings or [])
    raw_working = find_working_task(connection, task_id)
    working, omitted = project_working_task(raw_working, delivery_profile=delivery_profile)
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
    if not include_retrieval or not evidence_tokens(query_tokens(request_query)):
        return packet

    retrieval_query = build_capsule_query(request_query, working)
    procedural_limit = min(limit, CAPSULE_LAYER_LIMITS["procedural"])
    admitted_images = {}
    semantic_limit = min(limit, CAPSULE_LAYER_LIMITS["semantic"])
    episodic_limit = min(limit, CAPSULE_EPISODIC_LIMIT)  # lightweight: no history search
    procedural = search_documents(
        connection, request_query, procedural_limit, "procedural",
        relevant_only=True, strong_only=True, repository=repository, _image_sink=admitted_images,
    )
    semantic = search_documents(
        connection, request_query, semantic_limit, "semantic",
        relevant_only=True, repository=repository, _image_sink=admitted_images,
    )
    episodic = search_documents(
        connection, request_query, episodic_limit, "episodic",
        relevant_only=True, repository=repository, _image_sink=admitted_images,
    ) + search_episodes(connection, request_query, episodic_limit)
    if retrieval_query != request_query:
        if len(deduplicate_context_items(procedural)) < procedural_limit:
            procedural += search_documents(
                connection, retrieval_query, procedural_limit, "procedural",
                relevant_only=True, strong_only=True, repository=repository, _image_sink=admitted_images,
            )
        if len(deduplicate_context_items(semantic)) < semantic_limit:
            semantic += search_documents(
                connection, retrieval_query, semantic_limit, "semantic",
                relevant_only=True, repository=repository, _image_sink=admitted_images,
            )
        if len(deduplicate_context_items(episodic)) < episodic_limit:
            episodic += search_documents(
                connection, retrieval_query, episodic_limit, "episodic",
                relevant_only=True, repository=repository, _image_sink=admitted_images,
            ) + search_episodes(connection, retrieval_query, episodic_limit)
    # Recorded from what survived the relevance test, before the per-layer
    # truncation below: a layer that is short because the limit cut it is not
    # a layer memory had nothing for.
    packet["no_match"] = [
        layer
        for layer, found in (
            ("procedural", procedural),
            ("semantic", semantic),
            ("episodic", episodic),
        )
        if not found
    ]
    semantic += source_link_candidates(
        connection, repository, semantic,
        {str(item["path"]) for item in semantic},
        _image_sink=admitted_images,
    )
    # The governed capsule's rule: a skill's sub-file vacates the slot rather
    # than handing it to a weaker skill.
    # The governed capsule's rule: hosts list their skills and load their
    # instruction files themselves, so no procedural item is carried. The
    # search above still runs, for `no_match`.
    packet["procedural"] = []
    packet["semantic"] = deduplicate_context_items(semantic)[:semantic_limit]
    packet["episodic"] = deduplicate_context_items(episodic)[:episodic_limit]
    return _with_index_images(packet, admitted_images)


def serialize_capsule(capsule: dict[str, object]) -> str:
    return json.dumps(capsule, ensure_ascii=False, separators=(",", ":"))


def capsule_character_count(capsule: dict[str, object]) -> int:
    return len(serialize_capsule(capsule))


def truncate_progress(progress: str, keep: int) -> str:
    """Cut a progress narrative to ``keep`` characters from both ends.

    A narrative opens with what the work is bound by and ends with where it
    now stands. Keeping only the tail preserves the action and discards the
    reason for it, which is what lets a later turn "optimize" away a decision
    whose justification the capsule no longer carries.
    """
    if keep >= len(progress):
        return progress
    head = keep // 2
    tail = keep - head
    return f"{progress[:head]}…{progress[-tail:]}"


# Capsule sections the budget may remove, in the order a report names them.
# A capsule that silently lost a constraint reads exactly like a complete
# one, so these counters have to reach the prompt, not only the JSON. The
# goal and next-step counts exist only in the rendered text, which bounds
# lines the JSON carries whole.
COMPACTION_LABELS = (
    ("procedural", "procedural result(s)"),
    ("semantic", "semantic result(s)"),
    ("episodic", "episodic result(s)"),
    ("working_files", "working file(s)"),
    ("working_sources", "working source(s)"),
    ("working_next_steps", "next step(s)"),
    ("working_goal_characters", "characters of the goal"),
    ("working_progress_characters", "characters of progress"),
    ("working_next_step_characters", "characters of next steps"),
    ("last_turn_characters", "characters of the last-turn report"),
)
COMPACTION_ADVICE = (
    "re-read the cited source before revising a decision it no longer explains"
)
# What the compaction line keeps when the rendered ceiling has no room for its
# counts. It is never dropped: the line used to yield whole, so a capsule cut
# at the ceiling - exactly when the cut is tightest - read as a complete one.
COMPACTION_MARKER = f"compaction: lossy view — {COMPACTION_ADVICE}"


def compaction_summary(
    capsule: dict[str, object], rendered: Optional[dict[str, int]] = None
) -> Optional[str]:
    """Name what this capsule no longer shows, or None when it shows all.

    ``rendered`` is what the rendered text cuts besides: the JSON carries a
    working state the text bounds again, and the text is what a host hands to
    the model. Each count is added to the capsule's own ``omitted`` count, so
    a task's files are reported once - as all the ones the text leaves out.
    """
    counts: dict[str, int] = {}
    omitted = capsule.get("omitted")
    for source in (omitted if isinstance(omitted, dict) else {}, rendered or {}):
        for key, value in source.items():
            if isinstance(value, int) and not isinstance(value, bool):
                counts[key] = counts.get(key, 0) + value
    parts = [
        f"{counts[key]} {label}"
        for key, label in COMPACTION_LABELS
        if counts.get(key, 0) > 0
    ]
    if not parts:
        return None
    return ", ".join(parts)


def enforce_capsule_budget(capsule: dict[str, object]) -> dict[str, object]:
    compacted = json.loads(serialize_capsule(capsule))
    omitted = compacted["omitted"]
    for key in (
        "working_files",
        "working_next_steps",
        "working_sources",
        "working_progress_characters",
        "last_turn_characters",
        "semantic",
        "episodic",
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
            omitted["episodic"] += 1
            continue
        if compacted["semantic"]:
            compacted["semantic"].pop()
            omitted["semantic"] += 1
            continue
        if working is not None and len(working["sources"]) > 1:
            working["sources"].pop()
            omitted["working_sources"] += 1
            continue
        if working is not None and len(working["files"]) > 1:
            # Oldest first: the newest file is the work in hand.
            working["files"].pop(0)
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
            working["progress"] = truncate_progress(working["progress"], keep)
            continue
        raise ContextError(
            "mandatory Task Capsule content exceeds "
            f"{CAPSULE_CHARACTER_LIMIT} characters"
        )

    return _carry_index_images(capsule, compacted)


def synchronize_capsule_views(capsule: dict[str, object]) -> None:
    """Hold `selected` and `categories` to the documents the layers carry."""
    selected_paths = {
        item["path"]
        for layer in DOCUMENT_LAYERS
        for item in capsule.get(layer) or []
        if isinstance(item, dict) and isinstance(item.get("path"), str)
    }
    capsule["selected"] = [
        item
        for item in capsule.get("selected", [])
        if isinstance(item, dict) and item.get("path") in selected_paths
    ]
    categories = capsule.get("categories")
    if isinstance(categories, dict):
        for category, items in categories.items():
            categories[category] = [
                item
                for item in items
                if isinstance(item, dict) and item.get("path") in selected_paths
            ]


def enforce_governed_capsule_contract(
    capsule: dict[str, object],
) -> dict[str, object]:
    """Apply the shared layer limits and 8,000-character contract to governed
    output: no procedural item reaches it (see retrieve), 3 semantic, the
    changelog plus two event/episode slots."""
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
    # Merge rather than replace: the working-state counters were computed
    # when the task was projected, and a governed capsule drops working
    # content of its own below. Losing them here would report a partial
    # capsule as a complete one.
    omitted = dict(compacted.get("omitted") or {})
    omitted.update(
        {
            "procedural": max(
                0,
                len(capsule.get("procedural", []))
                - CAPSULE_LAYER_LIMITS["procedural"],
            ),
            "semantic": max(
                0,
                len(capsule.get("semantic", []))
                - CAPSULE_LAYER_LIMITS["semantic"],
            ),
            "episodic": max(
                0,
                len(capsule.get("episodic", []))
                - CAPSULE_LAYER_LIMITS["episodic"],
            ),
        }
    )
    for key in ("working_progress_characters", "last_turn_characters"):
        omitted.setdefault(key, 0)
    compacted["omitted"] = omitted

    working = compacted.get("working")
    if isinstance(working, dict):
        next_steps = list(working.get("next_steps", []))
        files = list(working.get("files", []))
        sources = list(working.get("sources", []))
        working["next_steps"] = next_steps[-CAPSULE_WORKING_NEXT_STEP_LIMIT:]
        # A task's files are kept newest last; the head of the list is the
        # first thing the branch ever touched, not the work in hand.
        working["files"] = files[-CAPSULE_WORKING_FILE_LIMIT:]
        working["sources"] = sources[:CAPSULE_WORKING_SOURCE_LIMIT]
        # `omitted` is the same object stored above, so these land in the
        # capsule: the projection limits hide governed working state exactly
        # as the lightweight projection does, and hiding it unreported is
        # what makes a partial view read as the whole task. Added to rather
        # than replaced: a working state that arrives already projected
        # carries its own counts and is within the limits here.
        for key, before, after in (
            ("working_next_steps", next_steps, working["next_steps"]),
            ("working_files", files, working["files"]),
            ("working_sources", sources, working["sources"]),
        ):
            omitted[key] = int(omitted.get(key) or 0) + len(before) - len(after)

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
    synchronize_capsule_views(compacted)

    while capsule_character_count(compacted) > CAPSULE_CHARACTER_LIMIT:
        hints = [index for index, item in enumerate(compacted["semantic"])
                 if item.get("selection") == "source-hint"]
        if hints:
            compacted["semantic"].pop(hints[-1])
            compacted["omitted"]["semantic"] += 1
            synchronize_capsule_views(compacted)
            continue
        if compacted.get("last_turn"):
            omitted["last_turn_characters"] += len(compacted["last_turn"])
            compacted["last_turn"] = None
            continue
        related = _related_positions(compacted["semantic"])
        if related:
            # Related knowledge fills capacity left after direct matches, so it
            # is the first to give it back - before history the query found.
            compacted["semantic"].pop(related[-1])
            compacted["omitted"]["semantic"] += 1
            synchronize_capsule_views(compacted)
            continue
        dropped = False
        for layer in ("episodic", "semantic", "procedural"):
            if compacted[layer]:
                compacted[layer].pop()
                compacted["omitted"][layer] += 1
                synchronize_capsule_views(compacted)
                dropped = True
                break
        if dropped:
            continue
        if isinstance(working, dict) and working.get("progress"):
            excess = capsule_character_count(compacted) - CAPSULE_CHARACTER_LIMIT
            keep = max(0, len(working["progress"]) - excess - 1)
            if keep:
                omitted["working_progress_characters"] += (
                    len(working["progress"]) - keep
                )
                working["progress"] = truncate_progress(working["progress"], keep)
                continue
        raise ContextError(
            "mandatory Task Capsule content exceeds "
            f"{CAPSULE_CHARACTER_LIMIT} characters"
        )
    return _carry_index_images(capsule, compacted)


RELATED_SELECTIONS = ("source-link", *AUTOMATIC_LINK_SELECTIONS, "source-hint")


def _related_positions(items: object) -> list[int]:
    """Positions of items a relation brought in rather than the query."""
    return [
        index for index, item in enumerate(items if isinstance(items, list) else [])
        if isinstance(item, dict) and item.get("selection") in RELATED_SELECTIONS
    ]


def validate_task_id(task_id: str) -> str:
    normalized = task_id.strip()
    if TASK_ID_PATTERN.fullmatch(normalized) is None:
        raise ContextError("Task ID must use letters, digits, '.', '_', '/', '#', '+' or '-'")
    reject_secrets("working task", [normalized])
    return normalized


def branch_task_id(branch: str) -> str:
    """The task ID of a branch, also of one whose name the grammar refuses.

    Git takes names a task ID does not - an apostrophe, `@`, parentheses,
    more than 128 characters - and such a branch had no memory at all: every
    prompt said the task ID was invalid and no turn was recorded. Each run of
    refused characters becomes `-`, the name is cut to fit, and eight hex
    digits of the whole name's SHA-256 keep two such branches apart. A name
    the grammar takes is its own task ID.
    """
    if TASK_ID_PATTERN.fullmatch(branch):
        return branch
    digest = hashlib.sha256(branch.encode("utf-8", "surrogatepass")).hexdigest()[:8]
    stem = re.sub(r"[^\w\u0300-\u036f./#+-]+", "-", branch)
    stem = re.sub(r"^[\W_]+", "", stem)[:119].rstrip("-")
    return (stem or "branch") + "-" + digest


def _on_branch(repository: Path, branch: str) -> bool:
    """Whether the project's checkout is on ``branch`` now, or rebasing it."""
    try:
        project = workspace_roots.project_root(repository.resolve())
        observed = _source_git_run(project, ["symbolic-ref", "--quiet", "HEAD"], _source_git_environment(),
                                   time.monotonic() + 2.0)
        if observed.returncode == 1:
            return _rebased_branch(project) == branch
        return (observed.returncode == 0 and isinstance(observed.stdout, bytes)
                and observed.stdout == ("refs/heads/" + branch + "\n").encode("utf-8", "surrogatepass"))
    except (SourceGitError, OSError, UnicodeError, ValueError):
        return False


# The commands the hooks run with the checkout's branch as the task ID.
_BRANCH_IDENTITY_COMMANDS = ("refresh", "hook-context", "turn", "cursor-rule", "msg-dispatch")
_RENAMED = re.compile(r"[^\t]*\tBranch: renamed refs/heads/(.+) to refs/heads/(.+)")


def _renamed_from(repository: Path, branch: str) -> list[str]:
    """The names ``branch`` had before `git branch -m`, latest first.

    Git moves a branch's reflog with it and adds "Branch: renamed
    refs/heads/<old> to refs/heads/<new>"; the log is read as a file, without
    starting Git, as most branches were never renamed. A reftable repository
    keeps no such file, and a branch there keeps no earlier name here.
    """
    try:
        checkout = _source_git_checkout(workspace_roots.project_root(repository.resolve()))
        if checkout is None:
            return []
        directory = checkout / ".git"
        if not directory.is_dir():
            with directory.open("rb") as handle:
                pointer = handle.read(4097).decode("utf-8", "strict")
            target = (pointer[len("gitdir: "):].splitlines() or [""])[0].strip()
            if len(pointer) > 4096 or not pointer.startswith("gitdir: ") or not target:
                return []
            directory = checkout / target
        if (directory / "commondir").is_file():
            directory = directory / (directory / "commondir").read_text(encoding="utf-8").strip()
        with (directory / "logs" / "refs" / "heads" / branch).open("rb") as handle:
            raw = handle.read((1 << 20) + 1)
        # A long-lived branch's log runs to thousands of lines and almost
        # never names a rename: one byte search decides, not a parse.
        if len(raw) > 1 << 20 or b"\tBranch: renamed refs/heads/" not in raw:
            return []
        renames = [match.groups() for match in map(_RENAMED.fullmatch, raw.decode("utf-8", "replace").splitlines())
                   if match is not None]
    except (OSError, UnicodeError, ValueError, SourceGitError):
        return []
    names, current = [], branch
    for old, new in reversed(renames):
        if new == current and old != branch and old not in names:
            names.append(old)
            current = old
    return names


def _branch_task_identity(arguments: argparse.Namespace) -> None:
    """Give a branch-named task the ID its branch has.

    A name the grammar refuses gets the ID ``branch_task_id`` derives - only
    for the branch the checkout is on: the hooks name it as ``--task-id``,
    with ``--expected-branch`` beside it where they read (that branch is
    checked before anything is delivered), alone where they record a turn.
    Any other ID the grammar refuses is still refused.

    A branch renamed with `git branch -m` used to start over: its prompts
    read "working: not recorded yet", and the fifth turn recorded a second
    task beside the one that held its state. For the commands the hooks run,
    a branch with no active task of its own whose earlier name has one works
    with that task; the record keeps its ID, so a copy of the branch still
    under the old name shares it.
    """
    # Module state outlives a command run in-process (tests, the rerun after
    # a damaged database is moved aside): this command's own answer only.
    _COMMAND_STATE.pop("renamed_from", None)
    raw = getattr(arguments, "task_id", None)
    if not isinstance(raw, str) or not raw:
        return
    on_branch = None
    if not TASK_ID_PATTERN.fullmatch(raw.strip()):
        on_branch = getattr(arguments, "expected_branch", None) == raw or _on_branch(arguments.root, raw)
        if not on_branch:
            return
        arguments.task_id = branch_task_id(raw)
    if arguments.command not in _BRANCH_IDENTITY_COMMANDS:
        return
    earlier = _renamed_from(arguments.root, raw)
    if not earlier:
        return
    try:
        repository = arguments.root.resolve()
        if _active_claimants(repository, arguments.task_id) != []:
            return
        if on_branch is None:
            on_branch = getattr(arguments, "expected_branch", None) == raw or _on_branch(arguments.root, raw)
        if not on_branch:
            return
        for name in earlier:
            task_id = branch_task_id(name)
            if _active_claimants(repository, task_id):
                arguments.task_id = arguments.renamed_from = task_id
                _COMMAND_STATE["renamed_from"] = task_id
                return
    except (BrainError, OSError, ValueError):
        return


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


def merge_recent(existing: list[str], additions: list[str]) -> list[str]:
    """Merge with re-touched values moved to the end, so the tail is newest.

    The capsule projects a task's newest files. Under `merge_unique` a file
    edited all along kept the position of its first touch, so the file being
    worked on now fell out of the projection while long-finished ones stayed.
    """
    added = list(dict.fromkeys(additions))
    seen = set(added)
    return [value for value in existing if value not in seen] + added


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
    replace_next_steps: bool = False,
) -> dict[str, object]:
    """Merge an update into the lightweight working task.

    ``progress`` is the operator's narrative; ``auto_checkpoint`` is the
    automated turn flush's own field, replaced wholesale on every flush. They
    are separate parameters so automation can checkpoint without ever
    overwriting what the operator wrote. ``replace_next_steps`` makes
    ``next_steps`` the whole list, so a step that is done can leave it.
    """
    task_id = validate_task_id(task_id)
    next_steps = normalize_values("Working task next step", next_steps)
    files = validate_paths("Working task file", files)
    sources = normalize_values("Working task source", sources)
    if progress is None and auto_checkpoint is None and not (
        next_steps or files or sources or replace_next_steps
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
                    next_steps
                    if replace_next_steps
                    else merge_unique(json.loads(row["next_steps"]), next_steps),
                    ensure_ascii=False,
                ),
                json.dumps(merge_recent(json.loads(row["files"]), files), ensure_ascii=False),
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
        path = workspace_roots.resolve(repository, row["path"])
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
    except SourceGitError:
        raise
    except (ContextError, BrainError, RetrievalError, OSError, sqlite3.Error) as error:
        return (
            {layer: "failed" for layer in DOCUMENT_LAYERS},
            {},
            [f"Index refresh failed: {error}"],
        )
    return {layer: "updated" for layer in DOCUMENT_LAYERS}, result, []


def last_turn_report_path(repository: Path) -> Path:
    return repository / "memory-bank" / "local" / LAST_TURN_REPORT_FILENAME


REFRESH_HEALTH_FILE = "refresh-health.ndjson"


def refresh_health_path(repository: Path) -> Path:
    return repository / "memory-bank" / "local" / REFRESH_HEALTH_FILE


def append_refresh_health(repository: Path, record: dict[str, object]) -> None:
    """Append one turn's health line, or quietly do nothing.

    The honesty the hook already has lives exactly one turn: a timeout, a
    stalled index phase or a capsule that dropped content is visible in that
    turn's report and nowhere afterwards, so no one can say whether it happens
    once a week or every third prompt.

    Append-only NDJSON rather than the whole-file replace `write_last_turn_report`
    uses, because this is a series and that is a snapshot. Wrapped like it,
    though: a health write must never be the reason a turn fails.
    """
    path = refresh_health_path(repository)
    retention = refresh_health_retention(load_config(repository))
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(record, ensure_ascii=False) + "\n")
        lines = path.read_text(encoding="utf-8").splitlines()
        if len(lines) > retention:
            path.write_text(
                "\n".join(lines[-retention:]) + "\n", encoding="utf-8"
            )
    except (OSError, BrainError, ValueError):
        return


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


def summarize_last_turn(report: dict[str, object], *, routine: bool = True) -> Optional[str]:
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
    if report.get("native_capture_incomplete") is True:
        parts.append("native memory result not saved completely; inspect the prior save before relying on it")
    if report.get("promotion_retry_unavailable") is True:
        parts.append("promotion retry unavailable: local scheduling state could not be saved")
    if count("discarded_turns"):
        parts.append(
            f"this branch's task is closed: {count('discarded_turns')} turn(s) "
            "after its completion were not recorded; to keep further work, "
            + _closed_task_remedy(str(report.get("task_id") or ""))
        )
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
    if routine and promoted:
        parts.append(f"promoted: {len(promoted)} record(s)")
    if routine and count("promotion_skipped"):
        parts.append(
            f"promotion deferred: {count('promotion_skipped')} record(s)"
        )
    if routine and count("archived"):
        parts.append(f"archived: {count('archived')} record(s)")
    if routine and report.get("flushed"):
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


def last_turn_section(repository: Path, *, routine: bool = True) -> Optional[str]:
    """The 'Last turn' capsule section, or None when there is nothing to say."""
    report = load_last_turn_report(repository)
    if report is None:
        return None
    return summarize_last_turn(report, routine=routine)


# Entry points that build the capsule a hook delivers on its own; the
# capsule profile applies to these and never to an explicit retrieval.
AUTOMATIC_ENTRY_POINTS = ("refresh", "hook-context")


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
    query_source: str = "explicit",
    phase_seconds: Optional[dict[str, object]] = None,
    gate_mode: str = RETRIEVAL_GATE_DEFAULT,
    paths: Optional[list[str]] = None,
    source_hints: object = None,
    host: str = "cli",
    entry_point: str = "retrieve",
    allow_unprovisioned: bool = False,
    session_id: Optional[str] = None,
    transcript: Optional[str] = None,
    render: bool = False,
    prompt: Optional[str] = None,
    automatic_links: bool = False,
    trace_paths: Optional[list[str]] = None,
    index_warnings: Optional[list[str]] = None,
    index_notices: Optional[dict[str, int]] = None,
    delivery_profile: str = 'ordinary',
    source_provenance_sink: Optional[dict[str, object]] = None,
    delivery_admission: Optional[Callable[[], None]] = None,
    capsule_profile: Optional[str] = None,
) -> dict[str, object]:
    """Build the layered capsule for one request.

    ``prompt`` is the whole request ``query`` was distilled from; recorded
    history is searched with all of it (see history_query).

    ``automatic_links`` seeds the file-edge channel - paths ``prompt`` names
    and files this branch touched on its last turns - for automatic entry
    points (refresh, hook-context); explicit retrieval stays deterministic.

    ``refresh_index`` exists so a caller that already refreshed does not index
    twice; retrieval reads the index rather than the sources, so the refresh
    has to happen somewhere before this runs.

    ``query_source`` and ``phase_seconds`` are provenance only the caller
    knows - where the query came from, and what the index phases cost - and
    are recorded in the governed manifest. A caller that refreshed elsewhere
    passes its own timings; one that refreshes here has them measured for it.

    ``allow_unprovisioned`` lets the prompt-time refresh retrieve for a branch
    whose governed task does not exist yet - every read-only session, and the
    first turns of every branch until the checkpoint provisions it. The
    capsule then carries the retrieval layers with no working state, instead
    of nothing but a warning.

    ``render`` says the caller delivers the rendered text rather than the
    JSON: a governed capsule then keeps only the items that text shows, and
    retrieval records only those as delivered (see shown_in_render).
    """
    if source_provenance_sink is not None:
        if type(source_provenance_sink) is not dict:
            raise ContextError('Invalid source provenance transport')
        source_provenance_sink.clear()
    warnings: list[str] = list(index_warnings or [])
    source_notices = _normalized_source_notices(index_notices)
    if refresh_index:
        _, index_result, refresh_warnings = refresh_layers(connection, repository)
        warnings.extend(refresh_warnings)
        source_notices = _normalized_source_notices(index_result.get('source_notices'))
        # The refresh that just ran is this turn's index cost, so it wins over
        # anything the caller guessed.
        phase_seconds = index_result.get("phase_seconds") or None
    # What the silenced Stop hook did last turn, folded into this request's
    # capsule. Rendered once here so both modes report it identically; the
    # summary itself is bounded, and the lightweight budget below may still
    # drop it first.
    last_turn = last_turn_section(repository, routine=delivery_profile != 'cursor-rule')
    # The automatic capsule may be configured to carry navigation only; an
    # explicit retrieval always searches.
    navigation_only = (
        entry_point in AUTOMATIC_ENTRY_POINTS
        and configured_capsule_profile(repository, capsule_profile) == "navigation"
    )
    if mode == "lightweight":
        result = build_context_packet(
            connection,
            repository,
            query,
            task_id,
            limit,
            include_retrieval=not warnings and not navigation_only,
            warnings=warnings,
            delivery_profile=delivery_profile, query_source=query_source,
        )
        result["last_turn"] = last_turn
        if source_notices:
            result['source_notices'] = source_notices
        result.setdefault("query_source", query_source)
        # The Stop hook keeps the recent files in either mode.
        attach_recent_files(connection, repository, result)
        result = enforce_capsule_budget(result)
        if delivery_admission is not None:
            delivery_admission()
        return result
    if task_id is None:
        raise ContextError("Governed retrieval requires --task-id")
    # The governed path bypasses build_context_packet, so apply the same query
    # guards build_context_packet would have applied.
    reject_secrets("Task Capsule", [query])
    reject_capsule_privacy("Task Capsule request", [query])
    if prompt is not None:
        reject_secrets("Task Capsule", [prompt])
        reject_capsule_privacy("Task Capsule request", [prompt])
    try:
        binding = governed_binding(connection, task_id)
    except ContextError as error:
        if not allow_unprovisioned or "Working task not found" not in str(error):
            raise
        binding = None
    unbound = binding is None
    binding_notices: list[str] = []
    unreadable = False
    problem, bound_record = (_bound_record(repository, binding) if binding is not None and allow_unprovisioned
                             else (None, None))
    if problem == "missing":
        # The working task's record was removed with uncommitted files. Serve
        # the turn as for a task not recorded yet - knowledge included - and
        # say so; the next checkpoint records the task again.
        binding_notices.append(
            f"The working task's record {binding['task_uuid']} is gone (git stash -u or git clean "
            "removes an uncommitted task); the next checkpoint records the task again.")
        binding = None
    elif problem is not None:
        # The record is there but cannot be read as one. The turn keeps the
        # rest of its memory; the record is the person's to repair.
        binding_notices.append(f"The working task has no state this turn: {problem}. "
                               "Repair or restore that file; checkpoints wait for it.")
        binding, unreadable = None, True
    served = str(binding["task_uuid"]) if binding is not None else None
    served_id = str(bound_record["external_id"]) if bound_record is not None else task_id
    successor = _successor(repository, binding, bound_record) if binding is not None else None
    if successor is not None:
        # Read what the next checkpoint will bind to (see ensure_working_task).
        served = str(successor["id"])
    if unbound and allow_unprovisioned:
        # A fresh clone or a second machine holds the branch's task in the
        # Brain but no binding until its first checkpoint: the turns before
        # it read the task's state instead of "not recorded yet".
        record = _unbound_task(repository, task_id)
        if record is not None:
            served, served_id = str(record["id"]), str(record["external_id"])
    if served is not None and allow_unprovisioned:
        binding_notices.extend(_twin_notices(connection, served_id, served))
    if _COMMAND_STATE.get("renamed_from") == task_id:
        binding_notices.append(f"This branch was renamed from {task_id}; its task continues here.")
    # A finished task and nothing after it: no checkpoint starts another, so
    # "not recorded yet" would promise one that never comes.
    closed = (closed_task_record(connection, repository, task_id)
              if served is None and not unreadable else None)
    query_fallback = None
    if bound_record is not None and _task_fallback_prompt(query):
        # The prompt is an acknowledgement or a short continuation, not a
        # subject. The task it continues carries the retrieval terms; without
        # them a "fix it" turn asked memory about the word "fix".
        try:
            task_working, _ = project_working_task(
                governed_task_view(bound_record), delivery_profile=delivery_profile
            )
            if task_working:
                external_id = str(bound_record.get("external_id") or task_id)
                if task_working.get("goal") == derive_goal(external_id):
                    # The branch slug re-cased is not a subject; the same
                    # exclusion the hook's task query already applies.
                    task_working = {**task_working, "goal": ""}
                query_fallback = build_capsule_query("", task_working)
                query_source = "continuation"
        except (ContextError, RetrievalError, KeyError, TypeError, ValueError):
            query_fallback = None
    request_query = build_capsule_query(query, None)
    if warnings or navigation_only:
        # The previous index is not evidence of current eligibility. Keep
        # only fresh authoritative working state; do not search or record a
        # delivery, even if old document bytes still match their hashes.
        # A navigation-only profile takes the same path by choice: working
        # state and recent files, no documents and no delivery record.
        task = task_record = None
        if served is not None:
            task_record, task, _ = find_task(repository, served)
            validate_record(task)
        working, omitted = project_working_task(governed_task_view(task) if task else None,
                                                delivery_profile=delivery_profile)
        if working is not None:
            working.update(phase=task.get("phase"), created_at=task["created_at"],
                           updated_at=task["updated_at"])
        result: dict[str, object] = {
            "mode": mode, "query": request_query, "query_source": query_source,
            "task_id": task["external_id"] if task else task_id,
            "task_uuid": task["id"] if task else None,
            "task_revision": task["revision"] if task else None,
            "task_record": task_record.relative_to(repository).as_posix() if task_record else None,
            "working": working, "omitted": omitted, "last_turn": last_turn,
            "procedural": [], "semantic": [], "episodic": [],
            "categories": {}, "selected": [], "no_match": [], "repeated": 0,
            "manifest": None, "warnings": [*warnings, *binding_notices],
            "token_estimates": {"procedural": 0, "semantic": 0, "episodic": 0,
                                "local_episodes": 0, "total": 0},
        }
        if served is None:
            result.update(kind="warming", task_id=task_id,
                          pending_turns=len(pending_turn_deltas(connection, task_id)))
            if unreadable:
                result["task_record_unreadable"] = True
            if closed:
                result["task_closed"] = True
        if source_notices:
            result['source_notices'] = source_notices
        attach_recent_files(connection, repository, result)
        result = enforce_governed_capsule_contract(result)
        if delivery_admission is not None:
            delivery_admission()
        return result
    seeds: Optional[dict[str, list[str]]] = None
    if automatic_links:
        seeds = {"prompt": [], "touched": recent_touched_paths(connection, task_id)}
        if prompt:
            try:
                seeds["prompt"] = prompt_path_seeds(connection, repository, prompt)
            except (OSError, ValueError, sqlite3.Error):
                # Seeds add one item at most; they never cost the capsule.
                seeds["prompt"] = []

    def pack(result: dict[str, object]) -> dict[str, object]:
        # Inside retrieve(), before it records anything: what the delivered
        # capsule leaves out is not recorded as handed.
        deduplicate_capsule_layers(result)
        result["query_source"] = query_source
        # The print tail dereferences result["warnings"]; retrieve() has no such key.
        result["warnings"] = [*warnings, *binding_notices]
        if source_notices:
            result['source_notices'] = source_notices
        result["last_turn"] = last_turn
        if served is None:
            # Rendered as "working: not recorded yet" plus how far the first
            # checkpoint is, by the same fields the Cursor warming capsule uses.
            result["kind"] = "warming"
            result["task_id"] = task_id
            if unreadable:
                # Recorded, but its record cannot be read: not "not recorded
                # yet", and no checkpoint creates it while it is there.
                result["task_record_unreadable"] = True
            if closed:
                result["task_closed"] = True
            result["pending_turns"] = len(pending_turn_deltas(connection, task_id))
        attach_recent_files(connection, repository, result)
        capsule = enforce_governed_capsule_contract(result)
        if render:
            shown_in_render(connection, capsule, delivery_profile=delivery_profile)
        return capsule

    return retrieve(
        connection,
        repository,
        request_query,
        served,
        limit=limit,
        manifest_scope="local" if ephemeral else "governed",
        query_source=query_source,
        query_fallback=query_fallback,
        phase_seconds=phase_seconds,
        gate_mode=gate_mode,
        paths=paths,
        source_hints=source_hints,
        host=host,
        entry_point=entry_point,
        history_text=prompt if prompt is not None else query,
        automatic_seeds=seeds,
        session_id=session_id,
        transcript=transcript,
        pack=pack,
        trace_paths=trace_paths,
        delivery_profile=delivery_profile,
        describe_windows=describe_excerpt_windows,
        source_provenance_sink=source_provenance_sink,
        admit_delivery=delivery_admission,
    )


def hook_capsule_query(
    repository: Path, task_id: str, task_uuid: str,
    *, task_record: Optional[dict[str, object]] = None,
) -> tuple[str, str]:
    """The query the hook should ask with, and where it came from.

    A hook without a prompt - Cursor's stop and session-start renders -
    supplies only a task identifier, usually a branch name, and the governed
    path then tokenized that slug and retrieved on it. A branch called `main` asks
    memory for documents about the word "main"; `chore/accelerator-hardening`
    asks for a release skill. The enrichment `build_capsule_query` already
    performs for the lightweight path is applied here instead, so the question
    is the task's own goal and state.

    Deliberately excluded is the automatic checkpoint: it is a sentence of
    turn counts and an ISO timestamp with no topical content, it changes on
    every flush, and it would spend a third of a 32-token budget displacing
    real terms. An auto-provisioned goal is excluded too - it is the branch
    slug re-cased, so it adds the noise back under another name. When nothing
    substantive remains the bare identifier is used and the manifest says so.
    """
    try:
        task = get_task(repository, task_uuid) if task_record is None else task_record
        goal = str(task.get("goal") or "")
        # `derive_goal` builds the auto-provisioned form, so comparing against
        # it keeps the two definitions in lockstep instead of duplicating the
        # suffix as a literal here.
        if goal == derive_goal(task_id):
            goal = ""
        working = {
            "goal": goal,
            # The manual progress note only: `render_current_state` would fold
            # the checkpoint sentence back in.
            "progress": str(task.get("progress") or ""),
            "next_steps": task.get("next_steps") or [],
            "files": task.get("files") or [],
        }
        if not any(
            value for value in working.values() if value not in ("", [], None)
        ):
            return task_id, "task-id"
        query = build_capsule_query(task_id, working)
    except SourceGitError:
        raise
    except (ContextError, BrainError, RetrievalError, KeyError, ValueError, OSError):
        # A capsule built on a bare identifier is worse than one built on the
        # task, and far better than none: Cursor would otherwise keep serving
        # the previous rule with no signal that anything failed.
        return task_id, "task-id"
    return query, "task"


def _cursor_continuation_query(
    connection: sqlite3.Connection, repository: Path, task_id: str,
) -> Optional[tuple[str, str]]:
    """Use current nonterminal governed state; never reuse a previous rule."""
    try:
        binding = governed_binding(connection, task_id)
        task = get_task(repository, binding["task_uuid"])
        if task["status"] in LIFECYCLES["task"]["terminal"]:
            return None
        query_task = dict(task)
        if str(task.get("goal") or "").strip() and task["goal"] != derive_goal(task_id):
            # Incidental progress/next-step words can dilute an explicit goal
            # below relevance coverage. Only the query projection is narrowed;
            # the capsule still carries all current working fields.
            query_task.update(progress="", next_steps=[], files=[])
        query, source = hook_capsule_query(
            repository, task_id, binding["task_uuid"], task_record=query_task,
        )
        # Empty/auto-derived state and read failures otherwise seed a branch
        # name. A continuation is allowed only a substantive current task.
        return (query, source) if source == "task" else None
    except SourceGitError:
        raise
    except (ContextError, BrainError, RetrievalError, KeyError, ValueError, OSError):
        return None


CURSOR_TOPIC_KEY = 'cursor-topic-v1'
CURSOR_TOPIC_BYTES = 6144
CURSOR_TOPIC_AGE = 86400


def _cursor_topic_scope(connection, repository, task_id, mode):
    """Current query-cache scope, not authority or old-source custody proof."""
    from context_retrieval import _cursor_git_config_read, _cursor_protected_safe_root
    from validate import secret_policy_fingerprint
    if not isinstance(task_id, str) or not task_id or len(task_id) > 200:
        return None
    selectors = {'GIT_DIR', 'GIT_COMMON_DIR', 'GIT_WORK_TREE', 'GIT_INDEX_FILE',
                 'GIT_OBJECT_DIRECTORY', 'GIT_ALTERNATE_OBJECT_DIRECTORIES',
                 'GIT_CONFIG', 'GIT_CONFIG_PARAMETERS', 'GIT_CONFIG_COUNT'}
    if any(os.environ.get(name) for name in selectors):
        return None
    try:
        project = workspace_roots.project_root(repository)
        environment = {k: v for k, v in os.environ.items() if not k.startswith('GIT_')}
        environment.update(GIT_OPTIONAL_LOCKS='0', GIT_CONFIG_NOSYSTEM='1',
                           GIT_CONFIG_GLOBAL=os.devnull, GIT_TERMINAL_PROMPT='0')
        command = ['git', '--no-optional-locks', '--no-pager', '-C', str(project)]
        observed = _cursor_git_config_read([*command, 'symbolic-ref', '--quiet', '--short', 'HEAD'], environment)
        if observed is not None and observed[0] == 128 and _cursor_protected_safe_root(project):
            observed = _cursor_git_config_read([*command, '-c', 'safe.directory='+str(project.resolve()),
                                                'symbolic-ref', '--quiet', '--short', 'HEAD'], environment)
        if observed is None or observed[0] != 0 or not 1 <= len(observed[1]) <= 1024:
            return None
        branch = observed[1].decode('utf-8', 'strict').strip()
        if not branch or any(char in branch for char in ('\n', '\r', '\0')):
            return None
        config_leaf = brain_root(repository) / 'config/runtime.json'
        try:
            meta = config_leaf.lstat()
            if not stat_module.S_ISREG(meta.st_mode) or meta.st_nlink != 1 or meta.st_size > 65536:
                return None
            def same(image):
                return (all(getattr(image, key) == getattr(meta, key)
                            for key in ('st_dev','st_ino','st_size','st_mtime_ns','st_mode','st_nlink'))
                        and workspace_roots.change_time_ns(image) == workspace_roots.change_time_ns(meta))
            with config_leaf.open('rb') as stream:
                raw = stream.read(65537)
                after = os.fstat(stream.fileno())
                if len(raw) > 65536 or not same(after):
                    return None
            if not same(config_leaf.lstat()):
                return None
            config_identity = hashlib.sha256(raw).hexdigest()
        except FileNotFoundError:
            config_identity = 'absent'
        config = load_config(repository)
        alias = task_id
        task_uuid = generation = goal_hash = None
        kind = 'warming'
        if mode == 'governed':
            rows = connection.execute('SELECT external_id, task_uuid FROM task_bindings '
                'WHERE external_id=? OR task_uuid=?', (task_id, task_id)).fetchall()
            if len(rows) > 1:
                return None
            if rows:
                alias, task_uuid = rows[0]['external_id'], rows[0]['task_uuid']
                task = get_task(repository, task_uuid)
                if (task['external_id'] != alias or task['status'] in TERMINAL_STATES
                        or task['privacy'] not in set(config['allowed_privacy']) & {'public','team'}):
                    return None
                pointer = find_working_task(connection, alias)
                if pointer is None or pointer['goal'] != task_uuid:
                    return None
                generation = pointer['created_at']
                goal_hash = hashlib.sha256(task['goal'].encode('utf-8')).hexdigest()
                kind = 'bound'
            else:
                if find_working_task(connection, alias) is not None:
                    return None
                # Positively absent active/archive identity, not a catch-all
                # "not found" after parse/permission/ambiguity failure.
                from brain_runtime import iter_records
                problems = []
                if any(record.get('id') == alias or record.get('external_id') == alias
                       for _, record, _ in iter_records(repository, include_archive=True,
                                                        problems=problems)):
                    return None
                # A skipped record claiming this identity is not absence either.
                if any(problem['identity'] is not None and alias in problem['identity'][1:]
                       for problem in problems):
                    return None
        else:
            task = find_working_task(connection, alias)
            if task is not None:
                generation = task['created_at']
                goal_hash = hashlib.sha256(str(task['goal']).encode('utf-8')).hexdigest()
                kind = 'bound'
        return {'mode': mode, 'branch': branch, 'task_id': alias, 'task_uuid': task_uuid,
                'generation': generation, 'goal': goal_hash, 'kind': kind,
                'root': hashlib.sha256(os.fsencode(project.resolve())).hexdigest(),
                'config': config_identity, 'secret_policy': secret_policy_fingerprint()}
    except (OSError, ValueError, UnicodeError, BrainError, ContextError, sqlite3.Error, KeyError):
        return None


def _cursor_topic_begin(connection, repository, task_id, mode):
    scope = _cursor_topic_scope(connection, repository, task_id, mode)
    raw = None
    try:
        row = connection.execute("SELECT CASE WHEN typeof(value)='text' AND length(CAST(value AS BLOB))<=6144 "
            "THEN value ELSE NULL END FROM index_state WHERE key=?", (CURSOR_TOPIC_KEY,)).fetchone()
        raw = row[0] if row else None
        seed = None
        if isinstance(raw, str) and len(raw.encode('utf-8')) <= CURSOR_TOPIC_BYTES:
            value = json.loads(raw)
            if (isinstance(value, dict) and set(value) == {'profile','scope','query','written_at'}
                    and value['profile'] == CURSOR_TOPIC_KEY and value['scope'] == scope and scope is not None
                    and isinstance(value['query'], str) and 1 <= len(value['query']) <= 1024
                    and type(value['written_at']) in (int,float)
                    and 0 <= value['written_at'] <= time.time() and math.isfinite(value['written_at'])
                    and 0 <= time.time() - value['written_at'] <= CURSOR_TOPIC_AGE):
                reject_secrets('Cursor continuation query', [value['query']])
                reject_capsule_privacy('Cursor continuation query', [value['query']])
                seed = value
        return scope, raw, seed
    except (ValueError, UnicodeError, ContextError, sqlite3.Error, RecursionError, OverflowError):
        return scope, raw, None


def _cursor_topic_write(connection, expected, value):
    """Own short CAS transaction; never commit or roll back a caller's work."""
    if connection.in_transaction:
        return False
    previous_timeout = None
    try:
        # Topic bookkeeping must not spend the hook's delivery timeout waiting
        # for another writer. Restore the caller's setting even after failure.
        previous_timeout = connection.execute('PRAGMA busy_timeout').fetchone()[0]
        connection.execute('PRAGMA busy_timeout=0')
        with connection:
            if value is None:
                if expected is not None:
                    connection.execute('DELETE FROM index_state WHERE key=? AND value=?', (CURSOR_TOPIC_KEY,expected))
                return True
            raw = json.dumps(value, ensure_ascii=False, sort_keys=True)
            if len(raw.encode('utf-8')) > CURSOR_TOPIC_BYTES:
                return False
            if expected is None:
                # Repair only an unavailable oversized/non-text reserved row;
                # never replace another operation's valid admitted query.
                connection.execute("DELETE FROM index_state WHERE key=? AND "
                    "(typeof(value)!='text' OR length(CAST(value AS BLOB))>6144)", (CURSOR_TOPIC_KEY,))
                cursor = connection.execute('INSERT INTO index_state(key,value) VALUES (?,?) '
                    'ON CONFLICT(key) DO NOTHING', (CURSOR_TOPIC_KEY,raw))
            else:
                cursor = connection.execute('UPDATE index_state SET value=? WHERE key=? AND value=?',
                                            (raw,CURSOR_TOPIC_KEY,expected))
            return cursor.rowcount == 1
    except (ValueError, UnicodeError, sqlite3.Error):
        return False
    finally:
        if previous_timeout is not None:
            try:
                connection.execute('PRAGMA busy_timeout='+str(int(previous_timeout)))
            except sqlite3.Error:
                pass


def _cursor_topic_query(connection, repository, task_id, mode, plan):
    scope, _, seed = plan
    if scope is None:
        return None
    try:
        task = (get_task(repository, scope['task_uuid']) if mode == 'governed' and scope['task_uuid']
                else find_working_task(connection, task_id) if mode == 'lightweight' else None)
        if task is not None and str(task.get('goal') or '').strip() and task['goal'] != derive_goal(task_id):
            if mode == 'governed':
                if task['privacy'] not in set(load_config(repository)['allowed_privacy']) & {'public','team'}:
                    return None
                # A newly explicit goal must not inherit incidental words from
                # the old branch alias. Use the current goal alone for recall.
                return build_capsule_query(task['goal'], None), 'task'
            return build_capsule_query(str(task['goal']), None), 'task'
        return (seed['query'], 'continuation') if seed is not None else None
    except SourceGitError:
        raise
    except (BrainError, ContextError, KeyError, ValueError, OSError, sqlite3.Error):
        return None


def _cursor_topic_finish(connection, repository, task_id, mode, plan, query, eligible, warnings, *, continued=False, acknowledgement=False, selected=True):
    scope, raw, seed = plan
    if not eligible or warnings or scope is None:
        _cursor_topic_write(connection, raw, None)
        return
    if (not isinstance(query, str) or not 1 <= len(query) <= 1024
            or _cursor_topic_scope(connection, repository, task_id, mode) != scope):
        _cursor_topic_write(connection, raw, None)
        return
    if continued and seed is not None and seed['query'] == query:
        return  # Acknowledgements do not renew the substantive topic's age.
    if acknowledgement:
        if seed is None:
            _cursor_topic_write(connection, raw, None)
        return  # An acknowledgement is never a new substantive topic.
    if not selected:
        # A prompt that retrieved nothing is no topic to continue: a short
        # reply became one, and the next "yes" searched by it.
        _cursor_topic_write(connection, raw, None)
        return
    _cursor_topic_write(connection, raw, {'profile':CURSOR_TOPIC_KEY, 'scope':scope,
        'query':query, 'written_at':time.time()})


def _cursor_topic_handoff(connection, repository, task_id, mode, plan, created):
    scope, raw, seed = plan
    if scope is None or scope['kind'] != 'warming' or seed is None or not created:
        return
    current = _cursor_topic_scope(connection, repository, task_id, mode)
    if current is None or current['kind'] != 'bound':
        return
    expected = created[0]
    if current['task_uuid'] != expected['task_uuid'] or current['generation'] != expected['generation']:
        return
    if any(current[key] != scope[key] for key in ('mode','branch','task_id','root','config','secret_policy')):
        return
    if current['goal'] != hashlib.sha256(derive_goal(task_id).encode('utf-8')).hexdigest():
        return
    _cursor_topic_write(connection, raw, {**seed, 'scope': current})


def assemble_hook_context(
    connection: sqlite3.Connection,
    repository: Path,
    *,
    mode: str,
    task_id: str,
    gate_mode: str = RETRIEVAL_GATE_DEFAULT,
    host: str = "cli",
    render: bool = False,
    delivery_profile: str = 'ordinary',
    delivery_admission: Optional[Callable[[], None]] = None,
) -> Optional[dict[str, object]]:
    """Return a governed capsule or a sanitized pre-provision warming capsule."""
    task_id = validate_task_id(task_id)
    reject_capsule_privacy("Task Capsule request", [], [task_id])
    if delivery_profile == 'cursor-rule':
        plan = _cursor_topic_begin(connection, repository, task_id, mode)
        topic = _cursor_topic_query(connection, repository, task_id, mode, plan)
        if topic is not None:
            return assemble_capsule(connection, repository, mode=mode, query=topic[0], task_id=task_id,
                limit=3, ephemeral=True, query_source=topic[1], gate_mode=gate_mode, host=host,
                entry_point='hook-context', render=render, allow_unprovisioned=True,
                automatic_links=True, delivery_profile=delivery_profile,
                delivery_admission=delivery_admission)
    if mode == "lightweight":
        local_task = find_working_task(connection, task_id)
        if local_task is not None:
            working, _ = project_working_task(local_task)
            query, query_source = build_capsule_query("", working), "task"
            return assemble_capsule(
                connection,
                repository,
                mode=mode,
                query=query,
                task_id=task_id,
                limit=3,
                ephemeral=True,
                query_source=query_source,
                gate_mode=gate_mode,
                host=host,
                entry_point="hook-context",
                delivery_profile=delivery_profile,
                delivery_admission=delivery_admission,
            )
    else:
        try:
            binding = governed_binding(connection, task_id)
        except ContextError:
            binding = None
        # A fresh clone holds the branch's task in the Brain before its first
        # checkpoint binds it: the session's rule shows that task too.
        record = _unbound_task(repository, task_id) if binding is None else None
        if binding is not None or record is not None:
            query, query_source = hook_capsule_query(
                repository, task_id, binding["task_uuid"] if binding is not None else str(record["id"])
            )
            return assemble_capsule(
                connection,
                repository,
                mode=mode,
                query=query,
                task_id=task_id,
                limit=3,
                ephemeral=True,
                query_source=query_source,
                gate_mode=gate_mode,
                host=host,
                entry_point="hook-context",
                render=render,
                allow_unprovisioned=binding is None,
                automatic_links=True,
                delivery_profile=delivery_profile,
                delivery_admission=delivery_admission,
            )

    files, excluded = changed_paths(repository)
    if delivery_admission is not None:
        delivery_admission()
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


def _bounded_cut(text: object, limit: int) -> tuple[str, int]:
    """``text`` on one line within ``limit``, and how many characters it lost."""
    collapsed = " ".join(str(text or "").split())
    if len(collapsed) <= limit:
        return collapsed, 0
    kept = collapsed[: limit - 1].rstrip()
    return kept + "…", len(collapsed) - len(kept)


def _bounded(text: object, limit: int) -> str:
    return _bounded_cut(text, limit)[0]


def _whole_paths(label: str, paths: list[str], limit: int = RENDERED_LINE_LIMIT) -> list[str]:
    """The leading ``paths`` that fit one rendered line after ``label``.

    Whole paths only: a path cut in half names no file. Even the first is
    omitted if it cannot fit; the caller counts it as omitted.
    """
    kept: list[str] = []
    width = len(label)
    for path in paths:
        width += len(path) + (2 if kept else 0)
        if width > limit:
            if kept:
                break
            width = len(label)
            continue
        kept.append(path)
    return kept


def recent_files_line(project: list[str], task_files: list[str]) -> tuple[Optional[str], int]:
    """The rendered "recent files" line, newest first: the project's recent
    files, then the task's changed files they do not name, ten at most and
    whole paths only. Returns the line and how many files it leaves out, for
    the compaction line: every one of the task's changed files it does not
    show counts, a file the project's pushed out of the ten included -
    otherwise the task's own state shrank with nothing to say so."""
    candidates = list(dict.fromkeys(project))
    for path in reversed(task_files):
        if path not in candidates:
            candidates.append(path)
    candidates = candidates[:RECENT_FILES_LIMIT]
    kept = _whole_paths("recent files: ", candidates, RECENT_FILES_LINE_LIMIT)
    shown = set(kept)
    omitted = sum(1 for path in dict.fromkeys([*candidates, *task_files]) if path not in shown)
    if not kept:
        return None, omitted
    return "recent files: " + ", ".join(kept), omitted


def working_state_lines(capsule: dict[str, object]) -> list[str]:
    """The task's state as the rendered capsule shows it, under `working:`."""
    return rendered_working_state(capsule)[0]


def rendered_working_state(
    capsule: dict[str, object],
) -> tuple[list[str], dict[str, int]]:
    """The task's state as the rendered capsule shows it, and what it cuts.

    The JSON capsule always carried progress, next steps and files, but the
    rendered capsule - the only form Claude Code, Codex and Cursor read -
    printed the goal alone. "Continue where we left off" then retrieved skill
    pointers and never the place the work stopped. Every line is bounded and
    left out when empty, because the rendered capsule repeats on every turn.

    The bounds cut what the JSON holds whole, so the cuts are returned in
    `omitted` terms for the compaction line: a narrative cut here and never
    counted read as the task's complete state. Progress keeps both ends, as
    `truncate_progress` does for the JSON budget - the constraint opens it and
    where the work now stands closes it.
    """
    working = capsule.get("working")
    if not isinstance(working, dict):
        return [], {}
    cuts = {
        "working_progress_characters": 0,
        "working_next_step_characters": 0,
        "working_files": 0,
        "working_sources": 0,
    }
    lines: list[str] = []
    if working.get("phase"):
        lines.append(f"phase: {working['phase']}")
    progress = " ".join(str(working.get("progress") or "").split())
    if len(progress) > RENDERED_PROGRESS_LIMIT:
        keep = RENDERED_PROGRESS_LIMIT - 1
        cuts["working_progress_characters"] = len(progress) - keep
        progress = truncate_progress(progress, keep)
    if progress:
        lines.append(f"progress: {progress}")
    for step in working.get("next_steps") or []:
        text, lost = _bounded_cut(step, RENDERED_PROGRESS_LIMIT // 2)
        cuts["working_next_step_characters"] += lost
        lines.append(f"next: {text}")
    files = [str(path) for path in working.get("files") or []]
    project = [path for path in capsule.get("recent_files") or [] if isinstance(path, str)]
    if project:
        # The project's recent files lead; the task's own changed files
        # follow where the project's do not already name them.
        line, cuts["working_files"] = recent_files_line(project, files)
        if line:
            lines.append(line)
    elif files:
        # Newest last in the task, so the tail is the work in hand: the
        # newest that fit, shown in the task's order.
        recent = _whole_paths(
            "recent files: ", files[-RENDERED_FILE_LIMIT:][::-1]
        )[::-1]
        cuts["working_files"] = len(files) - len(recent)
        lines.append("recent files: " + ", ".join(recent))
    sources = [str(path) for path in working.get("sources") or []]
    if sources:
        # The task's own citations, first first as the budget keeps them:
        # the source a compaction line says to re-read.
        cited = _whole_paths("sources: ", sources)
        cuts["working_sources"] = len(sources) - len(cited)
        lines.append("sources: " + ", ".join(cited))
    record = capsule.get("task_record")
    if isinstance(record, str) and record:
        lines.append(f"task record: {record}")
    return lines, cuts


class Excerpt(str):
    """An excerpt's text, carrying the heading of the section it quotes and
    the length it was allowed."""

    heading = ""
    limit = None
    cut = False


def best_excerpt(content: str, terms: set[str], limit: int, title: str = "") -> str:
    """The part of a document that answers the query, bounded to `limit`.

    A pointer - path and title - carried the answer in 0 of 49 measured
    questions, and agents opened 0 of 148 skill pointers they were handed: a
    pointer is only worth something if it is followed, and it was not. The
    section sharing the most query terms is what gets delivered, the stretch
    of it where the terms are; a document with no matching section gives its
    opening prose instead. The capsule marks matches with the index's own
    tokenizer (capsule_excerpts); this form marks the given words as written.
    """
    pattern = (
        re.compile(
            r"\b(" + "|".join(re.escape(term) for term in sorted(terms, key=len, reverse=True)) + r")\b",
            re.IGNORECASE,
        )
        if terms
        else None
    )
    marked = pattern.sub(lambda m: f"\x02{m.group(0)}\x03", content) if pattern else content
    section = quoted_section(marked, title)
    if section is None:
        return ""
    text = excerpt_window(section["units"], max(1, limit - len(section["shown"]) - 2) if section["shown"] else limit)
    return _bounded(f"{section['shown']}: {text}" if section["shown"] else text, limit)


def capsule_excerpts(
    connection: sqlite3.Connection, capsule: dict[str, object]
) -> dict[str, str]:
    """Answer-bearing text for the capsule's project knowledge, by path,
    the best-ranked excerpts grown into the room the capsule leaves.

    Each excerpt is first cut to its rank's allowance. When the excerpts
    leave room under RENDERED_CAPSULE_LIMIT beside EXCERPT_GROWTH_RESERVE,
    the first and second excerpts that were cut short - their section runs
    on past the window - are cut again with that room, up to
    EXCERPT_GROWTH_CAPS. The renderer still holds the whole text to the
    ceiling, shrinking the last excerpts first.
    """
    excerpts = _capsule_excerpts(connection, capsule)
    grown = _excerpt_growth(capsule, excerpts)
    return _capsule_excerpts(connection, capsule, limits=grown) if grown else excerpts


def _excerpt_growth(capsule: dict[str, object], excerpts: dict[str, str]) -> dict[str, int]:
    """Larger allowances, by path, for the best-ranked excerpts that were cut.

    The head's reserve takes the whole `recent files:` line while the capsule
    carries one: the reserve was set before that line could fill 720
    characters, and growth that counted on the room made the renderer cut the
    later excerpts to pay for it. Its limit, not its length, so the windows
    stay the same from turn to turn."""
    reserve = EXCERPT_GROWTH_RESERVE + (RECENT_FILES_LINE_LIMIT + 1 if capsule.get("recent_files") else 0)
    room = RENDERED_CAPSULE_LIMIT - reserve - sum(len(text) + 3 for text in excerpts.values() if text)
    order = [
        item["path"] for layer in ("semantic", "episodic") for item in capsule.get(layer) or []
        if isinstance(item, dict) and isinstance(item.get("path"), str) and excerpts.get(item["path"])
    ]
    grown: dict[str, int] = {}
    for cap, path in zip(EXCERPT_GROWTH_CAPS, order):
        text = excerpts[path]
        limit = getattr(text, "limit", None)
        # A section quoted whole gains nothing from more room. Length cannot
        # tell: a window of whole entries stops short of its allowance.
        if room <= 0 or not limit or not getattr(text, "cut", False) or cap <= limit:
            continue
        grown[path] = limit + min(cap - limit, room)
        room -= grown[path] - limit
    return grown


def _capsule_excerpts(
    connection: sqlite3.Connection, capsule: dict[str, object],
    *, limits: Optional[dict[str, int]] = None,
) -> dict[str, str]:
    """Answer-bearing text for the capsule's project knowledge, by path.

    Skills get none: every host loads its skills itself, so a skill line names
    which one applies and its text is one Skill call away. Weak matches get
    none: one shared rare word is a reason to mention a document, not to quote
    it. Each value is an Excerpt whose ``heading`` names the section quoted.
    """
    if not capsule.get("semantic") and not capsule.get("episodic"):
        return {}
    query = str(capsule.get("query") or "")
    weights = excerpt_weights(connection, query) if query else {}
    images = _capsule_index_images(capsule)
    excerpts: dict[str, str] = {}
    rank = 0
    for layer in ("semantic", "episodic"):
        for item in capsule.get(layer) or []:
            if (
                layer == "episodic" and isinstance(item, dict)
                and "path" not in item and item.get("id") is not None
            ):
                # A local episode has no document to quote: its outcome is
                # what it learned. Without it only a 160-character summary
                # reached the reader.
                outcome = _bounded(str(item.get("outcome") or ""), EPISODIC_EXCERPT_CHARACTERS)
                if outcome:
                    excerpt = Excerpt(outcome)
                    excerpt.heading = ""
                    excerpts[f"episode {item['id']}"] = excerpt
                continue
            if not isinstance(item, dict) or not isinstance(item.get("path"), str):
                continue
            if item.get("match") == "distinctive":
                continue
            if images is None:
                # A detached/public capsule has bounded admitted snippets,
                # never authority to rehydrate another live indexed body.
                text = item.get('snippet')
                if isinstance(text, str) and text:
                    excerpt = Excerpt(_bounded(text, max(EXCERPT_CHARACTERS)
                                               if layer == 'semantic' else EPISODIC_EXCERPT_CHARACTERS))
                    excerpt.heading = ''
                    excerpts[item['path']] = excerpt
                continue
            image = images.get(item['path'])
            if image is None:
                continue
            if item.get("selection") in AUTOMATIC_LINK_SELECTIONS and item.get("kind") == "brain-task":
                # What a sibling task says about the shared file: its goal and
                # the files it touched, the shared one first. 12 of the 14
                # answers held by tasks on the evaluation were Files entries.
                limit = (limits or {}).get(item["path"], EXCERPT_CHARACTERS[min(rank, len(EXCERPT_CHARACTERS) - 1)])
                rank += 1
                text, heading = linked_task_excerpt(
                    image[2], str(item.get("via") or ""), limit
                )
                if text:
                    excerpt = Excerpt(text)
                    excerpt.heading = heading
                    excerpt.limit = limit
                    excerpt.cut = len(text) >= limit - 40
                    excerpts[item["path"]] = excerpt
                continue
            limit = (limits or {}).get(item["path"], (
                EXCERPT_CHARACTERS[min(rank, len(EXCERPT_CHARACTERS) - 1)]
                if layer == "semantic"
                else EPISODIC_EXCERPT_CHARACTERS
            ))
            rank += 1
            marked = marked_document(connection, item["path"], query, indexed_image=image) if query else None
            if marked is None:
                marked = image[2]
            prefer_head = item.get("kind") == "changelog"
            # The marked image of a long document is a stretch around the
            # query; the head of a chronological document is outside it.
            quote_text = image[2] if prefer_head else marked
            section = (
                excerpt_section(connection, item, quote_text, weights, query=query,
                                prefer_head=prefer_head)
                if quote_text
                else None
            )
            text = (
                excerpt_window(section["units"], limit, weights, prefer_head=prefer_head)
                if section else ""
            )
            if not text and isinstance(item.get("snippet"), str):
                text = _bounded(item["snippet"].replace("[", "").replace("]", ""), limit)
            if text:
                excerpt = Excerpt(text)
                excerpt.heading = section["shown"] if section else ""
                excerpt.limit = limit
                excerpt.cut = bool(section) and sum(len(_unmarked(unit)) + 1 for unit in section["units"]) > limit
                excerpts[item["path"]] = excerpt
    return excerpts


_TASK_SECTION = re.compile(r"^## (.+?)\s*$", re.M)
_TASK_LIST_ITEM = re.compile(r"^- `?([^`\n]+?)`?\s*$")


def linked_task_excerpt(content: str, via: str, limit: int) -> tuple[str, str]:
    """A linked task's goal and its files, the shared one first: (text,
    heading of the list quoted)."""
    sections: dict[str, str] = {}
    matches = list(_TASK_SECTION.finditer(content))
    for index, match in enumerate(matches):
        end = matches[index + 1].start() if index + 1 < len(matches) else len(content)
        sections[match.group(1).strip()] = content[match.end():end].strip()
    goal = next((part.strip() for part in sections.get("Goal", "").split("\n\n") if part.strip()), "")
    heading = "Sources" if via and via in sections.get("Sources", "") and via not in sections.get("Files", "") else "Files"
    entries = [
        found.group(1).strip()
        for line in sections.get(heading, "").splitlines()
        if (found := _TASK_LIST_ITEM.match(line.strip()))
    ]
    ordered = ([via] if via in entries else []) + [entry for entry in reversed(entries) if entry != via]
    parts = []
    if goal:
        parts.append(f"Goal: {' '.join(goal.split())}.")
    if ordered:
        parts.append(f"{heading}: {', '.join(ordered[:6])}")
    return _bounded(" ".join(parts), limit), heading


def _item_line(item: dict[str, object], kind: str, section: str = "") -> str:
    if "path" in item:
        label, title = _bounded(item["path"], 260), str(item.get("title") or "")
        if section:
            # The section the excerpt below quotes: a later question answered
            # by another section of the document is handed that one too.
            label += " § " + _bounded(section.replace(" — ", " - "), 80)
    else:
        label, title = f"episode {item.get('id')}", str(item.get("summary") or "")
    marks = []
    if item.get("match") == "distinctive":
        # Admitted on one rare term rather than on covering the query: still
        # worth naming, not worth being read as an answer.
        marks.append("weak match")
    if item.get("match") == "conflict":
        marks.append("conflicts with another item here")
    if item.get("attestation") == "agent":
        marks.append("agent-attested, not reviewed by a person")
    if item.get("selection") in AUTOMATIC_LINK_SELECTIONS and item.get("via"):
        marks.append(
            f"linked through {_bounded(str(item['via']), 160)}, "
            + ("named in the request" if item["selection"] == "prompt-link" else "changed in this task")
        )
    changed = item.get("source_changed")
    if isinstance(changed, list) and changed:
        # One path and a count: a changed citation keeps records in the
        # capsule now, and three long paths per item ate the excerpts.
        more = f" +{len(changed) - 1} more" if len(changed) > 1 else ""
        marks.append(
            f"cited file changed: {_bounded(str(changed[0]), 120)}{more}; "
            "check it before relying on this"
        )
    suffix = f" ({'; '.join(marks)})" if marks else ""
    return f"- {kind} {label} — {_bounded(title, 160)}{suffix}"


def render_capsule_lines(
    capsule: dict[str, object], excerpts: Optional[dict[str, str]] = None,
    *, trace: Optional[dict[str, object]] = None, delivery_profile: str = 'ordinary',
) -> list[str]:
    """The capsule as Claude Code, Codex, Cursor and Harness deliver it."""
    return _render_capsule(capsule, excerpts, trace=trace, delivery_profile=delivery_profile)[0]


def shown_in_render(connection: sqlite3.Connection, capsule: dict[str, object], *, delivery_profile: str = 'ordinary') -> None:
    """Leave in a capsule only the items its rendered text shows.

    The text is held to RENDERED_CAPSULE_LIMIT by dropping whole entries once
    the excerpts are gone - skills first, then the weakest knowledge - and a
    dropped entry never reaches the model. Retrieval used to record it as
    handed all the same, so the next turns left it out as an item that
    "still applies". The capsule now stops carrying it, retrieval records
    only what the capsule holds, and the text rendered from the capsule shows
    every item it holds: they fitted without their excerpts, beside the
    compaction marker that every drop puts in front of them first.
    """
    _, shown = _render_capsule(capsule, capsule_excerpts(connection, capsule), delivery_profile=delivery_profile)
    kept = {id(item) for item in shown}
    omitted = capsule.get("omitted")
    for layer in DOCUMENT_LAYERS:
        items = capsule.get(layer)
        if not isinstance(items, list):
            continue
        capsule[layer] = [
            item for item in items if not isinstance(item, dict) or id(item) in kept
        ]
        dropped = len(items) - len(capsule[layer])
        if dropped and isinstance(omitted, dict) and isinstance(omitted.get(layer), int):
            omitted[layer] += dropped
    synchronize_capsule_views(capsule)


def _screen_capsule_excerpts(capsule: dict[str, object], excerpts: dict[str, str]) -> dict[str, str]:
    """The renderer privacy screen, shared with metadata-only proposals."""
    excerpts = dict(excerpts)
    # Excerpts are document text on its way into a prompt: held to the same
    # personal-data and secret screen as the query. capsule_excerpts already
    # chose and sized them; the bound here only keeps text from elsewhere
    # inside the ceiling. Each keeps the heading of the section it quotes.
    for layer in ("semantic", "episodic"):
        for item in capsule.get(layer) or []:
            if not isinstance(item, dict):
                continue
            path = str(item.get("path", f"episode {item.get('id')}"))
            raw = excerpts.get(path)
            if not raw:
                continue
            try:
                safe = sanitize_automatic_query(str(raw))
            except ContextError:
                safe = ""
            limit = getattr(raw, "limit", None) or (
                max(EXCERPT_CHARACTERS) if layer == "semantic" else EPISODIC_EXCERPT_CHARACTERS
            )
            excerpt = Excerpt(_bounded(safe, limit))
            excerpt.limit = getattr(raw, "limit", None)
            try:
                excerpt.heading = sanitize_automatic_query(str(getattr(raw, "heading", "")))
            except ContextError:
                excerpt.heading = ""
            excerpts[path] = excerpt

    return excerpts


def describe_excerpt_windows(connection: sqlite3.Connection, capsule: dict[str, object]) -> dict[str, object]:
    """Hash the sanitized pre-novelty windows; never claim final pack fit."""
    raw = capsule_excerpts(connection, capsule)
    screened = _screen_capsule_excerpts(capsule, raw)
    result = {}
    for layer in ("procedural", "semantic", "episodic"):
        for item in capsule.get(layer) or []:
            if not isinstance(item, dict) or not isinstance(item.get("path"), str):
                continue
            doc_ref = item["path"]
            text = str(screened.get(doc_ref) or "")
            encoded = text.encode("utf-8")
            result[doc_ref] = {
                "sha256": hashlib.sha256(encoded).hexdigest(), "bytes": len(encoded),
                "state": "excerpt" if text else "sanitized-empty" if raw.get(doc_ref) else "no-excerpt",
            }
    return result


def _render_capsule(
    capsule: dict[str, object], excerpts: Optional[dict[str, str]] = None,
    *, trace: Optional[dict[str, object]] = None, delivery_profile: str = 'ordinary',
) -> tuple[list[str], list[dict[str, object]]]:
    """The rendered lines, and the items they show."""
    excerpts = dict(excerpts or {})
    excerpt_present = {path: bool(text) for path, text in excerpts.items()} if trace is not None else {}
    excerpts = _screen_capsule_excerpts(capsule, excerpts)

    # The first line is the render marker: the Cursor hooks accept a capsule
    # only if it starts with "working:", which is how a broken render is kept
    # from replacing a good rule now that they no longer parse JSON.
    head: list[str] = []
    # What the head's own bounds cut from the task's state, beside what the
    # capsule's `omitted` counters say its JSON lost.
    cuts: dict[str, int] = {}
    working = capsule["working"]
    if working is None and capsule.get("task_record_unreadable"):
        # The task is recorded, but its record cannot be read; the warning
        # below names the file and the cause.
        head.append("working: unavailable (the task's record cannot be read)")
    elif working is None and capsule.get("working_changed"):
        # Rewritten while this turn read it; the record has the state.
        head.append("working: unavailable (the task's record changed while this turn read it; read "
                    + _bounded(str(capsule.get("task_record") or "the task record"), 200) + ")")
    elif working is None and capsule.get("task_closed"):
        # The branch's task is finished; the turns after it are not recorded.
        head.append("working: closed (this branch's task is finished; turns are not recorded until a "
                    "new one starts: " + _closed_task_remedy(str(capsule.get("task_id") or "")) + ")")
    elif working is None and capsule.get("kind") == "warming":
        # A branch whose task the first checkpoint has not created yet: the
        # layers below are still this turn's retrieval.
        head.append(
            "working: not recorded yet (the task starts at the "
            f"{DEFAULT_TURN_FLUSH_AFTER}-turn checkpoint)"
        )
    elif working is None:
        head.append("working: unavailable")
    else:
        goal, cuts["working_goal_characters"] = _bounded_cut(working["goal"], 200)
        head.append(f"working: {_bounded(working['task_id'], 200)} — {goal}")
        state, state_cuts = rendered_working_state(capsule)
        cuts.update(state_cuts)
        head.extend(
            _bounded(line, RECENT_FILES_LINE_LIMIT if line.startswith("recent files: ") else RENDERED_LINE_LIMIT)
            for line in state
        )
    if working is None and capsule.get("recent_files"):
        # Before the task is recorded - a new branch, a closed task - the
        # project still says where the work was.
        line, cuts["working_files"] = recent_files_line(
            [path for path in capsule["recent_files"] if isinstance(path, str)], []
        )
        if line:
            head.append(line)
    if capsule.get("kind") == "warming" and not capsule.get("task_closed"):
        head.append('warming: task checkpoint pending' if delivery_profile == 'cursor-rule'
                    else f"warming: {capsule['pending_turns']} turn(s) pending")
    notice = _source_notice_text(capsule.get('source_notices'))
    if notice:
        head.append(_bounded(notice, 160))
    for warning in capsule["warnings"][:3]:
        head.append(f"warning: {_bounded(warning, 160)}")
    # The JSON form has carried these counters all along; the rendered form
    # is what a prompt-time hook shows, so without this line a compacted
    # capsule reaches the model looking complete. It counts what the text
    # itself cuts, too: the text is the only form the model reads.
    dropped = compaction_summary(capsule, cuts)
    compaction: Optional[str] = (
        f"compaction: omitted {dropped} — this capsule is a lossy view; "
        f"{COMPACTION_ADVICE}"
        if dropped
        else None
    )
    tail: list[str] = []
    if capsule.get("last_turn"):
        tail.append(f"Last turn: {_bounded(capsule['last_turn'], 600)}")
    source = capsule.get("query_source")
    if source in ("task", "task-id") and not (delivery_profile == 'cursor-rule' and source == 'task'):
        # A capsule retrieved on a branch slug and one retrieved on the task's
        # own goal are worth very different amounts.
        tail.append(
            "query: from task goal"
            if source == "task"
            else "query: from branch name only"
        )
    gate = capsule.get("gate")
    if isinstance(gate, dict) and gate.get("mode") == "enforce" and gate.get(
        "decision"
    ) == "skip":
        # An empty capsule and a withheld one are different facts and must
        # not render the same.
        tail.append(f"gate: skipped — {gate.get('reason', 'unknown')}")
    repeated = capsule.get("repeated")
    if isinstance(repeated, int) and repeated > 0:
        tail.append(
            f"memory: {repeated} item(s) handed earlier in this conversation "
            "still apply"
        )
    no_match = capsule.get("no_match")
    delivered = any(capsule.get(layer) for layer in DOCUMENT_LAYERS) or bool(repeated)
    if no_match and not delivered:
        # "Memory has nothing for this" is an answer, and until it was said
        # out loud it looked exactly like "memory was not consulted". Beside
        # delivered knowledge an empty layer tells the reader nothing.
        tail.append(f"no-match: {', '.join(sorted(str(layer) for layer in no_match))}")
    # Project knowledge first, with its text; then the past; then which
    # skills apply, by name.
    entries: list[tuple[str, dict[str, object], str]] = []
    kinds = {"semantic": "memory", "episodic": "history", "procedural": "skill"}
    for layer in ("semantic", "episodic", "procedural"):
        for item in capsule.get(layer) or []:
            if isinstance(item, dict):
                kind = kinds[layer]
                if layer == "procedural" and item.get("kind") == "policy":
                    kind = "policy"
                entries.append((layer, item, kind))
    excerpt_of = {
        id(item): excerpts.get(str(item.get("path", f"episode {item.get('id')}"))) for _, item, _ in entries
    }
    # Kept when shrinking drops the excerpt: it still says where to look.
    section_of = {
        key: getattr(text, "heading", "") for key, text in excerpt_of.items()
    }
    initial_excerpts = {key: str(text or "") for key, text in excerpt_of.items()} if trace is not None else {}

    def assemble() -> list[str]:
        lines = [*head, *([compaction] if compaction else []), *tail]
        if entries:
            lines.append(MEMORY_RENDER_HEADER)
        for _, item, kind in entries:
            lines.append(_item_line(item, kind, section_of.get(id(item)) or ""))
            text = excerpt_of.get(id(item))
            if text:
                lines.append(f"  {text}")
        return lines

    def size(lines: list[str]) -> int:
        return sum(len(line) + 1 for line in lines)

    lines = assemble()
    # Over the ceiling the excerpts shrink first, tail first, then go; then
    # the compaction line gives up its counts for COMPACTION_MARKER; then
    # skills, then related knowledge (a link brought it, not the query), then
    # the weakest knowledge, then the head's lines from the last. Working
    # state is cut only by those last drops: its own lines are bounded where
    # they are built. The marker is never dropped, and it goes in before the
    # first thing this loop drops, so a capsule the ceiling cut never reads as
    # a complete one. shown_in_render keeps the items one render shows, and
    # the capsule it leaves renders again with those items and a line that
    # now counts the dropped ones: that render fits them beside the same
    # marker, or beside the counted line where it fits, so the items recorded
    # are the ones the final text shows.
    while size(lines) > RENDERED_CAPSULE_LIMIT:
        hinted = [index for index, (_, item, _) in enumerate(entries) if item.get("selection") == "source-hint"]
        if hinted:
            entries.pop(hinted[-1])
            compaction = COMPACTION_MARKER
            lines = assemble()
            continue
        shrinkable = [
            key for key in reversed(list(excerpt_of)) if excerpt_of[key]
        ]
        if shrinkable:
            key = shrinkable[0]
            text = excerpt_of[key] or ""
            excerpt_of[key] = _bounded(text, len(text) // 2) if len(text) > 120 else None
        elif compaction != COMPACTION_MARKER:
            compaction = COMPACTION_MARKER
        elif any(layer == "procedural" for layer, _, _ in entries):
            index = max(
                position
                for position, (layer, _, _) in enumerate(entries)
                if layer == "procedural"
            )
            entries.pop(index)
        elif _related_positions([item for _, item, _ in entries]):
            entries.pop(_related_positions([item for _, item, _ in entries])[-1])
        elif entries:
            entries.pop()
        elif tail:
            tail.pop()
        elif len(head) > 1:
            head.pop()
        else:
            break
        lines = assemble()
    if trace is not None:
        encoded = "\n".join(lines).encode("utf-8")
        records = []
        for _, item, kind in entries:
            path = str(item.get("path", f"episode {item.get('id')}"))
            body = str(excerpt_of.get(id(item)) or "")
            heading = section_of.get(id(item)) or ""
            try:
                snippet = sanitize_automatic_query(str(item.get("snippet") or ""))
            except ContextError:
                snippet = ""
            header = _item_line(item, kind, heading)
            records.append({"path": path, "header_present": True,
                            "header_sha256": hashlib.sha256(header.encode("utf-8")).hexdigest(),
                            "body_bytes": len(body.encode("utf-8")),
                            "body_sha256": hashlib.sha256(body.encode("utf-8")).hexdigest(),
                            "pointer_only": not bool(body),
                            "initial_excerpt_bytes": len(initial_excerpts.get(id(item), "").encode("utf-8")),
                            "body_reason": "excerpt" if body else "render-limit" if initial_excerpts.get(id(item)) else "sanitized-empty" if excerpt_present.get(path) else "no-excerpt",
                            "section": heading if heading in ("Sources", "Goal", "Progress", "Files", "Next Steps") else "other" if heading else "none",
                            "section_sha256": hashlib.sha256(heading.encode("utf-8")).hexdigest(),
                            "snippet_sha256": hashlib.sha256(snippet.encode("utf-8")).hexdigest(),
                            "snippet_bytes": len(snippet.encode("utf-8")),
                            "snippet_in_body": bool(snippet) and snippet in body,
                            "shrunk": body != initial_excerpts.get(id(item), "")})
        trace.update({"sha256": hashlib.sha256(encoded).hexdigest(), "bytes": len(encoded),
                      "form": "capsule_text", "records": records})
    return lines, [item for _, item, _ in entries]


def print_capsule(
    capsule: dict[str, object], excerpts: Optional[dict[str, str]] = None,
    *, repository: Optional[Path] = None, delivery_profile: str = 'ordinary',
    _diagnosis_sink: Optional[Callable[[dict[str, object]], None]] = None,
) -> None:
    trace: dict[str, object] = {}
    lines = render_capsule_lines(capsule, excerpts, trace=trace if repository is not None else None,
                                 delivery_profile=delivery_profile)
    if repository is not None:
        emitted = ("\n".join(lines) + "\n").encode("utf-8")
        trace.update({"sha256": hashlib.sha256(emitted).hexdigest(), "bytes": len(emitted), "form": "printed_capsule"})
        record_render_trace(repository, capsule, trace)
        if _diagnosis_sink is not None:
            try:
                _diagnosis_sink(trace)
            except Exception:
                pass
    for line in lines:
        print(line)


def promote_on_resolution(
    repository: Path, record: dict[str, object], owner: str
) -> Optional[dict[str, object]]:
    """Promote a record the moment it reaches a promotable state.

    Promotion used to wait for the turn boundary, five turns and often days
    after the resolving update. By then the fix that resolved the finding had
    edited the files it cites, and the record never became durable memory:
    on a real project 2 of 70 resolved findings were ever served. Promoting on
    the resolving update digests the citations while they still say what was
    verified. A failure here never undoes the update; it is reported.
    """
    states = PROMOTABLE_STATES.get(str(record.get("type")))
    if (
        states is None
        or record.get("status") not in states
        or record.get("authority") != "verified"
    ):
        return None
    try:
        return auto_promote(repository, owner=owner, only={str(record["id"])})
    except (BrainError, OSError) as error:
        return {
            "enabled": True, "promoted": [], "blocked": [], "skipped": 0,
            "failed": [{"record_id": str(record.get("id")), "reason": str(error)}],
        }


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
    replace_next_steps: bool = False,
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
                replace_next_steps=replace_next_steps,
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
    # Git always describes the project: attached, the state directory is not
    # a checkout and the accelerator clone is not the work.
    try:
        result = subprocess.run(
            ["git", "-C", str(workspace_roots.project_root(repository)), *arguments],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            check=False,
        )
    except OSError as error:
        raise ContextError(f"{label} failed") from error
    if result.returncode != 0:
        raise ContextError(f"{label} failed with exit status {result.returncode}")
    return result.stdout


def _readiness_git_probe(
    repository: Path, arguments: list[str]
) -> tuple[Optional[str], Optional[str]]:
    """Run a bounded Git metadata probe without making status fail."""
    try:
        result = subprocess.run(
            ["git", "-C", str(workspace_roots.project_root(repository)), *arguments],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            check=False,
            timeout=2,
        )
    except FileNotFoundError:
        return None, "git-unavailable"
    except (OSError, subprocess.TimeoutExpired):
        return None, "git-probe-failed"
    if result.returncode != 0:
        return None, "not-a-worktree"
    return result.stdout.strip(), None


def _percentiles(values: list[float]) -> dict[str, object]:
    """p50/p95/max over the values that exist, nulls dropped rather than zeroed.

    A phase that did not run this turn is recorded as null, and coercing that
    to 0.0 would report an instantaneous index rather than an absent one.
    """
    present = sorted(value for value in values if isinstance(value, (int, float)))
    if not present:
        return {"samples": 0, "p50": None, "p95": None, "max": None}

    def at(fraction: float) -> float:
        index = min(len(present) - 1, int(round(fraction * (len(present) - 1))))
        return round(float(present[index]), 6)

    return {
        "samples": len(present),
        "p50": at(0.5),
        "p95": at(0.95),
        "max": round(float(present[-1]), 6),
    }


def read_manifests(repository: Path, scope: str, since: Optional[int]) -> list[dict]:
    """Manifests newest first, from the requested store or both.

    Lightweight mode writes none at all, which the caller reports as its own
    fact rather than as an empty aggregate.
    """
    directories = []
    if scope in ("governed", "all"):
        directories.append(brain_root(repository) / "control" / "retrieval-manifests")
    if scope in ("local", "all"):
        directories.append(repository / "memory-bank" / "local" / "retrieval-manifests")
    manifests: list[dict] = []
    for directory in directories:
        if not directory.is_dir():
            continue
        for path in directory.glob("*.json"):
            try:
                loaded = json.loads(path.read_text(encoding="utf-8"))
            except (OSError, ValueError):
                continue
            if isinstance(loaded, dict):
                manifests.append(loaded)
    manifests.sort(key=lambda item: str(item.get("created_at") or ""), reverse=True)
    return manifests[:since] if since else manifests


def retrieval_report(
    repository: Path, *, scope: str, since: Optional[int]
) -> dict[str, object]:
    """Aggregate what the retrieval manifests recorded, across turns.

    Manifests were written and never read: every number a gate decision needs
    existed for one turn and then only on disk. This is the reader.

    Two deliberate reporting choices. Turns are broken down by `query_source`,
    because a prompt, a task, a bare branch name and an operator's explicit
    query are not comparable and an undifferentiated average of them says
    nothing. And the top score is reported twice - `top_candidate` from the
    gate signals, before any policy filter, and `top_delivered` from the
    selection - because those diverge exactly when the best candidate was
    withheld, which is the case a report like this exists to surface.

    Version 1 manifests carry no gate, no phase timings, no query source and
    no per-item score. They are counted, and every metric says how many of the
    manifests could answer it, rather than silently averaging over a subset.
    """
    manifests = read_manifests(repository, scope, since)
    reasons: dict[str, int] = {}
    paths: dict[str, int] = {}
    sources: dict[str, int] = {}
    hosts: dict[str, int] = {}
    entry_points: dict[str, int] = {}
    gate_decisions: dict[str, int] = {}
    gate_skip_reasons: dict[str, int] = {}
    gate_slices: dict[tuple[str, str, str, str, str], int] = {}
    shadow_decided = 0
    enforce_decided = 0
    would_skip = 0
    withheld = 0
    no_match_counts: dict[str, int] = {}
    token_series: dict[str, list[float]] = {}
    matches: dict[str, int] = {}
    top_candidate: list[float] = []
    top_delivered: list[float] = []
    phases: dict[str, list[float]] = {"stat": [], "index": [], "retrieval": []}
    empty_selection = 0
    local_episode_count = 0
    versions: dict[str, int] = {}

    for manifest in manifests:
        version = str(manifest.get("schema_version"))
        versions[version] = versions.get(version, 0) + 1
        selected = manifest.get("selected") or []
        manifest_episode_count = manifest.get("local_episode_count", 0)
        if (
            isinstance(manifest_episode_count, int)
            and not isinstance(manifest_episode_count, bool)
            and manifest_episode_count > 0
        ):
            local_episode_count += manifest_episode_count
        else:
            manifest_episode_count = 0
        if not selected and not manifest_episode_count:
            empty_selection += 1
        best = None
        for item in selected:
            if not isinstance(item, dict):
                continue
            path = str(item.get("path", ""))
            if path:
                paths[path] = paths.get(path, 0) + 1
            strength = item.get("match")
            if isinstance(strength, str):
                matches[strength] = matches.get(strength, 0) + 1
            score = item.get("score")
            if isinstance(score, (int, float)):
                best = score if best is None else max(best, score)
        if best is not None:
            top_delivered.append(float(best))
        for item in manifest.get("excluded") or []:
            if isinstance(item, dict):
                reason = str(item.get("reason", "unknown"))
                reasons[reason] = reasons.get(reason, 0) + 1
        source = manifest.get("query_source")
        if isinstance(source, str):
            sources[source] = sources.get(source, 0) + 1
        host = manifest.get("host")
        if isinstance(host, str):
            hosts[host] = hosts.get(host, 0) + 1
        entry_point = manifest.get("entry_point")
        if isinstance(entry_point, str):
            entry_points[entry_point] = entry_points.get(entry_point, 0) + 1
        estimates = manifest.get("token_estimates")
        if isinstance(estimates, dict):
            for name, value in estimates.items():
                if isinstance(value, (int, float)) and not isinstance(value, bool):
                    token_series.setdefault(str(name), []).append(float(value))
        gate = manifest.get("gate")
        if isinstance(gate, dict):
            decision = str(gate.get("decision", "unknown"))
            gate_mode = str(gate.get("mode", "unknown"))
            if gate_mode in ("shadow", "enforce"):
                gate_decisions[decision] = gate_decisions.get(decision, 0) + 1
                if gate_mode == "shadow":
                    shadow_decided += 1
                    if decision == "skip":
                        would_skip += 1
                else:
                    enforce_decided += 1
                    if decision == "skip":
                        withheld += 1
                slice_key = (
                    gate_mode,
                    decision,
                    str(source or "unknown"),
                    str(host or "unknown"),
                    str(entry_point or "unknown"),
                )
                gate_slices[slice_key] = gate_slices.get(slice_key, 0) + 1
                if decision == "skip":
                    reason = str(gate.get("reason", "unknown"))
                    gate_skip_reasons[reason] = gate_skip_reasons.get(reason, 0) + 1
            signal = (gate.get("signals") or {}).get("top_score")
            if isinstance(signal, (int, float)):
                top_candidate.append(float(signal))
            for layer in (gate.get("signals") or {}).get("no_match") or []:
                if isinstance(layer, str):
                    no_match_counts[layer] = no_match_counts.get(layer, 0) + 1
        timings = manifest.get("phase_seconds")
        if isinstance(timings, dict):
            for name in phases:
                phases[name].append(timings.get(name))

    total = len(manifests)
    gated = sum(gate_decisions.values())
    return {
        "scope": scope,
        "turns": total,
        "schema_versions": dict(sorted(versions.items())),
        "query_source": dict(sorted(sources.items())),
        "host": dict(sorted(hosts.items())),
        "entry_point": dict(sorted(entry_points.items())),
        "empty_selection": empty_selection,
        "local_episode_count": local_episode_count,
        # Membership counted from the manifest, which records what the
        # delivered capsule carried after its character limits: the rendered
        # text for a refresh or a printed capsule, the JSON for `--json`.
        "top_paths": dict(
            sorted(paths.items(), key=lambda pair: (-pair[1], pair[0]))[:20]
        ),
        "excluded_reasons": dict(sorted(reasons.items())),
        "match_strength": dict(sorted(matches.items())),
        "gate": {
            "decided": gated,
            "decisions": dict(sorted(gate_decisions.items())),
            "skip_reasons": dict(sorted(gate_skip_reasons.items())),
            "shadow_decided": shadow_decided,
            "would_skip": would_skip,
            "would_skip_rate": (
                round(would_skip / shadow_decided, 4) if shadow_decided else None
            ),
            "enforce_decided": enforce_decided,
            "withheld": withheld,
            "withheld_rate": (
                round(withheld / enforce_decided, 4) if enforce_decided else None
            ),
            "slices": [
                {
                    "mode": gate_mode,
                    "decision": decision,
                    "query_source": source,
                    "host": host,
                    "entry_point": entry_point,
                    "turns": count,
                }
                for (gate_mode, decision, source, host, entry_point), count
                in sorted(gate_slices.items())
            ],
        },
        "token_estimates": {
            name: _percentiles(values) for name, values in sorted(token_series.items())
        },
        "no_match": dict(sorted(no_match_counts.items())),
        "top_candidate_score": _percentiles(top_candidate),
        "top_delivered_score": _percentiles(top_delivered),
        "phase_seconds": {name: _percentiles(values) for name, values in phases.items()},
    }


def funnel_diagnosis_summary(records: list[dict]) -> dict[str, Any]:
    """Fixed historical occurrence sums over at most 500 selected records."""
    selected = records[-500:]
    template = unknown_funnel_diagnosis()
    def counter():
        return {"known_records": 0, "unknown_records": 0, "saturated_records": 0,
                "sum_known_counts": None, "state": "unknown"}
    summary = {
        "summary_schema_version": 1, "records_considered": len(selected),
        "records_omitted_by_cap": max(0, len(records) - len(selected)),
        "availability": {key: 0 for key in ("observed", "unavailable", "missing", "rejected")},
        **{key: template[key] for key in ("source_coverage", "quality", "count_scope", "coverage", "emission", "consumer")},
        "loss_observations": {key: counter() for key in template["loss_observations"]},
        "boundaries": {key: counter() for key in template["boundaries"]},
        "render_projection": {"scope": template["render_projection"]["scope"],
                              "headers": counter(),
                              "reasons": {key: counter() for key in template["render_projection"]["reasons"]}},
    }
    def slots(value):
        return ([value["loss_observations"][key] for key in template["loss_observations"]]
                + [value["boundaries"][key] for key in template["boundaries"]]
                + [value["render_projection"]["headers"]]
                + [value["render_projection"]["reasons"][key] for key in template["render_projection"]["reasons"]])
    totals = slots(summary)
    for record in selected:
        child = record.get("funnel_diagnosis")
        if "funnel_diagnosis" not in record:
            availability = "missing"
        else:
            try:
                accepted = valid_funnel_diagnosis(child)
            except Exception:
                accepted = False
            availability = child["observation_state"] if accepted else "rejected"
        summary["availability"][availability] += 1
        observations = slots(child if availability == "observed" else template)
        for total, observation in zip(totals, observations):
            if observation["count"] is None:
                total["unknown_records"] += 1
            else:
                total["known_records"] += 1
                total["sum_known_counts"] = (total["sum_known_counts"] or 0) + observation["count"]
                total["saturated_records"] += int(observation["state"] == "lower-bound")
    for total in totals:
        if total["known_records"]:
            total["state"] = ("lower-bound" if total["unknown_records"] or total["saturated_records"] else "exact")
    return summary


def ordinary_receipt_summary(records: list[dict]) -> dict[str, int]:
    """Historical record counts, never attempt, durability or ACK counts."""
    selected = records[-500:]
    summary = {"summary_schema_version": 1, "records_considered": len(selected),
               "observed_records": 0, "unavailable_records": 0,
               "no_plan_skipped_records": 0, "planned_skipped_records": 0,
               "planned_deferred_records": 0, "planned_committed_records": 0}
    for record in selected:
        try:
            child = normalize_ordinary_receipt(record.get("ordinary_receipt"))
        except Exception:
            child = None
        if child is None or child["observation_state"] != "observed":
            summary["unavailable_records"] += 1
        else:
            summary["observed_records"] += 1
            bucket = ("planned_" + child["returned_status"] if child["plans_present"] else "no_plan_skipped")
            summary[bucket + "_records"] += 1
    return summary


def refresh_health(repository: Path, window: Optional[int]) -> dict[str, object]:
    """Aggregate the per-turn health series the hook and the CLI append.

    Reads only `refresh-health.ndjson`, never the manifests: the manifest's
    `retrieval` phase times the body of `retrieve()`, while the series records
    the whole refresh including the episodic slot and the capsule contract.
    H1-05 split those on purpose, and averaging them under one name would
    compare two different intervals.
    """
    path = refresh_health_path(repository)
    records: list[dict] = []
    if path.is_file():
        try:
            for line in path.read_text(encoding="utf-8").splitlines():
                if not line.strip():
                    continue
                try:
                    loaded = json.loads(line)
                except ValueError:
                    continue
                if isinstance(loaded, dict):
                    records.append(loaded)
        except OSError:
            records = []
    if window:
        records = records[-window:]
    phases: dict[str, list[float]] = {"stat": [], "index": [], "retrieval": []}
    timeouts = 0
    omitted_turns = 0
    incomplete_layers = 0
    statuses: dict[str, int] = {}
    source_notice_totals: dict[str, int] = {}
    source_notice_records = 0
    for record in records:
        notices = _normalized_source_notices(record.get("source_notices"))
        if notices:
            source_notice_records += 1
            for reason, count in notices.items():
                source_notice_totals[reason] = source_notice_totals.get(reason, 0) + count
        timings = record.get("phase_seconds")
        if isinstance(timings, dict):
            for name in phases:
                phases[name].append(timings.get(name))
        status = record.get("hook_status")
        if status is not None:
            statuses[str(status)] = statuses.get(str(status), 0) + 1
            if status == 124:
                timeouts += 1
        # "Non-empty omitted" means a positive counter: a lightweight capsule
        # always carries the keys with zeros, so testing the dict for
        # emptiness would fire on every turn.
        omitted = record.get("omitted")
        if isinstance(omitted, dict) and any(
            isinstance(value, int) and value > 0 for value in omitted.values()
        ):
            omitted_turns += 1
        if record.get("layers_updated") is False:
            incomplete_layers += 1
    total = len(records)
    ratio = lambda count: round(count / total, 4) if total else None
    result = {
        "turns": total,
        "phase_seconds": {name: _percentiles(values) for name, values in phases.items()},
        "hook_status": dict(sorted(statuses.items())),
        "timeout_rate": ratio(timeouts),
        "lossy_capsule_rate": ratio(omitted_turns),
        "incomplete_layers_rate": ratio(incomplete_layers),
        "funnel_diagnosis": funnel_diagnosis_summary(records),
    }
    if source_notice_records:
        # Historical reported occurrences, not unique sources or distinct turns.
        result["source_notices"] = {"records": source_notice_records,
                                    "totals": source_notice_totals}
    try:
        result["ordinary_receipt"] = ordinary_receipt_summary(records)
    except Exception:
        pass
    return result


def emit_refresh_telemetry(
    repository: Path, capsule: Optional[dict], retrieval_seconds: float
) -> None:
    """Give the telemetry adapter its first production caller, safely.

    `telemetry.py` was written, disabled by default, covered by five tests and
    never called from anywhere, so the one thing it could not report was
    whether it worked. Three constraints its contract imposes, each of which
    would otherwise break a turn:

    * it reads `project-brain/config/telemetry.json` BEFORE it checks whether
      telemetry is enabled, and raises when that file is absent - the normal
      state of a lightweight-mode or partially installed project;
    * `task_id` must be a UUID, and the refresh branch's `--task-id` is an
      external id or a branch name; the UUID exists only on a governed
      capsule;
    * `context_tokens` exists only as the capsule's token estimate, which a
      refresh without `--query` never produces.

    Imported lazily and swallowed entirely: telemetry is off by default and
    must never be the reason a prompt fails.
    """
    try:
        import telemetry
    except ImportError:
        return
    payload: dict[str, object] = {
        "operation": "refresh",
        "provider": str(load_config(repository).get("provider") or "sqlite-fts5"),
        "ttfr_ms": max(0, int(round(retrieval_seconds * 1000))),
    }
    if isinstance(capsule, dict):
        task_uuid = capsule.get("task_uuid")
        if isinstance(task_uuid, str) and task_uuid:
            payload["task_id"] = task_uuid
        estimates = capsule.get("token_estimates")
        if isinstance(estimates, dict) and isinstance(estimates.get("total"), int):
            payload["context_tokens"] = estimates["total"]
    try:
        event = telemetry.write_metadata_event(repository, payload)
    except (telemetry.TelemetryError, OSError, ValueError):
        return
    if event is None:
        return
    try:
        validate_schema_file(
            repository,
            "token-usage-event.schema.json",
            json.loads(event.read_text(encoding="utf-8")),
        )
    except (BrainError, OSError, ValueError):
        # Validation belongs to the caller: telemetry.py has no Brain coupling
        # by design and keeping it that way is worth more than a hard failure
        # on a channel that is off by default.
        return


def consolidation_counters(repository: Path, mode: str) -> dict[str, object]:
    """How much the promotion pipeline has actually carried.

    "Nothing was ever consolidated" and "consolidation ran and had nothing to
    do" produced identical output, which is how a pipeline can be built,
    tested and left with no input for nine days without anyone noticing.

    Every count is `None` rather than 0 when it cannot be established, so an
    absent Project Brain stays distinguishable from an empty one. Walking the
    Brain costs a hash per cited source, and `status` is on the session-start
    hook's budget, so any failure degrades instead of propagating.
    """
    counters: dict[str, object] = {
        "promotable": None,
        "blocked": None,
        "applied": None,
        "chunks": None,
    }
    if mode == "lightweight" or not (repository / "project-brain").is_dir():
        return counters
    try:
        config = load_config(repository)
        promotable, blocked = promotable_records(repository, config)
        counters["promotable"] = len(promotable)
        counters["blocked"] = len(blocked)
        counters["applied"] = sum(
            1
            for _, promotion in iter_promotions(repository)
            if promotion.get("status") == "applied"
        )
    except (BrainError, OSError, ValueError):
        return counters
    try:
        counters["chunks"] = render_bank_index(repository / "memory-bank")[1]
    except (BrainError, OSError, ValueError):
        counters["chunks"] = None
    return counters


def automatic_memory_readiness(repository: Path) -> dict[str, object]:
    """Report task identity and Git readiness without mutating runtime state."""
    worktree, git_error = _readiness_git_probe(
        repository, ["rev-parse", "--show-toplevel"]
    )
    branch: Optional[str] = None
    git_state = git_error or "worktree"
    probe_failures = ("git-unavailable", "git-probe-failed")
    failed_probe: Optional[str] = None
    if git_error is None:
        branch, branch_error = _readiness_git_probe(
            repository, ["symbolic-ref", "--quiet", "--short", "HEAD"]
        )
        if branch_error in probe_failures:
            # A transient probe failure says nothing about HEAD, so report the
            # failed probe itself instead of guessing detached or unborn.
            branch = None
            git_state = branch_error
            failed_probe = "branch"
        else:
            head, head_error = _readiness_git_probe(
                repository, ["rev-parse", "--verify", "HEAD"]
            )
            if head_error in probe_failures:
                if branch_error is None:
                    # The branch probe already supplied a usable identity; a
                    # transient HEAD-classification failure must not discard
                    # it, so keep the pre-classification worktree report.
                    git_state = "worktree"
                else:
                    branch = None
                    git_state = head_error
                    failed_probe = "HEAD-classification"
            elif head_error is None and head:
                git_state = "worktree" if branch_error is None else "detached-head"
            else:
                # HEAD does not resolve to a commit: the branch is unborn
                # (fresh init, zero commits) or HEAD itself is broken.
                git_state = "unborn-head"

    if git_error is not None:
        git_metadata = {
            "status": "degraded",
            "state": git_state,
            "worktree": None,
            "branch": None,
            "reason": "Git changed-path metadata is unavailable; turn checkpointing cannot run.",
            "remediation": "Run the accelerator inside its Git worktree.",
        }
    elif git_state in probe_failures:
        git_metadata = {
            "status": "degraded",
            "state": git_state,
            "worktree": worktree,
            "branch": None,
            "reason": {
                "branch": (
                    "The Git branch probe failed before HEAD could be "
                    "classified, so branch state is unknown for this report."
                ),
                "HEAD-classification": (
                    "The Git HEAD-classification probe failed after no "
                    "branch was found, so HEAD state is unknown for this "
                    "report."
                ),
            }[failed_probe],
            "remediation": (
                "Retry when Git responds within the probe timeout, or set "
                "CONTEXT_TASK_ID for task-aware automation."
            ),
        }
    else:
        git_metadata = {
            "status": "active",
            "state": git_state,
            "worktree": worktree,
            "branch": branch,
            "reason": {
                "worktree": "Git changed-path metadata is available.",
                "detached-head": (
                    "Git changed-path metadata is available, but HEAD is "
                    "detached and cannot supply task identity."
                ),
                "unborn-head": (
                    "Git changed-path metadata is available, but HEAD does "
                    "not resolve to a commit yet (unborn branch)."
                ),
            }[git_state],
            "remediation": (
                "Set CONTEXT_TASK_ID for task-aware automation while HEAD is detached."
                if git_state == "detached-head"
                else None
            ),
        }

    explicit_identity = os.environ.get("CONTEXT_TASK_ID", "").strip()
    if explicit_identity:
        try:
            identity = validate_task_id(explicit_identity)
        except ContextError:
            task_identity = {
                "status": "degraded",
                "source": "environment",
                "task_id": None,
                "reason": "CONTEXT_TASK_ID is present but invalid.",
                "remediation": "Set CONTEXT_TASK_ID to a valid task identifier.",
            }
        else:
            task_identity = {
                "status": "active",
                "source": "environment",
                "task_id": identity,
                "reason": "Explicit task identity is available for retrieval and dispatch.",
                "remediation": None,
            }
    elif branch:
        try:
            identity = validate_task_id(branch_task_id(branch))
        except ContextError:
            task_identity = {
                "status": "degraded",
                "source": "git-branch",
                "task_id": None,
                "reason": "The current Git branch is not a valid task identifier.",
                "remediation": "Set CONTEXT_TASK_ID to a valid task identifier.",
            }
        else:
            task_identity = {
                "status": "active",
                "source": "git-branch",
                "task_id": identity,
                "reason": "The current Git branch supplies task identity.",
                "remediation": None,
            }
    else:
        task_identity = {
            "status": "degraded",
            "source": None,
            "task_id": None,
            "reason": "No task identity is available for task-aware retrieval, writes, or dispatch.",
            "remediation": (
                "Set CONTEXT_TASK_ID explicitly."
                if git_error is None
                else "Set CONTEXT_TASK_ID, and use a Git worktree to enable turn checkpointing."
            ),
        }

    identity_active = task_identity["status"] == "active"
    git_active = git_metadata["status"] == "active"
    if identity_active and git_active:
        overall = {
            "status": "active",
            "reason": "Task identity and Git metadata are available; automatic memory is active.",
            "remediation": None,
        }
    elif identity_active:
        overall = {
            "status": "retrieval-only",
            "reason": (
                "Task identity supports retrieval and dispatch, but Git metadata is "
                "unavailable so turn checkpointing is disabled."
            ),
            "remediation": "Run the accelerator inside its Git worktree.",
        }
    else:
        overall = {
            "status": "degraded",
            "reason": (
                "Automatic task-aware memory is degraded because no usable task "
                "identity is available."
            ),
            "remediation": task_identity["remediation"],
        }
    overall["task_identity"] = task_identity
    overall["git_metadata"] = git_metadata
    return overall


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
        try:
            task = create_task(repository, task_id, goal, files, sources, owner=owner)
        except BrainError as error:
            # A finished task keeps its ID in dynamic/ until compaction
            # archives it - automatic compaction waits for five finished
            # records - and a new task for the same branch was refused. That
            # finished record, and no other, is archived now.
            if "already exists" not in str(error) or not _finished_records_only(repository, task_id):
                raise
            compact(repository, only_external_id=task_id)
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


def _closed_task_remedy(task_id: str) -> str:
    """How a branch whose task is finished records again: a new task, which
    moves the finished record to the archive."""
    return "start a new task: `context.py start --task-id " + (task_id or "<task>") + ' --goal "..."`'


def _finished_records_only(repository: Path, task_id: str) -> bool:
    """Whether every Brain record that names ``task_id`` is a finished task,
    so a new task of that name may move them to the archive."""
    problems: list[dict[str, Any]] = []
    named = [record for _, record, _ in iter_records(repository, problems=problems)
             if task_id in (record.get("id"), record.get("external_id"))]
    if any(problem["identity"] is not None and task_id in problem["identity"][1:] for problem in problems):
        return False
    return bool(named) and all(record.get("type") == "task" and record.get("status") in TERMINAL_STATES
                               for record in named)


def closed_task_record(
    connection: sqlite3.Connection, repository: Path, task_id: str
) -> Optional[str]:
    """The id of this task id's completed or cancelled record, if that is all
    there is: no live binding, and a terminal Brain record of the same name."""
    try:
        governed_binding(connection, task_id)
        return None
    except ContextError:
        pass
    try:
        record = get_record(repository, task_id)
    except BrainError:
        return None
    if record.get("type") == "task" and record.get("status") in TERMINAL_STATES:
        return str(record["id"])
    return None


def ensure_working_task(
    connection: sqlite3.Connection,
    repository: Path,
    task_id: str,
    mode: str,
    owner: str,
    *, _created_sink: Optional[list[dict[str, object]]] = None,
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
        if _created_sink is not None:
            _created_sink.append({'task_uuid':None, 'generation':get_working_task(connection,task_id)['created_at']})
        return {"provisioned": True, "rebound": None}
    try:
        binding = governed_binding(connection, task_id)
    except ContextError:
        binding = None
    if binding is not None:
        problem, record = _bound_record(repository, binding)
        if problem != "missing":
            successor = _successor(repository, binding, record)
            if successor is None:
                return {"provisioned": False, "rebound": None}
            # The bound record was finished while another record took over
            # its task ID - the duplicate a merge left was cancelled here, on
            # this machine's side. Follow the task ID instead of dropping
            # every later turn as work on a closed task.
            with connection:
                connection.execute("DELETE FROM task_bindings WHERE task_uuid = ?", (binding["task_uuid"],))
                connection.execute("DELETE FROM working_tasks WHERE task_id = ? AND goal = ?",
                                   (binding["external_id"], binding["task_uuid"]))
            task = rebind_governed_task(connection, repository, task_id, str(successor["id"]))
            return {"provisioned": False, "rebound": str(task["task_uuid"])}
        # The record the binding names is gone - `git stash -u` or `git clean`
        # takes an uncommitted task with it. Drop the dead pointer and
        # provision as on a first flush: every later Stop failed with "record
        # not found" while status still reported memory as active.
        with connection:
            connection.execute("DELETE FROM task_bindings WHERE task_uuid = ?", (binding["task_uuid"],))
            # Its compatibility row names the same record (goal = task UUID).
            connection.execute("DELETE FROM working_tasks WHERE task_id = ? AND goal = ?",
                               (binding["external_id"], binding["task_uuid"]))
    try:
        task = rebind_governed_task(connection, repository, task_id)
    except ContextError as error:
        # Only a genuinely absent record means "first flush on this branch";
        # every other rebind refusal (terminal task, ambiguous identifier)
        # would also fail creation and is clearer stated as what it is.
        absence = error.__cause__
        if not (isinstance(absence, BrainRecordNotFound)
                and absence.identifier == task_id and absence.record_type is None):
            claimant = (_active_claimant(repository, task_id)
                        if isinstance(absence, BrainError) and "ambiguous" in str(absence) else None)
            if claimant is None:
                raise
            task = rebind_governed_task(connection, repository, task_id, str(claimant["id"]))
            return {"provisioned": False, "rebound": str(task["task_uuid"])}
        created = create_governed_task(connection, repository, task_id, goal, [], [], owner)
        if _created_sink is not None:
            _created_sink.append({'task_uuid':created['id'], 'generation':get_working_task(connection,task_id)['created_at']})
        return {"provisioned": True, "rebound": None}
    return {"provisioned": False, "rebound": str(task["task_uuid"])}


def _active_claimant(repository: Path, task_id: str) -> Optional[dict[str, Any]]:
    """The task record a clone without a binding works with when more than
    one record claims the task ID, or None to keep the refusal.

    Two copies of a branch that each started its task record one each; once
    they merge, the ID names both. Every checkpoint of a clone without a
    binding then failed as ambiguous - invisibly, the Stop hook hides it -
    and the branch's memory stopped for everyone new to it. The earliest
    created active record is chosen, so every clone settles on the same one;
    a record claiming the ID that cannot be read keeps the refusal, and so
    does a set of only finished tasks.
    """
    active = _active_claimants(repository, task_id)
    return active[0] if active else None


def _active_claimants(repository: Path, task_id: str) -> Optional[list[dict[str, Any]]]:
    """The active task records claiming a task ID, earliest created first;
    None when a record claiming it cannot be read."""
    problems: list[dict[str, Any]] = []
    claimants = [record for _, record, _ in iter_records(repository, problems=problems)
                 if record["type"] == "task" and record["external_id"] == task_id]
    if any(problem["identity"] is not None and task_id in problem["identity"][1:]
           for problem in problems):
        return None
    return sorted((record for record in claimants if record.get("status") not in TERMINAL_STATES),
                  key=lambda record: (str(record.get("created_at") or ""), str(record["id"])))


def _successor(repository: Path, binding: sqlite3.Row, record: Optional[dict[str, Any]]) -> Optional[dict[str, Any]]:
    """The active record that took over the task ID of a bound record that is
    finished - a duplicate cancelled after two copies of the branch merged -
    or None. A finished task with no active record beside it keeps its
    binding: the turns after it are dropped, as before."""
    if record is None or record.get("status") not in TERMINAL_STATES:
        return None
    active = _active_claimants(repository, str(record["external_id"])) or []
    return next((candidate for candidate in active if candidate["id"] != record["id"]), None)


def _unbound_task(repository: Path, task_id: str) -> Optional[dict[str, Any]]:
    """The active task record a task ID names on a machine that holds no
    binding for it yet - a fresh clone or a second machine - or None."""
    try:
        _, record, _ = find_task(repository, task_id)
    except BrainRecordNotFound:
        return None
    except BrainError as error:
        record = _active_claimant(repository, task_id) if "ambiguous" in str(error) else None
    if record is None or record.get("status") in TERMINAL_STATES:
        return None
    return record


def _twin_notices(connection: sqlite3.Connection, external_id: str, served: str) -> list[str]:
    """Say that another active record claims the same task ID, and how to
    keep one; two lines, as the capsule bounds each warning."""
    try:
        twins = json.loads(load_index_state(connection).get(TASK_TWINS_KEY) or "{}")
    except (ValueError, TypeError, sqlite3.Error):
        return []
    claimed = twins.get(external_id) if isinstance(twins, dict) else None
    if not isinstance(claimed, list) or served not in claimed or len(claimed) < 2:
        return []
    other = next(str(identifier) for identifier in claimed if identifier != served)
    return [f"Task record {other} also claims {external_id}; this turn uses {served}.",
            f"Keep one task per branch: context.py brain-update --record-id {other} "
            "--revision auto --transition cancelled"]


def _bound_record(repository: Path, binding: sqlite3.Row) -> tuple[Optional[str], Optional[dict[str, Any]]]:
    """What is wrong with the task record a local binding names, if anything,
    and the record: "missing" when it is gone from the Brain, archive
    included, or the refusal that reading it gives. An invalid or unreadable
    record is there, not missing: re-provisioning beside it would make a
    duplicate."""
    uuid = str(binding["task_uuid"])
    present = any(os.path.lexists(repository / "project-brain" / folder / "tasks" / f"{uuid}.md")
                  for folder in ("dynamic", "archive"))
    try:
        _, record, _ = find_record(repository, binding["task_uuid"], record_type="task", include_archive=True)
    except BrainRecordNotFound:
        return (None if present else "missing"), None
    except BrainError as error:
        return str(error), None
    return None, record


def _bound_record_problem(repository: Path, binding: sqlite3.Row) -> Optional[str]:
    return _bound_record(repository, binding)[0]


def _bound_record_missing(repository: Path, binding: sqlite3.Row) -> bool:
    return _bound_record_problem(repository, binding) == "missing"


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
            event = record_completion_event(
                repository, task, outcome, verification, owner
            )
            if event is not None:
                # The snapshot predates the event, so it has no entry for it;
                # marked absent, the restore below removes it with the rest
                # rather than leave "Completed" beside a reopened task.
                created = dynamic_path(repository, {"type": "event", "id": event})
                snapshot[created] = None
            connection.commit()
        except Exception:
            connection.rollback()
            restore_record_state(repository, snapshot)
            raise
    return {
        "episode_id": episode_id,
        "event_id": event,
        "task_uuid": completed["id"],
        "revision": completed["revision"],
        "status": completed["status"],
    }


def record_completion_event(
    repository: Path,
    task: dict[str, object],
    outcome: str,
    verification: list[str],
    owner: str,
) -> Optional[str]:
    """Write the completed task as a git-tracked episodic record.

    The episodic layer had exactly one Git-tracked source - the changelog -
    while completed work lived only in the disposable local database, so the
    one layer meant to hold "what happened here" could not survive a fresh
    clone. An `event` is the right carrier: it is the record type that reports
    that something happened rather than what to do about it, and it is
    excluded from promotion for that reason, so this is not the engine
    inventing knowledge - it fixes a lifecycle transition that occurred.

    Best-effort: a task that completed must not be reopened because its
    episode could not be written. The failure is reported, not raised.
    """
    external_id = f"EVENT-{str(task['id']).replace('-', '')[:12]}"
    goal_parts = [outcome.strip()]
    if verification:
        goal_parts.append("Verification: " + "; ".join(verification))
    try:
        # `create_record` fingerprints every cited source and raises on one
        # that no longer resolves. A completed task routinely cites files a
        # refactor removed, so only what still exists is carried over.
        sources = [
            source
            for source in list(task.get("sources") or [])
            if isinstance(source, str)
            and workspace_roots.resolve(repository, source).is_file()
        ]
        event = create_record(
            repository,
            "event",
            # Derived from the task UUID, not its external id: a branch name
            # is reused, and events are never archived, so a slug-derived id
            # would collide the second time round and fail the completion.
            external_id,
            f"Completed {task['external_id']}: {str(task['title'])[:80]}",
            [],
            sources,
            owner=owner,
            goal=" ".join(part for part in goal_parts if part),
        )
    except (BrainError, ContextError, OSError) as error:
        # Silent by design: the caller's own report is the channel, and a
        # completed task must never be reopened because its episode failed.
        del error
        return None
    return str(event["id"])


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
        rf"^project-brain/({'|'.join(TURN_RUNTIME_DIRECTORIES)})/"
    )
    fields = git_output(
        repository,
        [
            "status", "--porcelain=v1", "-z", "--untracked-files=normal", "--", ".",
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
        if prefix and path.startswith(prefix):
            path = path[len(prefix):]
        if path != "project-brain/" and not runtime_owned.match(path):
            paths.append(path)
    excluded = sorted({item for item in paths if TURN_PATH_DENYLIST.search(item)})
    allowed = [item for item in dict.fromkeys(paths) if item not in set(excluded)]
    return allowed, excluded

# What one shell command does to the files it names. Measured reads: a turn's
# files were named far more often through cat, sed -n, grep and git diff than
# through the Read tool. A command that only runs something names none.
_SHELL_READ_VERBS = frozenset((
    "cat", "head", "tail", "sed", "awk", "nl", "less", "more", "bat", "grep", "egrep",
    "fgrep", "rg", "ag", "ack", "wc", "diff", "cmp", "jq", "yq", "xxd", "od", "strings",
    "cut", "sort", "uniq", "column",
))
# Every file these name is changed. sed and perl change files only in place
# (-i); a copy or a move changes only its last operand and reads the others.
_SHELL_WRITE_VERBS = frozenset(("tee", "rm", "touch", "truncate", "chmod"))
_SHELL_COPY_VERBS = frozenset(("cp", "ln", "install", "mv"))
# Codex runs PowerShell on native Windows, and through it cmd.exe: cmdlets,
# their aliases and the cmd commands, matched without case. The POSIX names
# above are PowerShell aliases too.
_POWERSHELL_READ_VERBS = frozenset(("get-content", "gc", "type", "select-string", "sls", "format-hex", "findstr"))
_POWERSHELL_WRITE_VERBS = frozenset((
    "set-content", "sc", "add-content", "ac", "clear-content", "clc", "out-file", "new-item", "ni",
    "remove-item", "ri", "del", "erase", "tee-object",
))
_POWERSHELL_COPY_VERBS = frozenset((
    "copy-item", "copy", "cpi", "move-item", "move", "mi", "rename-item", "rni", "ren",
))
# Commands that run the command after them, their options, and the commands
# that move the directory later words are relative to.
_SHELL_WRAPPERS = frozenset(("sudo", "time", "nice", "nohup", "exec", "command", "env", "xargs", "timeout"))
_SHELL_DIRECTORY_VERBS = frozenset(("cd", "pushd", "chdir", "set-location", "sl", "push-location"))
_GIT_READ_SUBCOMMANDS = frozenset(("show", "diff", "log", "blame", "grep", "cat-file"))
_GIT_WRITE_SUBCOMMANDS = frozenset((
    "add", "commit", "mv", "rm", "checkout", "restore", "apply", "stash", "reset", "switch",
))
# The characters of the operators that separate commands and redirect them. A
# command used to be split on `|` and `;` as text, so a quoted grep
# alternation, sed script or jq filter cut the command in two and its file
# was never named: `rg "a|b" src/x.php` read nothing.
_SHELL_PUNCTUATION = "();<>|&\n"
# Redirections, by what the word after them is: a file written, a file read,
# or what to pass over - a here-document delimiter, a here-string, and the
# descriptor of `>&2`, which `>&file` writes instead.
_SHELL_WRITE_REDIRECTS = frozenset((">", ">>", ">|", "&>", "&>>"))
_SHELL_READ_REDIRECTS = frozenset(("<", "<>"))
_SHELL_SKIPPED_REDIRECTS = frozenset(("<<", "<<-", "<<<"))
_SHELL_DUPLICATIONS = frozenset((">&", "<&"))
_SHELL_ASSIGNMENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*=")
_SHELL_DURATION = re.compile(r"\d+(?:\.\d+)?[smhd]?")
# perl edits in place under -i, alone or after argument-free switches
# (-pi, -0pi, -i.bak); -Mstrict and -I are not it.
_PERL_IN_PLACE = re.compile(r"-[0-9pnlaswWXtTUc]*i.*")
_HEREDOC_OPERATOR = re.compile(r"(?<!<)<<(-?)[ \t]*(['\"]?)([A-Za-z_][A-Za-z0-9_]*)\2")
_POSIX_SHELLS = frozenset(("bash", "sh", "zsh", "dash"))
_WINDOWS_DIRECTORY = re.compile(r"^[A-Za-z]:[\\/]")
_PATCH_FILE_LINE = re.compile(r"^\*\*\* (?:Update|Add|Delete) File: (.+)$|^\*\*\* Move to: (.+)$", re.M)
_RECENT_RUNTIME_OWNED = re.compile(rf"^project-brain/({'|'.join(TURN_RUNTIME_DIRECTORIES)})/")


def _without_heredoc_bodies(command: str) -> str:
    """The command with each here-document's body left out: data, not commands.

    A delimiter whose closing line never comes is text inside a quoted string,
    and nothing after it is left out."""
    lines = command.split("\n")
    kept: list[str] = []
    index = 0
    while index < len(lines):
        line = lines[index]
        kept.append(line)
        index += 1
        for match in _HEREDOC_OPERATOR.finditer(line):
            strip_tabs, delimiter = match.group(1) == "-", match.group(3)
            end = next(
                (number for number in range(index, len(lines))
                 if (lines[number].lstrip("\t") if strip_tabs else lines[number]) == delimiter),
                None,
            )
            if end is not None:
                index = end + 1
    return "\n".join(kept)


def _shell_tokens(command: str, *, posix: bool) -> list[tuple[str, bool]]:
    """The words and operators of a command, as (text, is_operator).

    Quoting keeps a `|`, `;` or `>` inside an argument - a grep alternation,
    a sed or jq script, `cut -d '|'` - part of the word, and a newline
    separates commands. A quote left open closes at the end. ``posix=False``
    reads PowerShell and cmd, where a backslash is a path separator."""
    tokens: list[tuple[str, bool]] = []
    word, operator = "", ""
    in_word = False
    quote: Optional[str] = None
    index = 0

    def end_word() -> None:
        nonlocal word, in_word
        if in_word:
            tokens.append((word, False))
        word, in_word = "", False

    def end_operator() -> None:
        nonlocal operator
        if operator:
            tokens.append((operator, True))
        operator = ""

    while index < len(command):
        char = command[index]
        following = command[index + 1] if index + 1 < len(command) else ""
        index += 1
        if quote is not None:
            if char == quote:
                quote = None
            elif posix and quote == '"' and char == "\\" and following in '$`"\\\n':
                word += following if following != "\n" else ""
                index += 1
            else:
                word += char
            continue
        if posix and char == "\\":
            end_operator()
            if following != "\n":
                word, in_word = word + following, True
            index += 1
        elif char in "'\"":
            end_operator()
            quote, in_word = char, True
        elif char in _SHELL_PUNCTUATION:
            end_word()
            operator += char
        elif char in " \t\r":
            end_word()
            end_operator()
        else:
            end_operator()
            word, in_word = word + char, True
    end_word()
    end_operator()
    return tokens


def _shell_command_files(
    words: list[str], base: str, dialect: str
) -> tuple[Optional[str], list[tuple[str, bool, str]]]:
    """What one command does: the directory a `cd` moves to, else None, and
    (word, changed, directory) for the words that may name a file it reads
    or changes."""
    words = list(words)
    fold = (lambda word: word) if dialect == "sh" else (lambda word: word.lower())

    def drop_assignments() -> None:
        while words and _SHELL_ASSIGNMENT.match(words[0]):
            words.pop(0)

    drop_assignments()
    while words and fold(words[0]) in _SHELL_WRAPPERS:
        words.pop(0)
        while words and (words[0].startswith("-") or _SHELL_DURATION.fullmatch(words[0])):
            words.pop(0)
        drop_assignments()
    if not words:
        return None, []
    verb = fold(words[0].replace("\\", "/").rsplit("/", 1)[-1])
    if dialect != "sh" and verb.endswith(".exe"):
        verb = verb[:-4]
    operands = [word for word in words[1:] if not word.startswith("-")]
    if verb in _SHELL_DIRECTORY_VERBS:
        target = operands[0].strip("'\"") if operands else "~"
        if target == "-" or "$" in target or "`" in target:
            # Somewhere only the shell knows: the words stay where they were.
            return base, []
        target = os.path.expanduser(target)
        return os.path.normpath(target if os.path.isabs(target) else os.path.join(base, target)), []
    if verb == "git":
        rest, directory = words[1:], base
        while rest and rest[0].startswith("-"):
            if rest[0] == "-C" and len(rest) > 1:
                value = os.path.expanduser(rest[1].strip("'\""))
                directory = value if os.path.isabs(value) else os.path.join(directory, value)
            rest = rest[2:] if rest[0] in ("-C", "-c") else rest[1:]
        subcommand = rest[0] if rest else ""
        if subcommand in _GIT_READ_SUBCOMMANDS or subcommand in _GIT_WRITE_SUBCOMMANDS:
            changed = subcommand in _GIT_WRITE_SUBCOMMANDS
            return None, [(word, changed, directory) for word in rest[1:]]
        return None, []
    if verb == "sed":
        in_place = any(word.startswith(("-i", "--in-place")) for word in words[1:])
        return None, [(word, in_place, base) for word in words[1:]]
    if verb == "perl":
        if any(_PERL_IN_PLACE.fullmatch(word) for word in words[1:]):
            return None, [(word, True, base) for word in words[1:]]
        return None, []
    if verb in _SHELL_COPY_VERBS or (dialect != "sh" and verb in _POWERSHELL_COPY_VERBS):
        if any(word in ("-t", "--target-directory") or word.startswith("--target-directory=")
               for word in words[1:]):
            return None, [(word, False, base) for word in operands]
        last = len(operands) - 1
        return None, [(word, index == last, base) for index, word in enumerate(operands)]
    if verb in _SHELL_WRITE_VERBS or (dialect != "sh" and verb in _POWERSHELL_WRITE_VERBS):
        return None, [(word, True, base) for word in words[1:]]
    if verb in _SHELL_READ_VERBS or (dialect != "sh" and verb in _POWERSHELL_READ_VERBS):
        return None, [(word, False, base) for word in words[1:]]
    return None, []


def _shell_file_words(command: str, base: str, *, dialect: str = "sh") -> list[tuple[str, bool, str]]:
    """(word, changed, directory) for the words of a shell command that may
    name a file it reads or changes, each with the directory it is relative to.

    A `cd` moves the words of the commands after it, up to the parenthesis
    that closes around it; `git -C` moves the words of its own command. A
    redirection's file is named whatever runs: `php x.php > out.json` writes
    out.json. ``dialect`` is "sh", or "powershell" for PowerShell and cmd."""
    found: list[tuple[str, bool, str]] = []
    if dialect == "sh":
        command = _without_heredoc_bodies(command)
    words: list[str] = []
    # The command a parenthesis interrupts - `$(...)`, `<(...)`, a subshell -
    # and its directory, resumed at the closing one.
    outer: list[tuple[str, list[str]]] = []
    # What the next word is, after a redirection: "write", "read", "skip",
    # or "duplicate" - a descriptor to pass over, or else a file written.
    pending: Optional[str] = None

    def finish() -> None:
        nonlocal base, words
        if words:
            moved, named = _shell_command_files(words, base, dialect)
            found.extend(named)
            if moved is not None:
                base = moved
        words = []

    def close() -> None:
        nonlocal base, words
        finish()
        if outer:
            base, words = outer.pop()

    for token, is_operator in _shell_tokens(command, posix=dialect == "sh"):
        if not is_operator:
            if pending is None:
                words.append(token)
            elif pending == "write" or (pending == "duplicate" and not (token.isdigit() or token == "-")):
                found.append((token, True, base))
            elif pending == "read":
                found.append((token, False, base))
            pending = None
            continue
        operator = token.replace("(", "").replace(")", "")
        redirect = (
            "write" if operator in _SHELL_WRITE_REDIRECTS
            else "read" if operator in _SHELL_READ_REDIRECTS
            else "skip" if operator in _SHELL_SKIPPED_REDIRECTS
            else "duplicate" if operator in _SHELL_DUPLICATIONS
            else None
        )
        if redirect is not None and "(" not in token:
            for _ in range(token.count(")")):
                close()
            if words and words[-1].isdigit():
                # The descriptor of `2>`, not an argument.
                words.pop()
            pending = redirect
            continue
        # Anything else separates commands - ; & && | || |& a newline - or
        # opens and closes a parenthesis; `<(` and `>(` are a command too.
        pending = None
        for char in token:
            if char == "(":
                outer.append((base, words))
                words = []
            elif char == ")":
                close()
            elif char in ";|&\n":
                finish()
    finish()
    return found


def _shell_script(command: object, workdir: Optional[str] = None) -> Optional[tuple[str, str]]:
    """(dialect, script) for a recorded shell command. Codex records an argv
    - a `bash -lc` script, or on native Windows a PowerShell or cmd one - or
    one command line, which on Windows its PowerShell runs."""
    if isinstance(command, str):
        windows = isinstance(workdir, str) and _WINDOWS_DIRECTORY.match(workdir)
        return ("powershell" if windows else "sh"), command
    if not (isinstance(command, list) and command and all(isinstance(word, str) for word in command)):
        return None
    program = command[0].replace("\\", "/").rsplit("/", 1)[-1].lower()
    if program.endswith(".exe"):
        program = program[:-4]
    flags = [word.lower() for word in command]
    if program in _POSIX_SHELLS and len(command) >= 3 and command[1] in ("-c", "-lc"):
        return "sh", command[-1]
    if program in ("powershell", "pwsh"):
        for flag in ("-command", "-c"):
            if flag in flags[1:]:
                return "powershell", " ".join(command[flags.index(flag, 1) + 1:])
        return None
    if program == "cmd":
        return ("powershell", " ".join(command[flags.index("/c", 1) + 1:])) if "/c" in flags[1:] else None
    return "sh", shlex.join(command)


# What a host writes into the conversation as the user without starting a
# turn: an interruption marker, a local command's or the `!` shell's output
# and the caveat beside it, and Codex's environment and instructions. Any
# other message starts the turn the Stop hook ends - a prompt, and also text
# that opens with a tag: a slash command (`<command-name>`), pasted text
# (`<pasted_content>`), a background task's `<task-notification>`. Taking
# every `<` for the host's own text merged those turns into the one before,
# and its files came back as the newest; on 400 recent Claude Code
# transcripts 132 task notifications went on to call tools.
_TRANSCRIPT_PASSIVE_PREFIXES = (
    "[Request interrupted", "<local-command-stdout", "<local-command-stderr",
    "<local-command-caveat", "<bash-stdout", "<bash-stderr",
    "<environment_context", "<user_instructions",
)


def _transcript_prompt(record: dict[str, object]) -> bool:
    """A message that starts a turn: where the transcript's latest turn begins."""
    if record.get("type") == "user":
        if (record.get("isMeta") or record.get("toolUseResult")
                or record.get("isCompactSummary") or record.get("isSidechain")):
            return False
        message = record.get("message") if isinstance(record.get("message"), dict) else {}
        content = message.get("content")
        if isinstance(content, list):
            if any(isinstance(part, dict) and part.get("type") == "tool_result" for part in content):
                return False
            content = "".join(
                part["text"] for part in content
                if isinstance(part, dict) and isinstance(part.get("text"), str)
            )
        return (isinstance(content, str) and bool(content.strip())
                and not content.lstrip().startswith(_TRANSCRIPT_PASSIVE_PREFIXES))
    if record.get("type") == "response_item":
        payload = record.get("payload") if isinstance(record.get("payload"), dict) else {}
        if payload.get("type") != "message" or payload.get("role") != "user":
            return False
        text = "".join(
            part["text"] for part in payload.get("content") or []
            if isinstance(part, dict) and isinstance(part.get("text"), str)
        )
        return bool(text.strip()) and not text.lstrip().startswith(_TRANSCRIPT_PASSIVE_PREFIXES)
    return False


def _transcript_calls(record: dict[str, object], root: str) -> list[tuple[str, dict[str, object], str]]:
    """(tool, arguments, working directory) for the calls one record makes:
    Claude Code main-thread tool uses, Codex shell and patch calls."""
    calls: list[tuple[str, dict[str, object], str]] = []
    if record.get("type") == "assistant" and not record.get("isSidechain"):
        message = record.get("message") if isinstance(record.get("message"), dict) else {}
        base = record.get("cwd") if isinstance(record.get("cwd"), str) else root
        for part in message.get("content") or []:
            if (isinstance(part, dict) and part.get("type") == "tool_use"
                    and isinstance(part.get("input"), dict)):
                calls.append((str(part.get("name") or ""), part["input"], base))
    elif record.get("type") == "response_item":
        payload = record.get("payload") if isinstance(record.get("payload"), dict) else {}
        kind = payload.get("type")
        if kind == "function_call":
            try:
                arguments = json.loads(payload.get("arguments") or "{}")
            except (TypeError, ValueError):
                arguments = {}
            arguments = arguments if isinstance(arguments, dict) else {}
            base = arguments.get("workdir") if isinstance(arguments.get("workdir"), str) else root
            if payload.get("name") == "apply_patch":
                calls.append(("apply_patch", {"patch": arguments.get("input")}, base))
            else:
                script = _shell_script(arguments.get("cmd") or arguments.get("command"), arguments.get("workdir"))
                if script:
                    calls.append(("Bash", {"command": script[1], "dialect": script[0]}, base))
        elif kind == "local_shell_call":
            action = payload.get("action") if isinstance(payload.get("action"), dict) else {}
            base = action.get("working_directory") if isinstance(action.get("working_directory"), str) else root
            script = _shell_script(action.get("command"), action.get("working_directory"))
            if script:
                calls.append(("Bash", {"command": script[1], "dialect": script[0]}, base))
        elif kind == "custom_tool_call" and payload.get("name") == "apply_patch":
            calls.append(("apply_patch", {"patch": payload.get("input")}, root))
    return calls


def _call_file_words(name: str, data: dict[str, object], base: str) -> list[tuple[str, bool, str]]:
    """(word, changed, directory) for what one tool call names."""
    if name in ("Edit", "MultiEdit", "Write", "NotebookEdit"):
        value = data.get("file_path") or data.get("notebook_path")
        return [(value, True, base)] if isinstance(value, str) else []
    if name in ("Read", "Grep"):
        value = data.get("file_path" if name == "Read" else "path")
        return [(value, False, base)] if isinstance(value, str) else []
    if name == "Bash" and isinstance(data.get("command"), str):
        return _shell_file_words(data["command"], base, dialect=str(data.get("dialect") or "sh"))
    if name == "apply_patch" and isinstance(data.get("patch"), str):
        return [((update or moved).strip(), True, base) for update, moved in _PATCH_FILE_LINE.findall(data["patch"])]
    return []


def _transcript_project_file(word: str, base: str, root: str) -> Optional[str]:
    """The project file a transcript word names, project-relative, or None.

    It must exist inside the project after symlinks resolve; ``git show
    REV:path``, ``path:12`` and ``path#L3`` forms name their path."""
    word = word.strip().strip("'\"`,;()[]{}<>")
    if not word or word.startswith("-") or len(word) > 400 or "\0" in word:
        return None
    candidates = [word]
    if ":" in word and not os.path.isabs(word):
        candidates.append(word.split(":", 1)[1])
    candidates.append(re.sub(r"(:\d+(:\d+)?|#L\d+(-L?\d+)?)$", "", word))
    for candidate in candidates:
        if not candidate:
            continue
        try:
            joined = candidate if os.path.isabs(candidate) else os.path.join(base, candidate)
            resolved = os.path.realpath(joined)
        except (OSError, ValueError):
            continue
        if resolved.startswith(root + os.sep) and os.path.isfile(resolved):
            return Path(os.path.relpath(resolved, root)).as_posix()
    return None


def _recent_file_kept(path: str) -> bool:
    pure = PurePosixPath(path)
    return not (
        pure.is_absolute() or ".." in pure.parts
        or path.startswith(RECENT_FILES_SKIPPED_PREFIXES)
        or path.lower().endswith(RECENT_FILES_SKIPPED_SUFFIXES)
        or TURN_PATH_DENYLIST.search(path) or _RECENT_RUNTIME_OWNED.match(path)
    )


def _without_ignored(repository: Path, paths: list[str]) -> list[str]:
    """Drop what the project's ignore rules exclude; tracked files stay."""
    if not paths:
        return paths
    try:
        result = subprocess.run(
            ["git", "-C", str(workspace_roots.project_root(repository)), "check-ignore", "--stdin", "-z"],
            input=b"\0".join(os.fsencode(path) for path in paths) + b"\0",
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=False, timeout=2,
        )
    except (OSError, subprocess.SubprocessError):
        return paths
    if result.returncode not in (0, 1):
        return paths
    ignored = {os.fsdecode(item) for item in result.stdout.split(b"\0") if item}
    return [path for path in paths if path not in ignored]


def transcript_turn_files(transcript: Optional[str], repository: Path) -> list[str]:
    """The project files the agent read or changed in the transcript's latest
    turn: changed first, then read, each in call order.

    Only call metadata is read - the file paths a call names and the words of
    a shell command that name an existing project file. Tool results, file
    contents and the commands themselves are never kept. The transcript must
    be an absolute regular file; only its last few megabytes are read."""
    if not transcript:
        return []
    path = Path(transcript)
    try:
        if not path.is_absolute() or path.is_symlink() or not path.is_file():
            return []
        size = path.stat().st_size
        start = max(0, size - RECENT_FILES_TRANSCRIPT_BYTES)
        with path.open("rb") as handle:
            handle.seek(start)
            chunk = handle.read(size - start)
    except (OSError, ValueError):
        return []
    lines = chunk.split(b"\n")
    if start:
        # The window began inside a record.
        lines = lines[1:]
    records: list[dict[str, object]] = []
    for raw in lines:
        if not raw.strip():
            continue
        try:
            record = json.loads(raw)
        except (UnicodeDecodeError, ValueError):
            continue
        if isinstance(record, dict):
            records.append(record)
    begin = 0
    for index in range(len(records) - 1, -1, -1):
        if _transcript_prompt(records[index]):
            begin = index + 1
            break
    root = os.path.realpath(str(workspace_roots.project_root(repository)))
    changed: list[str] = []
    read: list[str] = []
    deadline = time.monotonic() + RECENT_FILES_PARSE_SECONDS
    for record in records[begin:]:
        if time.monotonic() > deadline:
            break
        for name, data, base in _transcript_calls(record, root):
            for word, is_change, directory in _call_file_words(name, data, base):
                relative = _transcript_project_file(word, directory, root)
                if relative is not None and _recent_file_kept(relative):
                    (changed if is_change else read).append(relative)
    return _without_ignored(repository, list(dict.fromkeys(changed + read)))


def _recent_files_key(repository: Path) -> str:
    """The state key of a checkout's recent files.

    An installed project keeps them in its checkout's own database. An
    attached one keeps one state for every checkout of the project, and
    parallel sessions in two worktrees then named each other's files as
    their own: each checkout has a key of its own there."""
    if not workspace_roots.is_attached(repository):
        return RECENT_FILES_STATE_KEY
    checkout = os.path.realpath(str(workspace_roots.project_root(repository)))
    digest = hashlib.sha256(os.fsencode(checkout)).hexdigest()[:16]
    return f"{RECENT_FILES_STATE_KEY}:{digest}"


def load_recent_files(connection: sqlite3.Connection, key: str = RECENT_FILES_STATE_KEY) -> list[str]:
    """The checkout's recent files, newest first, as the Stop hook kept them."""
    try:
        row = connection.execute(
            "SELECT value FROM index_state WHERE key = ?", (key,)
        ).fetchone()
    except sqlite3.Error:
        return []
    if row is None:
        return []
    try:
        state = json.loads(row[0])
    except (TypeError, ValueError):
        return []
    paths = state.get("paths") if isinstance(state, dict) and state.get("version") == 1 else None
    if not isinstance(paths, list):
        return []
    return [path for path in paths if isinstance(path, str)][:RECENT_FILES_KEPT]


def record_recent_files(
    connection: sqlite3.Connection, paths: list[str], key: str = RECENT_FILES_STATE_KEY
) -> int:
    """Move this turn's files to the front of the checkout's recent files."""
    if not paths:
        return 0
    began = not connection.in_transaction
    if began:
        connection.execute("BEGIN IMMEDIATE")
    try:
        merged = list(dict.fromkeys([*paths, *load_recent_files(connection, key)]))[:RECENT_FILES_KEPT]
        connection.execute(
            "INSERT INTO index_state(key, value) VALUES (?, ?) "
            "ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            (key, json.dumps({"version": 1, "paths": merged}, ensure_ascii=False)),
        )
        if began:
            connection.commit()
    except BaseException:
        if began:
            connection.rollback()
        raise
    return len(paths)


def attach_recent_files(
    connection: sqlite3.Connection, repository: Path, capsule: dict[str, object]
) -> None:
    """Name the checkout's recent files that are still there in a capsule."""
    root = workspace_roots.project_root(repository)
    shown: list[str] = []
    for path in load_recent_files(connection, _recent_files_key(repository)):
        try:
            present = _recent_file_kept(path) and (root / path).is_file()
        except (OSError, ValueError):
            present = False
        if present:
            shown.append(path)
        if len(shown) >= RECENT_FILES_LIMIT:
            break
    if shown:
        capsule["recent_files"] = shown


def head_commit(repository: Path) -> Optional[str]:
    try:
        head = os.fsdecode(
            git_output(
                repository,
                ["rev-parse", "--verify", "--quiet", "HEAD"],
                "Git HEAD probe",
            )
        ).strip()
    except ContextError:
        return None
    return head or None


def head_advanced(
    connection: sqlite3.Connection, repository: Path, task_id: str
) -> bool:
    """Whether HEAD moved since this task's previous turn; remember it now.

    A turn that ends in a commit leaves a clean tree, and a clean tree was read
    as a turn with nothing in it, so committed work - the work that mattered
    most - never reached the buffer or the checkpoint. The first turn a task
    sees only records the commit, so visiting a branch still mints nothing.
    Best-effort: losing the record costs one uncounted turn.
    """
    head = head_commit(repository)
    if head is None:
        return False
    try:
        stored = json.loads(load_index_state(connection).get(TURN_HEADS_KEY) or "{}")
    except (ValueError, sqlite3.Error):
        stored = {}
    if not isinstance(stored, dict):
        stored = {}
    previous = stored.pop(task_id, None)
    stored[task_id] = head
    for stale in list(stored)[: max(0, len(stored) - TURN_HEADS_RETENTION)]:
        stored.pop(stale, None)
    try:
        with connection:
            store_index_state(
                connection,
                {TURN_HEADS_KEY: json.dumps(stored, separators=(",", ":"))},
            )
    except sqlite3.Error:
        pass
    return isinstance(previous, str) and previous != head


def checkpoint_safe(text: str) -> bool:
    """Whether text may enter a checkpoint without tripping a privacy gate.

    The same gates every working-task write applies. A commit subject that
    fails one is left out rather than refused: a checkpoint must never fail
    on what someone once typed into a commit.
    """
    return not (
        any(pattern.search(text) for pattern in SECRET_PATTERNS.values())
        or any(pattern.search(text) for pattern in CAPSULE_PRIVATE_PATTERNS)
        or CAPSULE_RAW_TEXT_PATTERN.search(text)
    )


def branch_commit_subjects(
    connection: sqlite3.Connection, repository: Path
) -> tuple[list[str], int]:
    """This branch's newest commit subjects, and how many commits it has.

    Measured from the merge base with the default branch, so the list is the
    branch's own story rather than the project's history. On the default
    branch itself, or without one, there is no branch story and the list is
    empty. Merges are left out, and so is any subject `checkpoint_safe`
    rejects; both still count toward the total.
    """
    target = cached_default_branch(connection, repository)
    head = head_commit(repository)
    if target is None or head is None:
        return [], 0
    try:
        base = os.fsdecode(
            git_output(repository, ["merge-base", "HEAD", target], "Git merge-base probe")
        ).strip()
        if not base or base == head:
            return [], 0
        listing = git_output(
            repository,
            [
                "log", "--no-merges", "-z", "--format=%s",
                f"--max-count={CHECKPOINT_LOG_LIMIT}", f"{base}..HEAD", "--", ".",
            ],
            "Git log probe",
        )
    except ContextError:
        return [], 0
    subjects = [
        " ".join(os.fsdecode(item).split()) for item in listing.split(b"\0")
    ]
    subjects = [subject for subject in subjects if subject]
    shown = [
        subject
        if len(subject) <= CHECKPOINT_SUBJECT_CHARACTERS
        else subject[: CHECKPOINT_SUBJECT_CHARACTERS - 1].rstrip() + "…"
        for subject in subjects
        if checkpoint_safe(subject)
    ]
    return shown[:CHECKPOINT_COMMIT_LIMIT], len(subjects)


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


def recent_touched_paths(
    connection: sqlite3.Connection, task_id: str, limit: int = TOUCH_SEED_LIMIT
) -> list[str]:
    """Files this branch changed on its last few turns, newest first - also
    before its task exists, on a branch's first turns."""
    try:
        rows = connection.execute(
            "SELECT files FROM turn_deltas WHERE task_id = ? ORDER BY id DESC LIMIT 5", (task_id,)
        ).fetchall()
    except sqlite3.Error:
        return []
    paths: list[str] = []
    for row in rows:
        try:
            files = json.loads(row[0])
        except (TypeError, ValueError):
            continue
        if not isinstance(files, list):
            continue
        for path in reversed(files):
            if isinstance(path, str) and path not in paths:
                paths.append(path)
            if len(paths) >= limit:
                return paths
    return paths


def _checkpoint_time(value: object) -> str:
    """When a checkpoint's first turn began, to the minute. The capsule shows
    it on every prompt; seconds, microseconds and the offset told a reader
    nothing more."""
    try:
        moment = datetime.fromisoformat(str(value))
    except ValueError:
        return str(value)
    if moment.tzinfo is not None:
        moment = moment.astimezone(timezone.utc)
    return moment.strftime("%Y-%m-%d %H:%M UTC")


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
        f"Auto-checkpoint: {len(pending)} turn(s) since {_checkpoint_time(pending[0]['created_at'])}, "
        f"{len(files)} file(s) touched."
    )
    if omitted:
        checkpoint += f" {omitted} path(s) beyond the per-flush limit are not listed."
    # Paths say where work happened; commit subjects say what it was. They
    # follow the counts so the checkpoint still opens the way readers expect.
    subjects, commits = branch_commit_subjects(connection, repository)
    if subjects:
        checkpoint += " Branch commits, newest first: " + "; ".join(subjects)
        if commits > len(subjects):
            checkpoint += f" (+{commits - len(subjects)} more)"
        checkpoint += "."
    elif commits:
        checkpoint += f" {commits} commit(s) on this branch."
    files = files[:file_limit]
    closed = closed_task_record(connection, repository, task_id) if mode != "lightweight" else None
    if closed is not None:
        # The branch's task was completed and work went on: every flush tried
        # to reattach the closed task, failed, and kept the buffer - so the
        # turn boundary, and the promotion and compaction that ride on it,
        # never ran again. The turns after completion belong to no task;
        # they are dropped and said so, and maintenance proceeds.
        with connection:
            connection.executemany(
                "DELETE FROM turn_deltas WHERE id = ?",
                [(row["id"],) for row in pending],
            )
        return {
            "revision": None, "files_omitted": 0, "provisioned": False,
            "rebound": None, "closed_task": closed, "discarded_turns": len(pending),
        }
    with mutation_lock(repository) if mode == 'governed' else nullcontext():
        created = []
        topic_plan = (_cursor_topic_begin(connection, repository, task_id, mode)
                      if connection.execute('SELECT 1 FROM index_state WHERE key=?', (CURSOR_TOPIC_KEY,)).fetchone()
                      else (None, None, None))
        ensured = ensure_working_task(connection, repository, task_id, mode, owner, _created_sink=created)
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
        _cursor_topic_handoff(connection, repository, task_id, mode, topic_plan, created)
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
    # A turn that ended in a commit leaves a clean tree and is still work.
    committed = head_advanced(connection, repository, task_id)
    delta_id = (
        append_turn_delta(connection, task_id, files)
        if files or committed
        else None
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
    # Full maintenance remains a flush boundary. A scheduled promotion retry
    # is the only exception: ordinary turns use Git probes and local state,
    # while a due retry shares the Stop hook's timeout and mutation lock.
    completion_candidates: list[dict[str, object]] = []
    promotion = {
        "enabled": False, "promoted": [], "failed": [],
        "blocked": [], "skipped": 0,
    }
    compaction = {"enabled": False, "moved": 0, "pending": 0, "error": None}
    consolidation = {
        "enabled": False, "created": 0, "updated": 0, "superseded": 0,
        "blocked": 0, "tickets": 0, "error": None,
    }
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
        # A ticket's completed events become one finding before promotion,
        # which may carry it into the Memory Bank; the events leave the diary
        # on a later boundary, once a chunk holds what they said.
        consolidation = auto_consolidate(repository, owner=owner)
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
    elif mode != "lightweight":
        # A failed/interrupted promotion has its own local retry schedule.
        # Recover it without inventing a Git delta or changing task lifecycle.
        try:
            promotion = retry_pending_auto_promotion(repository, owner=owner)
        except (BrainError, OSError) as error:
            raise ContextError(f"Automatic promotion retry failed: {error}") from error
    # The project's recent files are a convenience for the next session's
    # capsule, so they come after the turn's own record: read first, a long
    # transcript on a slow disk could run the Stop hook out of its budget and
    # lose the checkpoint itself. A transcript that cannot be read never fails
    # the turn.
    recent = 0
    try:
        recent = record_recent_files(
            connection,
            transcript_turn_files(getattr(arguments, "transcript", None), repository),
            key=_recent_files_key(repository),
        )
    except (OSError, ValueError, sqlite3.Error):
        recent = 0
    result = {
        "task_id": task_id,
        "delta_id": delta_id,
        "files": len(files),
        "excluded": path_excluded,
        "recent_files": recent,
        "consolidation": consolidation,
        "pending": 0 if flushed else buffered,
        "flushed": flushed is not None and not flushed.get("closed_task"),
        "revision": flushed.get("revision") if flushed else None,
        "files_omitted": flushed.get("files_omitted") if flushed else 0,
        "provisioned": bool(flushed and flushed.get("provisioned")),
        # The UUID of the pre-existing record the flush restored the local
        # binding to — the second-machine signal. None on an ordinary flush.
        "rebound": flushed.get("rebound") if flushed else None,
        # The completed task the buffered turns were meant for, when the
        # branch went on after completion; those turns were dropped.
        "closed_task": flushed.get("closed_task") if flushed else None,
        "discarded_turns": flushed.get("discarded_turns", 0) if flushed else 0,
        "completion_candidates": completion_candidates,
        "promoted": promotion["promoted"],
        "promotion_failed": promotion["failed"],
        "promotion_blocked": promotion["blocked"],
        "promotion_skipped": promotion["skipped"],
        "archived": compaction["moved"],
        "archivable_pending": compaction["pending"],
    }
    if promotion.get("retry_unavailable"):
        result["promotion_retry_unavailable"] = True
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
            "closed_task": result["closed_task"],
            "discarded_turns": result["discarded_turns"],
            "completion_candidates": [
                str(item["task_id"]) for item in completion_candidates
            ],
            "promoted": promotion["promoted"],
            "promotion_failed": promotion["failed"],
            "promotion_blocked": promotion["blocked"],
            "promotion_skipped": promotion["skipped"],
            **({"promotion_retry_unavailable": True} if promotion.get("retry_unavailable") else {}),
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
        if result["closed_task"]:
            print(
                f"task {task_id} is closed: {result['discarded_turns']} turn(s) "
                "after its completion were not recorded; to keep further work, "
                + _closed_task_remedy(task_id)
            )
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
            [
                "git", "-C", str(workspace_roots.project_root(repository)),
                "rev-parse", "HEAD",
            ],
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
    # Named outright: run through context_cli.py, Python 3.14 would call the
    # program `python3 -m context` in usage and error text.
    parser = argparse.ArgumentParser(prog="context.py", description=__doc__)
    parser.add_argument("--root", type=Path, default=default_root())
    parser.add_argument("--db", type=Path)
    parser.add_argument("--mode", choices=("governed", "lightweight"))
    parser.add_argument(
        "--owner",
        default=None,
        help="Project Brain actor/owner (default: PROJECT_BRAIN_OWNER or local)",
    )
    commands = parser.add_subparsers(dest="command", required=True)
    cursor_maintenance = commands.add_parser("cursor-rule", help="maintain a proven generated local Cursor rule")
    cursor_maintenance.add_argument("--action", choices=("write", "keep", "retire", "prompt-held", "migrate"), required=True)
    cursor_maintenance.add_argument("--approved-current-sha256",
        help="explicit operator approval of one unreceipted current rule image; used only by migrate")
    cursor_maintenance.add_argument("--task-id", default="")
    cursor_maintenance.add_argument("--phase", choices=("prompt", "turn", "session"), default="prompt")
    cursor_maintenance.add_argument("--report-json", action="store_true")
    cursor_maintenance.add_argument("--json", action="store_true")
    cursor_maintenance.add_argument("--diagnostic", action="store_true", help="fixed maintenance status on stderr")
    cursor_retire = commands.add_parser("cursor-rule-retire", help="retire only a proven generated installed Cursor rule")
    cursor_retire.add_argument("--json", action="store_true")
    cursor_retire.add_argument("--diagnostic", action="store_true", help="fixed maintenance status on stderr")

    context_save = commands.add_parser(
        "context-save",
        help="write a curated, portable continuation handoff without task-state mutation",
    )
    context_save.add_argument("--input", required=True, help="path to one curated JSON object")
    context_save.add_argument("--output", type=Path, required=True)
    context_save.add_argument("--detail", choices=("summary", "topic", "full"), required=True)
    context_save.add_argument("--topic")
    context_save.add_argument("--task-id")
    context_save.add_argument("--source-client", choices=("codex", "claude", "cursor", "other"), required=True)
    context_save.add_argument("--transcript", type=Path)
    context_save.add_argument("--json", action="store_true")

    context_load = commands.add_parser(
        "context-load",
        help="read a curated continuation handoff and report repository drift",
    )
    context_load.add_argument("--input", type=Path, required=True)
    context_load.add_argument("--include-transcript", action="store_true")
    context_load.add_argument("--json", action="store_true")

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
    for transport_parser in (context, retrieve_command):
        transport_parser.add_argument('--provenance-response', action='store_true', help=argparse.SUPPRESS)

    hook_context = commands.add_parser(
        "hook-context",
        help="render a governed or pre-provision warming capsule for host hooks",
    )
    hook_context.add_argument("--task-id", required=True)
    hook_context.add_argument("--json", action="store_true")
    hook_context.add_argument('--delivery-profile', choices=('ordinary','cursor-rule'), default='ordinary', help=argparse.SUPPRESS)


    for retrieval_parser in (context, retrieve_command):
        retrieval_parser.add_argument("--source-hints-json", help=argparse.SUPPRESS)
        retrieval_parser.add_argument("--trace-path", action="append", default=[], dest="trace_paths",
                                      help="observe a document in local diagnostics only; repeatable, max 16")
        retrieval_parser.add_argument(
            "--path",
            action="append",
            default=[],
            dest="paths",
            help=(
                "repeatable; also deliver documents that cite this source "
                "path. Two chunks whose only connection is a shared source "
                "are unreachable from each other lexically — measured at 0 of "
                "2 — and this is the route that reaches them."
            ),
        )
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
    refresh.add_argument(
        "--session-id",
        help=(
            "the host conversation this capsule is for; what it was handed in "
            "its last few turns is not handed again"
        ),
    )
    refresh.add_argument(
        "--transcript",
        help=(
            "with --session-id: the host's transcript of that conversation; a "
            "compaction recorded there since the last turn means nothing "
            "counts as handed any more"
        ),
    )
    refresh.add_argument(
        "--sanitize",
        action="store_true",
        help=(
            "cut secrets, personal data and transcript-style prefixes out of "
            "--query instead of refusing it (the automatic paths pass this)"
        ),
    )
    refresh.add_argument("--source-hints-json", help=argparse.SUPPRESS)
    refresh.add_argument("--limit", type=int, default=3)
    refresh.add_argument("--path", action="append", default=[], dest="paths",
        help="also deliver documents citing this canonical source path")
    refresh.add_argument("--trace-path", action="append", default=[], dest="trace_paths",
                         help="observe a document in local diagnostics only; repeatable, max 16")
    refresh.add_argument(
        "--ephemeral",
        action="store_true",
        help="write the capsule's retrieval manifest to ignored local state",
    )
    refresh.add_argument(
        "--capsule-profile",
        choices=("full", "navigation"),
        default=None,
        help=(
            "what this capsule carries, over CONTEXT_CAPSULE_PROFILE and "
            "runtime.json; an explicit request (MCP memory_retrieve) passes full"
        ),
    )
    for inferred_parser in (refresh, hook_context):
        inferred_parser.add_argument("--expected-branch", default=None, help=argparse.SUPPRESS)
    # Every entry point that can retrieve takes the gate mode, because the
    # decision is about the turn and each of these is a turn. The resolution
    # ladder (flag, environment, runtime.json, then shadow) lives in
    # `configured_retrieval_gate`, so the flag is only the first rung.
    for gated_parser in (context, retrieve_command, refresh, hook_context):
        gated_parser.add_argument(
            "--host",
            choices=RETRIEVAL_HOSTS,
            default="cli",
            help="client invoking retrieval; recorded in the manifest",
        )
        gated_parser.add_argument(
            "--gate",
            choices=RETRIEVAL_GATE_MODES,
            default=None,
            help=(
                "whether this turn decides to retrieve: off never decides, "
                "shadow decides and records without acting, enforce acts. "
                "Overrides CONTEXT_RETRIEVAL_GATE and runtime.json."
            ),
        )
    refresh.add_argument(
        "--validate",
        action="store_true",
        help="also report Project Brain validation (reads every record)",
    )
    refresh.add_argument("--json", action="store_true")
    refresh.add_argument('--provenance-response', action='store_true', help=argparse.SUPPRESS)
    refresh.add_argument('--read-only', action='store_true',
        help='read compatible existing working state without refreshing or writing local state')
    refresh.add_argument('--read-only-reason', choices=('budget',), help=argparse.SUPPRESS)
    refresh.add_argument('--delivery-profile', choices=('ordinary','cursor-rule'), default='ordinary', help=argparse.SUPPRESS)

    turn = commands.add_parser(
        "turn", help="buffer a per-turn working-memory delta and flush on a boundary"
    )
    turn.add_argument("--task-id", required=True)
    turn.add_argument(
        "--flush", action="store_true", help="flush regardless of the threshold"
    )
    turn.add_argument(
        "--transcript",
        help="the host's transcript, for the files this turn read or changed (paths only)",
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
    update.add_argument(
        "--replace-next-steps",
        action="store_true",
        help=(
            "make this call's --next-step values the whole list instead of "
            "appending them; with none given, clear it"
        ),
    )
    update.add_argument("--file", action="append", default=[])
    update.add_argument("--source", action="append", default=[])
    update.add_argument(
        "--phase",
        choices=TASK_PHASE_INPUTS,
        help=(
            "delivery phase; aliases implementing/execution/review/quality are "
            "stored as implementation/implementation/verification/verification"
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

    record = commands.add_parser(
        "record-result",
        help=(
            "record a run's progress, next steps and up to three source-backed "
            "findings or decisions; replaying the same --result-id with the "
            "same content passively replays a completed save; uncertain partial saves are refused"
        ),
    )
    record.add_argument("--task-id", required=True)
    record.add_argument(
        "--result-id",
        required=True,
        help="stable for this result: the same ID and content is a replay, never a second copy",
    )
    record.add_argument("--revision", type=revision_argument, required=True)
    record.add_argument(
        "--input",
        required=True,
        help=(
            "JSON object with progress, next_steps, learnings (type, title, "
            "consequence, sources) and verified; '-' reads standard input"
        ),
    )
    record.add_argument(
        "--attestation",
        choices=("agent", "person"),
        default="agent",
        help="who checked the learnings against their sources (default: agent)",
    )
    record.add_argument("--reason")
    record.add_argument("--prepared-scope")
    record.add_argument("--json", action="store_true")

    for action in ("native-prompt", "native-stop"):
        native = commands.add_parser(action, help="bounded automatic native result capture")
        native.add_argument("--host", choices=("claude", "codex", "cursor"), required=True)
        native.add_argument("--task-id", required=True)
        native.add_argument("--json", action="store_true")
    native = commands.add_parser("native-result", help="submit one pending native structured result")
    native.add_argument("--request", required=True)
    native.add_argument("--token", required=True)
    content = native.add_mutually_exclusive_group(required=True)
    content.add_argument("--input")
    content.add_argument("--skip", action="store_true")
    native.add_argument("--json", action="store_true")

    for action in ("prepare-result", "replay-result"):
        prepared_result = commands.add_parser(action, help="validate or replay a prepared memory result")
        prepared_result.add_argument("--task-id", required=True)
        prepared_result.add_argument("--result-id", required=True)
        prepared_result.add_argument("--revision", type=revision_argument, required=True)
        prepared_result.add_argument("--input", required=True)
        prepared_result.add_argument("--attestation", choices=("agent", "person"), default="agent")
        prepared_result.add_argument("--reason", required=True)
        if action == "replay-result":
            prepared_result.add_argument("--prepared-scope", required=True)
        prepared_result.add_argument("--json", action="store_true")

    update_brain = commands.add_parser(
        "brain-update", help="CAS-update a governed dynamic Brain record"
    )
    update_brain.add_argument("--record-id", required=True)
    update_brain.add_argument("--revision", type=revision_argument, required=True)
    update_brain.add_argument("--progress")
    update_brain.add_argument("--next-step", action="append", default=[])
    update_brain.add_argument(
        "--replace-next-steps",
        action="store_true",
        help=(
            "make this call's --next-step values the whole list instead of "
            "appending them; with none given, clear it"
        ),
    )
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

    retrieval_report_command = commands.add_parser(
        "retrieval-report",
        help="aggregate what the retrieval manifests recorded, across turns",
    )
    retrieval_report_command.add_argument(
        "--since",
        type=int,
        default=None,
        help="use only the N most recent manifests (records, not days)",
    )
    retrieval_report_command.add_argument(
        "--scope", choices=("local", "governed", "all"), default="all"
    )
    retrieval_report_command.add_argument("--json", action="store_true")

    health = commands.add_parser(
        "health",
        help="aggregate per-turn refresh health: phase percentiles, timeouts, loss",
    )
    health.add_argument(
        "--window", type=int, default=None, help="use only the last N records"
    )
    health.add_argument("--json", action="store_true")

    links = commands.add_parser(
        "links",
        help="show every eligible document that cites, or active task that touched, a given path",
    )
    links.add_argument(
        "--path", required=True, help="the path to look up citations and touches of"
    )
    links.add_argument(
        "--prefix",
        action="store_true",
        help="also match sources under --path, treating it as a directory",
    )
    links.add_argument("--json", action="store_true")

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

    consolidate_command = commands.add_parser(
        "consolidate",
        help=(
            "collapse one ticket's completed events into a finding; dry-run "
            "unless --apply is passed"
        ),
    )
    consolidate_command.add_argument("--apply", action="store_true")
    consolidate_command.add_argument(
        "--ticket", default=None, help="only this ticket, for example CMS-741"
    )
    consolidate_command.add_argument("--json", action="store_true")

    bank_audit = commands.add_parser(
        "bank-audit",
        help="report chunks whose cited sources changed and chunks overdue for review",
    )
    bank_audit.add_argument(
        "--provenance",
        action="store_true",
        help="also report auto-promoted, external-source and instruction-like chunks",
    )
    bank_audit.add_argument("--json", action="store_true")

    bank_reverify = commands.add_parser(
        "bank-reverify",
        help="re-attest an active chunk against its sources as they are now",
    )
    bank_reverify.add_argument("--id", required=True, dest="memory_id")
    bank_reverify.add_argument(
        "--review-after",
        default=None,
        help="next review date (YYYY-MM-DD); defaults to one year from today",
    )
    bank_reverify.add_argument("--json", action="store_true")

    bank_retire = commands.add_parser(
        "bank-retire",
        help="close a durable chunk's validity period, archiving or superseding it",
    )
    bank_retire.add_argument("--id", required=True, dest="memory_id")
    bank_retire.add_argument(
        "--valid-to",
        required=True,
        help="the last date the chunk's knowledge held (YYYY-MM-DD)",
    )
    bank_retire.add_argument(
        "--superseded-by",
        default=None,
        help=(
            "the chunk that replaced this one; without it the chunk is "
            "archived, which is the honest status for knowledge that ceased "
            "without a successor"
        ),
    )
    bank_retire.add_argument("--reason", default=None)
    bank_retire.add_argument("--json", action="store_true")

    bank_quarantine = commands.add_parser(
        "bank-quarantine",
        help=(
            "stop serving a chunk now - for suspected poisoning or an unsafe "
            "claim - by closing its validity at today"
        ),
    )
    bank_quarantine.add_argument("--id", required=True, dest="memory_id")
    bank_quarantine.add_argument("--reason", default=None)
    bank_quarantine.add_argument("--json", action="store_true")

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

    promote_auto = commands.add_parser(
        "promote-auto",
        help=(
            "run automatic promotion now, under the same rules as a turn "
            "boundary; does nothing unless automatic_promotion is enabled"
        ),
    )
    promote_auto.add_argument("--json", action="store_true")

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


def utf8_output() -> None:
    """Use UTF-8 and untranslated LF on standard console and pipe streams.

    Every capsule carries '…' and '—'. On Windows a pipe takes the ANSI code
    page, the first such character raised UnicodeEncodeError, and the hook
    delivered nothing on every turn.
    """
    for stream in (sys.stdout, sys.stderr):
        options = {"encoding": "utf-8", "errors": "replace"}
        # Standard wrappers support newline on every supported Python. Custom
        # streams (including subclasses) keep their existing two-keyword API;
        # do not retry a call that may already have flushed or changed state.
        if type(stream) is io.TextIOWrapper:
            options["newline"] = "\n"
        try:
            stream.reconfigure(**options)
        except (AttributeError, TypeError, ValueError, OSError):
            continue


# SQLite names a damaged file in two ways; a lock, a full disk or a refused
# permission is not damage and stays an error.
DAMAGED_DATABASE = ("database disk image is malformed", "file is not a database")
# Commands the hooks run on their own, whose database in governed mode is
# derived: the index, the bindings the Brain restores, the turn buffer.
DAMAGE_RECOVERING_COMMANDS = ("refresh", "turn", "hook-context", "retrieve", "index")
# What this process moved aside, for the capsule to say.
_QUARANTINED: list[tuple[str, str]] = []
_COMMAND_STATE: dict[str, object] = {}


class _DamagedDatabase(Exception):
    """A governed command's local database is damaged; main() moves it aside
    and runs the command again once."""

    def __init__(self, database: Path, error: BaseException):
        super().__init__(str(error))
        self.database = database


def _damaged_database(error: BaseException) -> Optional[Path]:
    """The database to move aside, when `error` is damage to the governed
    local database of a command that recovers from it."""
    database = _COMMAND_STATE.get("database")
    if (_QUARANTINED or not isinstance(error, sqlite3.DatabaseError)
            or not any(text in str(error) for text in DAMAGED_DATABASE)
            or _COMMAND_STATE.get("mode") != "governed"
            or _COMMAND_STATE.get("command") not in DAMAGE_RECOVERING_COMMANDS
            or _COMMAND_STATE.get("read_only") or not isinstance(database, Path)):
        return None
    return database


def _quarantine_database(database: Path) -> str:
    """Move a damaged database and its journals aside, never delete them."""
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    target = database.with_name(f"{database.name}.quarantine-{stamp}")
    for suffix in ("", "-wal", "-shm", "-journal"):
        source = Path(str(database) + suffix)
        if os.path.lexists(source):
            os.replace(source, Path(str(target) + suffix))
    return target.name


def main() -> int:
    # The ignore checks of one command share the Git frames they confirm.
    with workspace_roots.source_frame_scope():
        try:
            return _command()
        except _DamagedDatabase as damaged:
            # The governed local database is derived: the index is rebuilt,
            # the binding restored from the Brain at the next checkpoint. Only
            # turns buffered since the last checkpoint were its own.
            try:
                _QUARANTINED.append((_quarantine_database(damaged.database), str(damaged)))
            except OSError:
                print(f"context: {damaged}", file=sys.stderr)
                return 1
    with workspace_roots.source_frame_scope():
        return _command()


def _command() -> int:
    utf8_output()
    arguments = build_parser().parse_args()
    _branch_task_identity(arguments)
    if arguments.command in ("cursor-rule", "cursor-rule-retire"):
        from context_retrieval import cursor_rule, CURSOR_RULE_LIMIT
        action = "retire" if arguments.command == "cursor-rule-retire" else arguments.action
        capsule = ""
        if action in ("write", "migrate"):
            try:
                admission = 1024 * 1024 if arguments.report_json else CURSOR_RULE_LIMIT
                raw = sys.stdin.buffer.read(admission + 1)
                if len(raw) <= admission:
                    capsule = raw.decode("utf-8", "strict")
                    if arguments.report_json:
                        value = json.loads(capsule)
                        packet = value.get("capsule") if isinstance(value, dict) else None
                        gate = packet.get("gate") if isinstance(packet, dict) else None
                        if (isinstance(gate, dict) and gate.get("mode") == "enforce"
                                and gate.get("decision") == "skip"
                                and gate.get("reason") not in ("no-relevant-match", "empty-after-filter")):
                            # A withheld render is not replacement context. Keep
                            # repeated/legacy skips, but a real empty selection
                            # publishes current working state without old facts.
                            action = "keep"
                            capsule = ""
                        else:
                            capsule = value.get("capsule_text", "") if isinstance(value, dict) else ""
            except (OSError, ValueError, UnicodeError, RecursionError):
                capsule = ""
        result = cursor_rule(arguments.root, action,
            task=getattr(arguments, "task_id", ""), phase=getattr(arguments, "phase", "prompt"), capsule=capsule,
            **({"approved_sha256": arguments.approved_current_sha256}
               if getattr(arguments, "approved_current_sha256", None) is not None else {}))
        # A held-rule question answered "preserved" is no failure to report.
        if (arguments.diagnostic and result.get('state') in ('preserved', 'incomplete')
                and not (action == 'prompt-held' and result.get('state') == 'preserved')):
            reason = ('maintenance_incomplete' if result['state'] == 'incomplete' else result.get('reason'))
            if reason not in ('maintenance_incomplete', 'configuration_unavailable', 'unsupported_custody',
                              'migration_unapproved', 'custody_changed', 'ownership_unverified', 'receipt_unavailable',
                              'linked_directory'):
                reason = 'custom_or_unavailable'
            print('Cursor memory rule was not updated: ' + reason + '.', file=sys.stderr)
        print(json.dumps(result))
        return 0
    repository = arguments.root.resolve()

    try:
        delivery_profile = getattr(arguments, 'delivery_profile', 'ordinary')
        if getattr(arguments, 'read_only_reason', None) and not getattr(arguments, 'read_only', False):
            raise ContextError('Read-only reason requires read-only refresh')
        if delivery_profile == 'cursor-rule' and (getattr(arguments,'host',None) != 'cursor'
                or getattr(arguments,'session_id',None) is not None or getattr(arguments,'transcript',None) is not None):
            raise ContextError('Unsupported persistent delivery profile')
        problem = workspace_roots.configuration_error()
        if problem is not None:
            raise ContextError(problem)
        expected_branch = getattr(arguments, "expected_branch", None)
        delivery_admission = None
        if expected_branch is not None:
            if getattr(arguments, "task_id", None) not in (branch_task_id(expected_branch),
                                                         getattr(arguments, "renamed_from", None)):
                raise SourceGitError("Inferred task branch changed or unavailable")
            # This includes the source frame admission; do not repeat it as a
            # separate preflight before the shared branch-operation deadline.
            delivery_admission = _branch_admission(repository, expected_branch)
        elif arguments.command in ("refresh", "context", "retrieve", "hook-context", "index"):
            # Even an empty candidate list must not let a foreign/unknown source
            # context create state or return unverified working memory.
            environment = _source_git_environment()
            _source_git_frame(workspace_roots.project_root(repository), environment,
                              time.monotonic() + 1.0)
        if workspace_roots.is_attached(repository) and os.name != "nt":
            # Attached state is the project's memory kept outside it, so it is
            # its owner's alone: whatever this run creates there - records, the
            # index database and its journals, logs, receipts - is owner-only,
            # whatever umask the hook or the tool passed on. Installed state
            # lives in the project and keeps the project's permissions.
            os.umask(0o077)
        # Attached, the state directory is created on first use; installed,
        # this does nothing and a missing root stays an error.
        if not (arguments.command == 'refresh' and arguments.read_only):
            ensure_attached_state(repository)
        if not repository.is_dir():
            if arguments.command == 'refresh' and arguments.read_only:
                validate_direct_query_request(arguments)
                if arguments.limit < 1 or (arguments.query is not None and arguments.task_id is None):
                    raise ContextError('Invalid read-only refresh request')
                mode = configured_mode(repository, arguments.mode)
                return degraded_refresh(None, repository, arguments, mode, delivery_profile, delivery_admission=delivery_admission)
            raise ContextError(
                f"Repository root must be an existing directory: {repository}"
            )
        # Portable handoffs deliberately run before opening SQLite.  Loading a
        # handoff is read-only, and saving one must not create a competing
        # local task/cache or mutate governed Project Brain state. A handoff
        # belongs to the project: Git, cited files and relative paths are the
        # project's, which is the state root itself unless attached.
        if arguments.command in ("context-save", "context-load"):
            handoff_project = workspace_roots.project_root(repository)

            def in_project(path: Path) -> Path:
                return path if path.is_absolute() else handoff_project / path

        if arguments.command == "context-save":
            result = save_handoff(
                handoff_project,
                input_json=str(in_project(Path(arguments.input))),
                output=in_project(arguments.output),
                detail=arguments.detail,
                topic=arguments.topic,
                task_id=arguments.task_id,
                source_client=arguments.source_client,
                transcript_path=in_project(arguments.transcript) if arguments.transcript else None,
                state_root=repository,
            )
            if arguments.json:
                print(json.dumps(result, ensure_ascii=False))
            else:
                print(f"Context handoff saved: {result['output']}.")
            return 0
        if arguments.command == "context-load":
            result = load_handoff(
                handoff_project, in_project(arguments.input), include_transcript=arguments.include_transcript
            )
            if arguments.json:
                print(json.dumps(result, ensure_ascii=False))
            else:
                print(f"Context handoff loaded: {result['input']}.")
                if result["drift"]:
                    print(f"  {len(result['drift'])} source drift item(s); review before continuing.")
                    print(json.dumps(result["drift"], ensure_ascii=False, indent=2))
                print(render_body(result["context"]))
                if "transcript" in result:
                    print("## Visible transcript\n")
                    sys.stdout.write(result["transcript"])
            return 0
        # Direct query commands have a strict CLI contract: reject the original
        # value before connect() can create a database or an index/manifest can
        # be refreshed. Host hooks remain fail-safe by swallowing this nonzero
        # result outside the CLI.
        validate_direct_query_request(arguments)
        if getattr(arguments, 'provenance_response', False) and not arguments.json:
            raise ContextError('Source provenance transport requires JSON output')
        database = (arguments.db or default_database(repository)).resolve()
        mode = configured_mode(repository, arguments.mode)
        _COMMAND_STATE.update(database=database, mode=mode, command=arguments.command,
                              read_only=bool(getattr(arguments, "read_only", False)))
        gate_mode = configured_retrieval_gate(
            repository, getattr(arguments, "gate", None)
        )
        owner = arguments.owner or os.environ.get("PROJECT_BRAIN_OWNER", "local")
        if arguments.command == 'refresh':
            if arguments.limit < 1:
                raise ContextError('--limit must be a positive integer')
            if arguments.query is not None and arguments.task_id is None:
                raise ContextError('--query requires --task-id')
            startup_wait, busy_wait = _refresh_waits(arguments)
            if arguments.read_only:
                connection = None
            else:
                try:
                    connection = connect(database, timeout=startup_wait, require_write=True,
                                         busy_seconds=busy_wait)
                except (OSError, sqlite3.Error) as error:
                    if _damaged_database(error) is not None:
                        raise _DamagedDatabase(database, error) from error
                    connection = None
            if connection is None:
                try:
                    connection = connect_read_only(database, timeout=startup_wait)
                except (ContextError, OSError, sqlite3.Error):
                    connection = None
                try:
                    return degraded_refresh(connection, repository, arguments, mode, delivery_profile, delivery_admission=delivery_admission)
                finally:
                    if connection is not None:
                        connection.close()
        else:
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
                    # A document that was dropped is the one thing a count of
                    # what stayed cannot report. Summarize by reason here and
                    # point at --json for the paths.
                    dropped = result.get("excluded") or []
                    if isinstance(dropped, list) and dropped:
                        tally: dict[str, int] = {}
                        for item in dropped:
                            reason = str(item.get("reason", "unknown"))
                            tally[reason] = tally.get(reason, 0) + 1
                        summary = ", ".join(
                            f"{reason}: {count}"
                            for reason, count in sorted(tally.items())
                        )
                        print(
                            f"Excluded: {len(dropped)} document(s) ({summary}); "
                            "--json lists the paths."
                        )
                    redacted = result.get("redacted") or []
                    if isinstance(redacted, list) and redacted:
                        print(
                            f"Redacted: {sum(int(item.get('values') or 0) for item in redacted)} "
                            f"value(s) in {len(redacted)} document(s); --json lists the paths."
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
                        replace_next_steps=arguments.replace_next_steps,
                    )
                else:
                    binding = governed_binding(connection, arguments.task_id)
                    progress = arguments.progress
                    if progress is None and not (
                        arguments.next_step
                        or arguments.file
                        or arguments.source
                        or arguments.phase
                        or arguments.replace_next_steps
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
                        replace_next_steps=arguments.replace_next_steps,
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
                if arguments.command == "msg-dispatch":
                    # A subagent that finishes on a branch the first
                    # checkpoint has not provisioned yet would otherwise
                    # lose its record silently: subagent-dispatch.sh reports
                    # a failed write, but reads a missing task as no channel
                    # to report to. A completion is real work, which is what
                    # provisioning waits for. Any refusal (a terminal task)
                    # is left to the lookup below to report as it always did.
                    try:
                        ensure_working_task(
                            connection, repository, arguments.task_id, mode, owner
                        )
                    except (ContextError, BrainError):
                        pass
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

            if arguments.command in ("native-prompt", "native-stop", "native-result"):
                import native_results
                if arguments.command == "native-result":
                    if arguments.skip:
                        data = None
                    elif arguments.input == "-":
                        data = native_results.read_input(sys.stdin.buffer)
                    else:
                        with Path(arguments.input).open("rb") as stream:
                            data = native_results.read_input(stream)
                    result = native_results.save(repository, arguments.request, arguments.token, data,
                        owner=owner, mode=mode, skip=arguments.skip,
                        provision=lambda task_id: ensure_working_task(connection, repository, task_id, mode, owner))
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    payload = native_results.read_input(sys.stdin.buffer)
                    operation = native_results.begin if arguments.command == "native-prompt" else native_results.stop
                    result = operation(repository, arguments.task_id, arguments.host, payload, owner=owner, mode=mode)
                    if arguments.command == "native-stop":
                        if result: print(json.dumps(result, ensure_ascii=False))
                    else:
                        if result.get("warning"):
                            report = load_last_turn_report(repository) or {"schema_version": 1}
                            report.update(timestamp=datetime.now(timezone.utc).isoformat(),
                                          task_id=arguments.task_id, native_capture_incomplete=True)
                            write_last_turn_report(repository, report)
                        if arguments.json:
                            print(json.dumps(result, ensure_ascii=False))
                        elif result.get("warning") and arguments.host != "cursor":
                            print(result["warning"])
                return 0

            if arguments.command in ("record-result", "prepare-result", "replay-result"):
                raw = (
                    sys.stdin.read()
                    if arguments.input == "-"
                    else Path(arguments.input).read_text(encoding="utf-8")
                )
                try:
                    data = json.loads(raw)
                except ValueError as error:
                    raise ContextError("The result must be a JSON object") from error
                revision = arguments.revision
                reason = arguments.reason or (
                    "Saved with context.py record-result; agent-attested, "
                    "not reviewed by a person"
                    if arguments.attestation == "agent"
                    else "Saved with context.py record-result; reviewed by a person"
                )
                operation = prepare_result if arguments.command == "prepare-result" else record_result
                result = operation(
                    repository,
                    arguments.task_id,
                    arguments.result_id,
                    revision,
                    data,
                    owner=owner,
                    reason=reason,
                    attestation=arguments.attestation,
                    **({"prepared": arguments.prepared_scope,
                        "replay_only": arguments.command == "replay-result"}
                       if arguments.command != "prepare-result" else {}),
                )
                if arguments.json or arguments.command == "prepare-result":
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    states = ", ".join(
                        f"{item['type']} {item['title']!r} {item['state']}"
                        for item in result["records"]
                    ) or "no learnings"
                    print(
                        ("Result already recorded" if result["replayed"] else "Result recorded")
                        + f": {states}."
                    )
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
                    and arguments.authority is None
                    and not arguments.replace_next_steps
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
                        replace_next_steps=arguments.replace_next_steps,
                    )
                promotion = promote_on_resolution(repository, result, owner)
                if arguments.json:
                    print(json.dumps(
                        {**result, "promotion": promotion} if promotion else result,
                        ensure_ascii=False,
                    ))
                else:
                    print(
                        f"Project Brain {result['type']} updated: {result['id']}."
                    )
                    for item in (promotion or {}).get("promoted", []):
                        print(f"Promoted to durable memory: {item['memory_id']}.")
                    for item in (promotion or {}).get("blocked", []):
                        print(f"warning: not promoted: {item['reason']}")
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

            if arguments.command == "retrieval-report":
                result = retrieval_report(
                    repository, scope=arguments.scope, since=arguments.since
                )
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                elif not result["turns"]:
                    print(
                        f"No retrieval manifests in scope {arguments.scope}. "
                        "Lightweight mode writes none."
                    )
                else:
                    gate = result["gate"]
                    print(
                        f"Retrieval report ({result['turns']} turn(s), scope "
                        f"{result['scope']}): {result['empty_selection']} with an "
                        "empty selection."
                    )
                    print(
                        "  query source: "
                        + (
                            ", ".join(
                                f"{name} {count}"
                                for name, count in result["query_source"].items()
                            )
                            or "not recorded"
                        )
                    )
                    print(
                        f"  gate: {gate['decided']} decided; shadow would skip "
                        f"{gate['would_skip_rate'] if gate['would_skip_rate'] is not None else 'n/a'}; "
                        "enforce withheld "
                        f"{gate['withheld_rate'] if gate['withheld_rate'] is not None else 'n/a'}"
                        + (
                            " ("
                            + ", ".join(
                                f"{name} {count}"
                                for name, count in gate["skip_reasons"].items()
                            )
                            + ")"
                            if gate["skip_reasons"]
                            else ""
                        )
                    )
                    for label in ("top_candidate_score", "top_delivered_score"):
                        band = result[label]
                        print(
                            f"  {label.replace('_', ' ')}: "
                            f"p50 {band['p50']} p95 {band['p95']} max {band['max']} "
                            f"({band['samples']} sample(s))"
                        )
                    for name, band in result["phase_seconds"].items():
                        print(
                            f"  phase {name}: p50 {band['p50']} p95 {band['p95']} "
                            f"({band['samples']} sample(s))"
                        )
                    if result["excluded_reasons"]:
                        print(
                            "  excluded: "
                            + ", ".join(
                                f"{name} {count}"
                                for name, count in result["excluded_reasons"].items()
                            )
                        )
                    if result["match_strength"]:
                        print(
                            "  match: "
                            + ", ".join(
                                f"{name} {count}"
                                for name, count in result["match_strength"].items()
                            )
                        )
                    for path_name, count in result["top_paths"].items():
                        print(f"    {count:>4}  {path_name}")
                return 0

            if arguments.command == "health":
                result = refresh_health(repository, arguments.window)
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                elif not result["turns"]:
                    print(
                        "No refresh health records yet "
                        "(memory-bank/local/refresh-health.ndjson)."
                    )
                else:
                    print(f"Refresh health ({result['turns']} turn(s)):")
                    for name, band in result["phase_seconds"].items():
                        print(
                            f"  phase {name}: p50 {band['p50']} p95 {band['p95']} "
                            f"max {band['max']} ({band['samples']} sample(s))"
                        )
                    print(
                        f"  timeouts: {result['timeout_rate']}; lossy capsules: "
                        f"{result['lossy_capsule_rate']}; incomplete layers: "
                        f"{result['incomplete_layers_rate']}"
                    )
                    if result["hook_status"]:
                        print(
                            "  hook status: "
                            + ", ".join(
                                f"{name} {count}"
                                for name, count in result["hook_status"].items()
                            )
                        )
                    if result.get("source_notices"):
                        notices = result["source_notices"]
                        print("  " + _source_notice_text(notices["totals"])
                              + f" ({notices['records']} record(s))")
                return 0

            if arguments.command == "status":
                layers = {layer: 0 for layer in DOCUMENT_LAYERS}
                consolidation = consolidation_counters(repository, mode)
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
                    "automatic_memory": automatic_memory_readiness(repository),
                    "consolidation": consolidation,
                    "workspace": workspace_roots.describe(repository),
                }
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    counts = result["consolidation"]
                    unknown = "unavailable"
                    print(
                        "Consolidation: "
                        f"{counts['promotable'] if counts['promotable'] is not None else unknown}"
                        " promotable candidate(s), "
                        f"{counts['blocked'] if counts['blocked'] is not None else unknown}"
                        " blocked, "
                        f"{counts['applied'] if counts['applied'] is not None else unknown}"
                        " promotion(s) applied, "
                        f"{counts['chunks'] if counts['chunks'] is not None else unknown}"
                        " durable chunk(s)."
                    )
                    print(
                        f"Context index: {result['documents']} documents, "
                        f"{result['episodes']} episodes, {result['working']} working tasks "
                        f"({', '.join(f'{layer}: {count}' for layer, count in layers.items())}; "
                        f"{result['database']})."
                    )
                    readiness = result["automatic_memory"]
                    print(
                        f"Automatic memory: {readiness['status']} — "
                        f"{readiness['reason']}"
                    )
                    if readiness["remediation"]:
                        print(f"Remediation: {readiness['remediation']}")
                return 0

            if arguments.command == "links":
                selected, withheld = linked_documents(
                    connection,
                    repository,
                    load_config(repository),
                    arguments.path,
                    prefix=arguments.prefix,
                    ref_kinds=("source", "file"),
                )
                result = {
                    "path": arguments.path,
                    "prefix": arguments.prefix,
                    "documents": [
                        {
                            key: item[key]
                            for key in (
                                "path", "layer", "kind", "title",
                                "ref_path", "ref_kind", "authority", "lifecycle",
                            )
                        }
                        for item in selected
                    ],
                    "excluded": withheld,
                }
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                elif not selected:
                    print(
                        f"No eligible document cites or touched {arguments.path}."
                        + (
                            f" {len(withheld)} withheld by policy or freshness."
                            if withheld
                            else ""
                        )
                    )
                else:
                    for item in result["documents"]:
                        print(
                            f"{item['layer']} {item['kind']}: "
                            f"{item['path']} — {item['title']}"
                        )
                        if item["ref_kind"] == "file":
                            print(f"  touched {item['ref_path']}")
                        else:
                            print(f"  cites {item['ref_path']} ({item['ref_kind']})")
                    for item in withheld:
                        print(f"withheld {item['path']}: {item['reason']}")
                return 0

            if arguments.command == "search":
                if arguments.limit < 1:
                    raise ContextError("--limit must be a positive integer")
                documents = search_documents(
                    connection, arguments.query, arguments.limit, arguments.layer, repository=repository
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
                source_provenance = {} if arguments.provenance_response else None
                result = assemble_capsule(
                    connection,
                    repository,
                    mode=mode,
                    query=arguments.query,
                    task_id=arguments.task_id,
                    limit=arguments.limit,
                    ephemeral=arguments.ephemeral,
                    gate_mode=gate_mode,
                    paths=arguments.paths,
                    source_hints=arguments.source_hints_json,
                    trace_paths=arguments.trace_paths,
                    host=arguments.host,
                    entry_point=arguments.command,
                    render=not arguments.json,
                    source_provenance_sink=source_provenance,
                    # AGENTS.md has an agent retrieve at the start of a complex
                    # request, often on a branch the first checkpoint has not
                    # provisioned yet: answer with the knowledge and say so,
                    # as the prompt hook does, instead of failing outright.
                    allow_unprovisioned=True,
                )
                if arguments.json:
                    transport = ({'response_profile': SOURCE_PROVENANCE_PROFILE,
                                  'capsule': result, 'source_provenance': source_provenance or None}
                                 if arguments.provenance_response else result)
                    print(serialize_capsule(transport))
                else:
                    print_capsule(result, capsule_excerpts(connection, result), repository=repository)
                return 0

            if arguments.command == "hook-context":
                result = assemble_hook_context(
                    connection,
                    repository,
                    mode=mode,
                    task_id=arguments.task_id,
                    gate_mode=gate_mode,
                    host=arguments.host,
                    render=not arguments.json,
                    delivery_profile=delivery_profile,
                    delivery_admission=delivery_admission,
                )
                if delivery_admission is not None:
                    delivery_admission()
                if result is None:
                    # A valid current branch with no meaningful change has no
                    # context. Cursor uses this distinct code to remove a
                    # foreign branch's stale rule; real failures remain code 1.
                    return 3
                gate = result.get("gate")
                if (
                    isinstance(gate, dict)
                    and gate.get("mode") == "enforce"
                    and gate.get("decision") == "skip"
                    and not (delivery_profile == 'cursor-rule' and gate.get('reason') in ('no-relevant-match','empty-after-filter'))
                ):
                    # Print nothing and exit with a status the Cursor delivery
                    # treats as "keep what you have". Rendering the withheld
                    # capsule would satisfy that hook's `working:*` check and
                    # overwrite the rule with an empty one on every skipped
                    # turn, which is the opposite of what a skip means.
                    return 4
                # Cursor's read path never enters the refresh branch, so
                # without this the health series would systematically
                # under-report the one client whose capsule is a turn stale.
                append_refresh_health(
                    repository,
                    {
                        "at": utc_now(),
                        "mode": mode,
                        "omitted": result.get("omitted"),
                        "source": "hook-context",
                    },
                )
                if delivery_admission is not None:
                    delivery_admission()
                if arguments.json:
                    print(serialize_capsule(result))
                else:
                    print_capsule(result, capsule_excerpts(connection, result), repository=repository, delivery_profile=delivery_profile)
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
                source_provenance = {} if arguments.provenance_response else None
                topic_plan = (_cursor_topic_begin(connection, repository, arguments.task_id, mode)
                              if delivery_profile == 'cursor-rule' else None)
                retrieval_seconds = 0.0
                if arguments.query is not None:
                    retrieval_started = time.monotonic()
                    query, query_source = arguments.query, 'prompt'
                    try:
                        # --query accepts the raw prompt: the hook passes it
                        # as-is and distillation to informative terms happens
                        # here. The raw text is screened before any of it can
                        # reach a query or a manifest.
                        reject_secrets("Task Capsule", [arguments.query])
                        query = arguments.query if warnings else distill_capsule_query(
                            connection, arguments.query
                        )
                        query_source = "prompt"
                        if delivery_profile == 'cursor-rule' and getattr(arguments,'cursor_continuation',False):
                            current_topic = _cursor_topic_query(connection, repository, arguments.task_id, mode, topic_plan)
                            if current_topic is not None:
                                query, query_source = current_topic
                        elif mode == "governed" and getattr(arguments, "cursor_continuation", False):
                            current_task_query = _cursor_continuation_query(
                                connection, repository, arguments.task_id,
                            )
                            if current_task_query is not None:
                                query, query_source = current_task_query
                        capsule = assemble_capsule(
                            connection,
                            repository,
                            mode=mode,
                            query=query,
                            task_id=arguments.task_id,
                            limit=arguments.limit,
                            ephemeral=arguments.ephemeral,
                            refresh_index=False,
                            index_warnings=warnings,
                            index_notices=index_result.get('source_notices'),
                            query_source=query_source,
                            phase_seconds=index_result.get("phase_seconds"),
                            gate_mode=gate_mode,
                            host=arguments.host,
                            entry_point="refresh",
                            allow_unprovisioned=True,
                            session_id=arguments.session_id,
                            transcript=arguments.transcript,
                            paths=arguments.paths,
                            source_hints=arguments.source_hints_json,
                            trace_paths=arguments.trace_paths,
                            # Every refresh with a query renders: hosts put
                            # capsule_text in front of the model.
                            render=True,
                            prompt=arguments.query,
                            automatic_links=True,
                            delivery_profile=delivery_profile,
                            source_provenance_sink=source_provenance,
                            delivery_admission=delivery_admission,
                            capsule_profile=arguments.capsule_profile,
                        )
                        # What a governed capsule says about the turn's own task
                        # - its record gone or unreadable - is this refresh's
                        # warning too, as the index's warnings are the capsule's.
                        # (A lightweight capsule warns "not found" until the
                        # first checkpoint; that stays the capsule's.)
                        if mode == "governed":
                            for name, damage in _QUARANTINED:
                                # Said after retrieval: the index is new, not stale.
                                capsule.setdefault("warnings", []).append(
                                    f"The memory database was damaged ({damage}) and was rebuilt; "
                                    f"the old one is memory-bank/local/{name}.")
                            warnings.extend(item for item in capsule.get("warnings") or []
                                            if item not in warnings)
                    except SourceGitError:
                        raise
                    except (ContextError, BrainError, RetrievalError) as error:
                        # A capsule needs an active task; the layer refresh
                        # above stands on its own and is already done.
                        # Name the consequence, not just the cause: an
                        # unexplained "unavailable" reads as a formality, and
                        # the turn proceeds as if task context had been
                        # consulted when none was assembled.
                        warnings.append(
                            f"Capsule unavailable: {error}. No task context "
                            "was assembled this turn; read the canonical "
                            "sources directly."
                        )
                    if topic_plan is not None:
                        _cursor_topic_finish(connection, repository, arguments.task_id, mode, topic_plan, query,
                            bool(getattr(arguments,'cursor_topic_eligible',False)) and capsule is not None,
                            warnings, continued=query_source == 'continuation',
                            acknowledgement=bool(getattr(arguments,'cursor_continuation',False)),
                            selected=isinstance(capsule, dict) and any(
                                capsule.get(layer) for layer in ('procedural', 'semantic', 'episodic')))
                    retrieval_seconds = time.monotonic() - retrieval_started
                elif topic_plan is not None:
                    _cursor_topic_write(connection, topic_plan[1], None)
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
                    # Writes no longer fail on a chunk past its review date,
                    # which was the only place that ever said so out loud.
                    # The chunk still leaves retrieval; this keeps the loss
                    # visible on every turn until someone re-verifies it.
                    "overdue_review": sum(
                        1
                        for item in index_result.get("excluded", [])
                        if isinstance(item, dict)
                        and item.get("reason") == "overdue-review"
                    ),
                    "parity_drift": index_result.get("parity_drift", []),
                    # Wall-clock seconds per phase, so an operator can see
                    # which side of the work approaches the hook budget.
                    "phases": phases,
                    "warnings": warnings,
                    "capsule": capsule,
                }
                if arguments.provenance_response:
                    result['response_profile'] = SOURCE_PROVENANCE_PROFILE
                    result['source_provenance'] = source_provenance or None
                source_notices = _normalized_source_notices(index_result.get('source_notices'))
                if source_notices:
                    result['source_notices'] = source_notices
                # A prompt the sanitizer left nothing of is still this turn:
                # the host learns memory had nothing to search, and the
                # operator report stays out of the prompt.
                withheld = bool(getattr(arguments, "query_withheld", False))
                if withheld:
                    result["query_withheld"] = True
                excerpts = (
                    capsule_excerpts(connection, capsule) if capsule is not None else {}
                )
                if capsule is not None:
                    # What a host puts in front of the model: Harness inserts
                    # this text instead of the JSON, which repeated every item
                    # three times and carried no answer-bearing text.
                    render_trace: dict[str, object] = {}
                    result["capsule_text"] = "\n".join(
                        render_capsule_lines(capsule, excerpts, trace=render_trace, delivery_profile=delivery_profile)
                    )
                    record_render_trace(repository, capsule, render_trace)
                elif delivery_profile == 'cursor-rule':
                    result['capsule_text'] = ('working: unavailable\nwarning: memory refresh incomplete; '
                                              'verify current canonical sources')
                if arguments.validate:
                    errors = validate_repository(repository, check_freshness=False)
                    result["brain_validation"] = "valid" if not errors else "invalid"
                    warnings.extend(errors)
                # The series the `health` command reads. Deliberately carries
                # no query text and no selected paths: a health record must
                # not become a second channel for what the manifest already
                # keeps under privacy rules.
                emit_refresh_telemetry(repository, capsule, retrieval_seconds)
                health_record = {
                    "at": utc_now(),
                    "mode": mode,
                    "phase_seconds": phases,
                    "omitted": (capsule or {}).get("omitted"),
                    "layers_updated": all(layers[layer] == "updated" for layer in DOCUMENT_LAYERS),
                    "warnings": len(warnings),
                    "source": "refresh",
                }
                if source_notices:
                    health_record['source_notices'] = source_notices
                def capture_diagnosis(render: Optional[dict[str, object]]) -> None:
                    receipt_sink = None
                    try:
                        receipt_sink = {}
                    except Exception:
                        pass
                    try:
                        diagnosis = (diagnose_funnel(repository, capsule, render, _ordinary_receipt_sink=receipt_sink)
                                     if receipt_sink is not None else diagnose_funnel(repository, capsule, render))
                    except Exception:
                        return
                    try:
                        if valid_funnel_diagnosis(diagnosis):
                            health_record["funnel_diagnosis"] = diagnosis
                            result["funnel_diagnosis"] = diagnosis
                    except Exception:
                        pass
                    try:
                        receipt = normalize_ordinary_receipt(receipt_sink.get("ordinary_receipt")) if receipt_sink is not None else None
                        if receipt is not None:
                            health_record["ordinary_receipt"] = receipt
                            result["ordinary_receipt"] = receipt
                    except Exception:
                        pass
                if arguments.json:
                    try:
                        capture_diagnosis(render_trace if capsule is not None else None)
                    finally:
                        append_refresh_health(repository, health_record)
                    if delivery_admission is not None:
                        delivery_admission()
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    # Keep the original timestamp/fields and retain one row even
                    # when stdout fails before the printed-form finalizer runs.
                    try:
                        capture_diagnosis(None)
                        if delivery_admission is not None:
                            delivery_admission()
                        counts = result["counts"]
                        # With --query this is a turn's delivery, read by a model
                        # on every prompt: the layer and timing report is only
                        # worth its characters when something is wrong. Without
                        # it, it is the operator's /memory report and stays whole.
                        per_turn = capsule is not None or withheld
                        healthy = all(
                            layers[layer] == "updated" for layer in DOCUMENT_LAYERS
                        )
                        # A prompt hook on a checkout that names no branch - a
                        # detached HEAD in a rebase or a bisect - has no task to
                        # read: its refresh only keeps the index current, and
                        # the operator's layer report reached the model as four
                        # lines of noise on every prompt. Failures still print.
                        if (healthy and arguments.host != "cli" and arguments.query is None
                                and getattr(arguments, "task_id", None) is None and not withheld):
                            return 0
                        for layer in DOCUMENT_LAYERS:
                            if per_turn and healthy:
                                break
                            suffix = (
                                f" ({counts[layer]} documents)" if layer in counts else ""
                            )
                            print(f"{layer}: {result[layer]}{suffix}")
                        if result["overdue_review"]:
                            print(
                                f"memory review: {result['overdue_review']} chunk(s) "
                                "overdue, not served until re-verified (bank-audit)"
                            )
                        if source_notices and capsule is None:
                            print(_source_notice_text(source_notices))
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
                        # The hook runs under a hard budget and the JSON branch it
                        # does not take was the only place these numbers appeared,
                        # so a run creeping toward the ceiling was invisible to
                        # everyone who could act on it.
                        elapsed = sum(
                            float(phases[name])
                            for name in ("stat", "index", "retrieval")
                            if isinstance(phases.get(name), (int, float))
                        )
                        if not per_turn or elapsed >= SLOW_TURN_SECONDS:
                            print(
                                "phases: "
                                + " ".join(
                                    f"{name} {round(float(phases[name]) * 1000)}ms"
                                    for name in ("stat", "index", "retrieval")
                                    if isinstance(phases.get(name), (int, float))
                                )
                            )
                        if "brain_validation" in result:
                            print(f"brain-validation: {result['brain_validation']}")
                        if capsule is None:
                            for warning in warnings:
                                print(f"warning: {warning}")
                        if capsule is not None:
                            print_capsule(capsule, excerpts, repository=repository, delivery_profile=delivery_profile,
                                          _diagnosis_sink=capture_diagnosis)
                    finally:
                        append_refresh_health(repository, health_record)
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
                # A record whose cited file changed is intact: retrieval serves
                # it marked for checking. It is listed, and does not fail the
                # project the way a broken record does.
                errors = validate_repository(repository, check_freshness=False)
                states = stale_record_states(repository)
                result = {
                    "valid": not errors, "errors": errors, "stale": [path for path, _ in states],
                }
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    for error in errors:
                        print(error)
                    if not errors:
                        print("Project Brain validation passed.")
                    for path, state in states:
                        where = (
                            " (archived: never retrieved)"
                            if "/project-brain/archive/" in path.replace("\\", "/") else ""
                        )
                        reading = {
                            "changed": "a cited source changed after the record was written; "
                                       "retrieval serves it marked for checking",
                            "source-missing": "a cited source no longer exists; "
                                              "retrieval leaves the record out",
                            "source-undigested": "a cited source has no stored digest; "
                                                 "retrieval leaves the record out",
                        }[state]
                        print(f"Warning: {path}: {reading}{where}")
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
                configured = str(load_config(repository)["canonical_edition"])
                canonical = effective_canonical_edition(repository, configured)
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
                if canonical != configured:
                    result["configured_canonical_edition"] = configured
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
                        stand_in = (
                            f"; configured {configured} is not installed here"
                            if canonical != configured
                            else ""
                        )
                        print(f"{scope} parity passed ({canonical} canonical{stand_in}).")
                return 0 if result["valid"] else 1

            if arguments.command == "compact":
                result = compact(repository)
                if arguments.json:
                    print(json.dumps(result))
                else:
                    print(f"Project Brain compacted: {result['moved']} record(s) archived.")
                return 0

            if arguments.command == "consolidate":
                result = consolidate(
                    repository,
                    apply=arguments.apply,
                    ticket=arguments.ticket,
                    owner=owner,
                )
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                else:
                    states = [item["state"] for item in result["tickets"]]
                    if arguments.apply:
                        print(
                            f"Created {result['created']} finding(s), updated "
                            f"{result['updated']}; {result['superseded']} event(s) "
                            f"superseded, {result['blocked']} blocked."
                        )
                    else:
                        print(
                            f"Planned {states.count('planned')} finding(s), "
                            f"{states.count('planned-update')} update(s); "
                            f"{sum(item.get('supersede', 0) for item in result['tickets'])} "
                            "event(s) to supersede."
                        )
                    for item in result["tickets"]:
                        print(
                            f"  {item['ticket']}: {item['state']} "
                            f"({item['events']} event(s))"
                            + (f" - {item.get('reason')}" if item.get("reason") else "")
                        )
                return 0

            if arguments.command == "bank-audit":
                result = audit_bank(repository, provenance=arguments.provenance)
                if arguments.json:
                    print(json.dumps(result))
                else:
                    print(
                        f"Memory bank audit: {result['chunks']} chunk(s), "
                        f"{len(result['source_changed'])} with a changed source, "
                        f"{len(result['overdue_review'])} overdue for review, "
                        f"{len(result['undigested'])} without source digests, "
                        f"{len(result['duplicates'])} near-duplicate pair(s)."
                    )
                    for pair in result["duplicates"]:
                        print(
                            f"  near-duplicate: {pair['left']} ~ {pair['right']} "
                            f"({pair['similarity']})"
                        )
                    for item in result["source_changed"]:
                        print(
                            f"  source-{item['state']}: {item['chunk']} "
                            f"cites {item['source']}"
                        )
                    for item in result["overdue_review"]:
                        print(
                            f"  overdue-review: {item['chunk']} "
                            f"(due {item['review_after']})"
                        )
                    for path in result["undigested"]:
                        print(f"  undigested: {path}")
                    provenance = result.get("provenance")
                    if provenance is not None:
                        # Only asked for: without --provenance nothing was
                        # counted, and "0 auto-promoted" would say otherwise.
                        print(
                            f"  provenance: {len(provenance.get('auto_promoted') or [])} "
                            f"auto-promoted, {len(provenance.get('external_sources') or [])} "
                            f"with external sources, "
                            f"{len(provenance.get('instruction_like') or [])} instruction-like."
                        )
                return 0

            if arguments.command == "bank-reverify":
                result = reverify_chunk(
                    repository,
                    arguments.memory_id,
                    review_after=arguments.review_after,
                )
                if arguments.json:
                    print(json.dumps(result))
                else:
                    print(
                        f"Re-verified {result['id']} against "
                        f"{len(result['source_digests'])} source(s) on "
                        f"{result['last_verified']}; next review "
                        f"{result['review_after']}."
                    )
                return 0

            if arguments.command == "bank-retire":
                result = retire_chunk(
                    repository,
                    arguments.memory_id,
                    valid_to=arguments.valid_to,
                    superseded_by=arguments.superseded_by,
                    reason=arguments.reason,
                )
                if arguments.json:
                    print(json.dumps(result))
                else:
                    replacement = (
                        f", superseded by {result['superseded_by']}"
                        if result["superseded_by"]
                        else ""
                    )
                    print(
                        f"Retired {result['id']}: {result['status']} "
                        f"as of {result['valid_to']}{replacement}. "
                        f"Index rewritten with {result['chunks']} chunk row(s)."
                    )
                return 0

            if arguments.command == "bank-quarantine":
                result = retire_chunk(
                    repository,
                    arguments.memory_id,
                    valid_to=date.today().isoformat(),
                    reason=arguments.reason or "quarantined",
                )
                if arguments.json:
                    print(json.dumps(result))
                else:
                    print(
                        f"Quarantined {result['id']}: {result['status']} as of "
                        f"{result['valid_to']}; index rewritten with "
                        f"{result['chunks']} chunk row(s)."
                    )
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

            if arguments.command == "promote-auto":
                # The turn boundary is the only other caller, and it waits for
                # `--flush-after` turns. Knowledge recorded deliberately, as a
                # reviewed session result is, should not wait for a counter.
                result = auto_promote(repository, owner=owner)
                if arguments.json:
                    print(json.dumps(result, ensure_ascii=False))
                elif not result["enabled"]:
                    print("Automatic promotion is disabled in runtime.json.")
                else:
                    print(
                        f"Promoted {len(result['promoted'])} record(s); "
                        f"{len(result['blocked'])} blocked, "
                        f"{len(result['failed'])} failed, "
                        f"{result['skipped']} deferred."
                    )
                return 0

            raise ContextError(f"Unsupported command: {arguments.command}")
        finally:
            connection.close()
    except (ContextError, HandoffError, BrainError, RetrievalError, OSError, sqlite3.Error) as error:
        damaged = _damaged_database(error)
        if damaged is not None:
            raise _DamagedDatabase(damaged, error) from error
        print(f"context: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
