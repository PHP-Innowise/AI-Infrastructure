#!/usr/bin/env python3
"""Governed FTS5 indexing, budgeting, and retrieval manifests."""

from __future__ import annotations

import hashlib
import json
import re
import sqlite3
import subprocess
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
    render_current_state,
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
# Skills whose body documents the host tool itself rather than a workflow this
# repository owns. `skill-creator` instructs the agent to drive its own product
# CLI - `codex exec`, `cursor-agent --print`, `claude -p` - with different
# environment variables and a different extension model per tool (Cursor builds
# command/agent wrappers; Codex is forbidden from creating them). Byte-parity
# would mean telling a Codex user to run `cursor-agent`, so these are compared
# per edition by review, not by the mirror check. This is the same category as
# `SKILL FLOW.md`, and the list is deliberately explicit: an entry here is a
# documented exemption, not a way to silence real drift.
EDITION_OWNED_SKILLS = frozenset({"skill-creator"})

# ---------------------------------------------------------------------------
# Mirror rewrite map - the single source of truth for how this edition's
# per-tool mirrors (.agents / .claude / .cursor / .codex) are derived from
# their canonical copies. `scripts/build_mirrors.py` at the monorepo root
# imports MIRROR_RULES from this file and can verify (--check) or regenerate
# (--write) every mirror. The map travels with the edition, so a copied
# edition keeps its own mirroring contract.
#
# Schema (per class):
#   name              stable identifier for reporting
#   canonical         edition-relative directory holding the source of truth
#   only              optional explicit list of relative paths (whitelist);
#                     without it the whole canonical tree is mirrored
#   mirrors           {mirror_dir: spec}; spec fields:
#                       transform     "copy" (default), "cursor-command",
#                                     or "cursor-agent"
#                       replacements  ordered [old, new] literal substitutions
#                                     applied to the file text
#                       skip          per-mirror relative paths that are NOT
#                                     mirrored (mirror-owned or absent)
#                       description_overrides / quote_description
#                                     parameters for "cursor-command"
#   skip              relative paths (or "dir/" prefixes) excluded from the
#                     class for every mirror - each entry is a documented
#                     exemption, not a way to silence real drift
#   skip_by_framework additional skips keyed by the `framework` value in
#                     project-brain/config/runtime.json, so the map itself
#                     stays byte-identical across editions
#
# Transforms:
#   copy            byte-identical copy after `replacements`
#   cursor-command  Claude command -> Cursor command: the orchestration
#                   frontmatter (spawns/phase/flow-*) is replaced by
#                   `name:` (file stem) + `description:` (first body
#                   paragraph unless overridden); commands that already
#                   carry `name:`/`description:` keep their frontmatter;
#                   `replacements` then apply to the body
#   cursor-agent    Claude agent -> Cursor agent: the Claude-specific
#                   `model:`, `invokes:` and `phase:` frontmatter keys are
#                   dropped (Cursor keeps exactly `name` + `description`);
#                   the body is copied verbatim
# ---------------------------------------------------------------------------

# loop-detection.sh path extraction: Claude always sends "file_path"; Cursor
# and Codex payloads may use "file_path" or "path", so their mirrors gain a
# fallback extraction.
_HOOK_PATH_EXTRACT = (
    "# Extract file path from JSON input (POSIX-compatible, no grep -P)\n"
    "FILE_PATH=$(echo \"$INPUT\" | sed -n "
    "'s/.*\"file_path\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p'"
    " | head -1)\n"
)
_HOOK_PATH_EXTRACT_CURSOR = (
    "# Extract file path from JSON input (POSIX-compatible, no grep -P).\n"
    "# Cursor payloads may use \"file_path\" or \"path\"; try both.\n"
    "FILE_PATH=$(echo \"$INPUT\" | sed -n "
    "'s/.*\"file_path\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p'"
    " | head -1)\n"
    "if [ -z \"$FILE_PATH\" ]; then\n"
    "  FILE_PATH=$(echo \"$INPUT\" | sed -n "
    "'s/.*\"path\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p'"
    " | head -1)\nfi\n"
)
_HOOK_PATH_EXTRACT_CODEX = (
    "# Extract file path from JSON input (POSIX-compatible, no grep -P).\n"
    "# Codex payloads may use \"file_path\" or \"path\"; try both.\n"
    "FILE_PATH=$(echo \"$INPUT\" | sed -n "
    "'s/.*\"file_path\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p'"
    " | head -1)\n"
    "if [ -z \"$FILE_PATH\" ]; then\n"
    "  FILE_PATH=$(echo \"$INPUT\" | sed -n "
    "'s/.*\"path\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p'"
    " | head -1)\nfi\n"
)

MIRROR_RULES: dict[str, Any] = {
    "version": 1,
    "classes": [
        {
            # Skill bodies are byte-identical in every mirror. The canonical
            # tree is .agents/skills (declared as canonical_edition in
            # project-brain/config/runtime.json).
            "name": "skills",
            "canonical": ".agents/skills",
            "mirrors": {
                ".claude/skills": {"transform": "copy"},
                ".cursor/skills": {"transform": "copy"},
            },
            "skip": [
                # Tool-owned: skill-creator drives each host product's own
                # CLI and extension model, so its three copies are separate
                # generations by design (see EDITION_OWNED_SKILLS above).
                "skill-creator/",
                # Pair-owned: the .agents copy speaks in bare skill names
                # (Codex has no slash commands); the slash-command wording is
                # shared by Claude and Cursor via the skill-flow class below.
                "SKILL FLOW.md",
            ],
        },
        {
            # SKILL FLOW.md: Claude and Cursor share one byte-identical
            # slash-command edition; the .agents (Codex) edition is a
            # deliberate per-tool adaptation and is not generated.
            "name": "skill-flow",
            "canonical": ".claude/skills",
            "only": ["SKILL FLOW.md"],
            "mirrors": {
                ".cursor/skills": {"transform": "copy"},
            },
        },
        {
            # Hook scripts: canonical in .claude/hooks, adapted per tool with
            # ordered literal substitutions (event-name header, TRACK_DIR
            # namespace prefix, skills directory, scanned dot-directory, JSON
            # "path" fallback, /debugger -> systematic-debugger for Codex).
            "name": "hooks",
            "canonical": ".claude/hooks",
            "mirrors": {
                ".cursor/hooks": {
                    "transform": "copy",
                    "replacements": [
                        [
                            "# Claude hook event: PreToolUse (Write|Edit).",
                            "# Cursor hook event: afterFileEdit.",
                        ],
                        [
                            "# Hook type: PostToolUse:Edit",
                            "# Cursor hook event: afterFileEdit.",
                        ],
                        [_HOOK_PATH_EXTRACT, _HOOK_PATH_EXTRACT_CURSOR],
                        [
                            "/tmp/claude-loop-detection-",
                            "/tmp/cursor-loop-detection-",
                        ],
                        [
                            "SKILLS_DIR=\"$ROOT_DIR/.claude/skills\"",
                            "SKILLS_DIR=\"$ROOT_DIR/.cursor/skills\"",
                        ],
                        [" .claude; do", " .cursor; do"],
                    ],
                    # Cursor has no UserPromptSubmit-equivalent hook event, so
                    # the read half of automatic memory is deliberately absent
                    # from the Cursor mirror.
                    "skip": ["working-memory-read.sh"],
                },
                ".codex/hooks": {
                    "transform": "copy",
                    "replacements": [
                        [
                            "# Claude hook event: PreToolUse (Write|Edit).",
                            "# Codex hook event: PreToolUse "
                            "(self-filters to file-edit payloads).",
                        ],
                        [
                            "# Hook type: PostToolUse:Edit",
                            "# Codex hook event: PostToolUse "
                            "(self-filters to file-edit payloads).",
                        ],
                        [_HOOK_PATH_EXTRACT, _HOOK_PATH_EXTRACT_CODEX],
                        [
                            "/tmp/claude-loop-detection-",
                            "/tmp/codex-loop-detection-",
                        ],
                        [
                            "SKILLS_DIR=\"$ROOT_DIR/.claude/skills\"",
                            "SKILLS_DIR=\"$ROOT_DIR/.agents/skills\"",
                        ],
                        [" .claude; do", " .agents .codex; do"],
                        [
                            "- /debugger to investigate the root cause",
                            "- systematic-debugger to investigate the root cause",
                        ],
                        [
                            "consider using /debugger.",
                            "consider using systematic-debugger.",
                        ],
                    ],
                },
            },
            # Each tool documents its own registration model (settings.json vs
            # hooks.json vs config.toml), so every hooks README is mirror-owned.
            "skip": ["README.md"],
        },
        {
            # Slash commands: Claude's orchestration frontmatter is reduced to
            # Cursor's `name` + `description`; bodies are shared except for the
            # skills path and the backticked `$ARGUMENTS` guard wording.
            "name": "commands",
            "canonical": ".claude/commands",
            "mirrors": {
                ".cursor/commands": {
                    "transform": "cursor-command",
                    "replacements": [
                        [".claude/skills/", ".cursor/skills/"],
                        [
                            "If $ARGUMENTS is not empty",
                            "If `$ARGUMENTS` is not empty",
                        ],
                    ],
                },
            },
            "skip": [
                # Mirror-owned: the Cursor command invokes the codebase-mapper
                # skill inline instead of spawning a Task sub-agent.
                "codebase-mapper.md",
            ],
            "skip_by_framework": {
                # Tool-owned (skill-creator family): the Symfony Cursor copy
                # was rewritten to speak about Cursor skills, not Claude skills.
                "symfony": ["skill-creator.md"],
            },
        },
        {
            # Agent wrappers: Cursor mirrors every Claude agent with reduced
            # frontmatter (exactly `name` + `description`); bodies are shared.
            "name": "agents",
            "canonical": ".claude/agents",
            "mirrors": {
                ".cursor/agents": {"transform": "cursor-agent"},
            },
            "skip": [
                # Mirror-owned: the Cursor copies intentionally condense the
                # Output Format section into a single instruction line.
                "memory-bank-agent.md",
                "project-brain-agent.md",
            ],
            "skip_by_framework": {
                # Tool-owned (skill-creator family): the Symfony Cursor copy
                # was rewritten to speak about Cursor capabilities.
                "symfony": ["skill-creator-agent.md"],
            },
        },
        {
            # Governance documents: shared verbatim except for self-references
            # to the host tool's own directory tree.
            "name": "governance-docs",
            "canonical": ".claude",
            "only": ["DOD.md", "GOLDEN-PRINCIPLES.md", "STABILIZATION.md"],
            "mirrors": {
                ".cursor": {
                    "transform": "copy",
                    "replacements": [[".claude/", ".cursor/"]],
                },
                ".codex": {
                    "transform": "copy",
                    "replacements": [
                        [".claude/skills/", ".agents/skills/"],
                        [".claude/", ".codex/"],
                    ],
                },
            },
        },
    ],
}

# ---------------------------------------------------------------------------
# Cross-edition core - the Python runtime, its tests, the Brain schemas and
# the protocol are byte-identical across the PHP editions of the monorepo
# (Laravel, Symfony, "PHP Core") and MUST stay that way: a fix that lands in
# one edition and not the others silently forks the engine. `context.py
# parity --cross-edition` walks this manifest against every sibling edition
# it can find next to this one; a standalone (copied-out) edition has no
# siblings and skips the check.
# ---------------------------------------------------------------------------

CROSS_EDITION_SIBLINGS = ("Laravel", "Symfony", "PHP Core")

# Glob patterns, relative to an edition root, of files that must be
# byte-identical in every sibling edition. Composition verified by direct
# md5 comparison of the editions before the manifest was frozen.
CROSS_EDITION_CORE_MANIFEST = (
    "memory-bank/scripts/*.py",
    "memory-bank/tests/*.py",
    "project-brain/scripts/*.py",
    "project-brain/schemas/**/*",
    "project-brain/PROTOCOL.md",
    "project-brain/tests/*.py",
)

# Legitimate cross-edition differences, each with its justification. Entries
# ending in "/" match a whole subtree. None of these currently intersect the
# manifest above; they are recorded so a future manifest extension cannot
# accidentally turn known-deliberate divergence into reported drift.
CROSS_EDITION_ALLOWED_DRIFT = {
    # Each edition's README introduces its own framework and stack.
    "README.md": "edition-specific introduction",
    # Durable memory is edition content, not runtime: MEM-0001 in particular
    # ships three deliberate per-edition versions of the sync playbook.
    "memory-bank/README.md": "edition-specific durable-memory docs",
    "memory-bank/chunks/": "durable memory is edition content (MEM-0001)",
    # Skills speak the edition's framework language: verify/SKILL.md encodes
    # the edition's own verification pipeline, and framework skills
    # (eloquent, doctrine-migration-designer, ...) exist in one edition only.
    ".agents/skills/": "skills are edition-specific (verify, framework skills)",
    ".claude/skills/": "mirror of .agents/skills - same edition-specific content",
    ".cursor/skills/": "mirror of .agents/skills - same edition-specific content",
}

MANIFEST_SCOPES = ("governed", "local")
STOPWORD_DOCUMENT_RATIO = 0.5
MIN_TOKEN_COVERAGE = 2
DISTINCTIVE_DOCUMENT_RATIO = 0.1
MAPPED_COMMIT_PATTERN = re.compile(r"^mapped_commit:\s*([0-9a-fA-F]{7,40})\s*$", re.M)
MAPPED_SCOPE_PATTERN = re.compile(r"^mapped_scope:\s*(\S+)\s*$", re.M)
CODEBASE_MAP_MAX_DRIFT = 25
LOCAL_MANIFEST_RETENTION = 200
DELETE_CHUNK = 500
INDEX_CONFIG_KEY = "config-fingerprint"
INDEX_SKILL_KEY = "skill-tree-fingerprint"
INDEX_PARITY_KEY = "skill-parity-drift"

# Column weights for bm25(): path, layer, kind, title, summary, content.
# What a document declares itself to be about outranks what its body mentions.
BM25_WEIGHTS = (1.0, 1.0, 1.0, 2.0, 8.0, 1.0)

DocumentRow = tuple[str, str, str, str, str, str]
SourceState = dict[str, tuple[int, int]]


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
    connection.execute(
        """
        CREATE TABLE IF NOT EXISTS document_source_state(
            path TEXT PRIMARY KEY,
            mtime_ns INTEGER NOT NULL,
            size INTEGER NOT NULL
        )
        """
    )
    connection.execute(
        """
        CREATE TABLE IF NOT EXISTS index_state(
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
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


def codebase_map_drift(repository: Path, content: str) -> Optional[int]:
    """Count commits landed on the mapped scope since a map was written.

    A codebase map describes code rather than itself, so its own bytes staying
    unchanged proves nothing: the content hash that keeps every other document
    honest cannot detect that the code moved on underneath it.

    Returns `None` when drift cannot be established — no recorded commit, or a
    commit this clone does not have. That is reported rather than treated as
    fresh, because an unverifiable map is exactly the one not to trust.
    """
    recorded = MAPPED_COMMIT_PATTERN.search(content)
    if recorded is None:
        return None
    arguments = [
        "git", "-C", str(repository), "rev-list", "--count",
        f"{recorded.group(1)}..HEAD",
    ]
    scope = MAPPED_SCOPE_PATTERN.search(content)
    if scope is not None:
        # After `--` the value is a pathspec, so a scope taken from the file
        # cannot turn into an option.
        arguments += ["--", scope.group(1)]
    try:
        result = subprocess.run(
            arguments,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            check=False,
        )
    except OSError:
        return None
    if result.returncode != 0:
        return None
    counted = result.stdout.decode("utf-8", "replace").strip()
    return int(counted) if counted.isdigit() else None


def load_index_state(connection: sqlite3.Connection) -> dict[str, str]:
    ensure_metadata_tables(connection)
    return {
        row[0]: row[1]
        for row in connection.execute("SELECT key, value FROM index_state")
    }


def store_index_state(connection: sqlite3.Connection, state: dict[str, str]) -> None:
    """Persist index fingerprints inside the caller's transaction."""
    connection.executemany(
        "INSERT INTO index_state(key, value) VALUES (?, ?) "
        "ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        sorted(state.items()),
    )


def config_fingerprint(repository: Path) -> str:
    path = brain_root(repository) / "config" / "runtime.json"
    try:
        return _content_hash(path.read_text(encoding="utf-8"))
    except OSError:
        return "absent"


def skill_tree_fingerprint(repository: Path) -> str:
    """Fingerprint every mirrored skill file from stat metadata alone.

    Parity compares all editions, but only the first discovered copy of a skill
    reaches the index, so the per-document stat cache cannot notice a drifting
    mirror. This fingerprint can, without reading any file.
    """
    digest = hashlib.sha256()
    for edition in SKILL_EDITIONS:
        root = repository / edition / "skills"
        if not root.is_dir():
            continue
        for path in sorted(root.glob("**/*.md")):
            if not path.is_file() or path.is_symlink():
                continue
            status = path.stat()
            digest.update(
                f"{edition}\0{path.relative_to(root).as_posix()}\0"
                f"{status.st_mtime_ns}\0{status.st_size}\n".encode("utf-8")
            )
    return digest.hexdigest()


def reusable_source_state(
    connection: sqlite3.Connection, repository: Path
) -> tuple[SourceState, dict[str, str]]:
    """Return the retainable stat cache plus the fingerprints that gate it."""
    ensure_metadata_tables(connection)
    fingerprints = {
        INDEX_CONFIG_KEY: config_fingerprint(repository),
        INDEX_SKILL_KEY: skill_tree_fingerprint(repository),
    }
    stored = load_index_state(connection)
    if stored.get(INDEX_CONFIG_KEY) != fingerprints[INDEX_CONFIG_KEY]:
        # Runtime configuration decides eligibility for every indexed record,
        # so a configuration change invalidates the whole cache.
        return {}, fingerprints
    indexed = {row[0] for row in connection.execute("SELECT path FROM documents")}
    return {
        row[0]: (int(row[1]), int(row[2]))
        for row in connection.execute(
            "SELECT path, mtime_ns, size FROM document_source_state"
        )
        if row[0] in indexed
    }, fingerprints


def skill_mirror_drift(repository: Path, canonical_edition: str) -> list[dict[str, object]]:
    logical: dict[str, dict[str, Path]] = {}
    for edition in SKILL_EDITIONS:
        root = repository / edition / "skills"
        if not root.is_dir():
            continue
        for path in sorted(root.glob("**/*.md")):
            if path.is_file() and not path.is_symlink():
                key = path.relative_to(root).as_posix()
                if key == "SKILL FLOW.md" or key.split("/")[0] in EDITION_OWNED_SKILLS:
                    continue
                logical.setdefault(key, {})[edition] = path
    present_editions = [
        edition for edition in SKILL_EDITIONS if (repository / edition / "skills").is_dir()
    ]
    drift: list[dict[str, object]] = []
    for key, copies in sorted(logical.items()):
        if canonical_edition not in copies:
            # A file only a mirror carries is drift too. Skipping it here is why
            # an accidental extra copy could sit in .claude indefinitely without
            # parity ever mentioning it.
            drift.append(
                {
                    "logical_path": key,
                    "canonical": canonical_edition,
                    "mismatched": sorted(copies),
                    "reason": "absent from canonical",
                }
            )
            continue
        canonical = copies[canonical_edition].read_bytes()
        mismatched = sorted(
            edition for edition, path in copies.items()
            if edition != canonical_edition and path.read_bytes() != canonical
        )
        missing = sorted(set(present_editions) - set(copies))
        if mismatched or missing:
            entry: dict[str, object] = {
                "logical_path": key,
                "canonical": canonical_edition,
                "mismatched": sorted(set(mismatched) | set(missing)),
                "reason": "content differs" if mismatched else "missing from mirror",
            }
            if missing:
                entry["missing"] = missing
            drift.append(entry)
    return drift


def format_skill_mirror_drift(drift: list[dict[str, object]]) -> str:
    """Render every drifted path, not just the first one.

    Reporting one entry per run meant a repository with twenty drifted files
    could only be repaired twenty runs later, which is how a parity gate stops
    being run at all.
    """
    lines = [f"Skill mirror parity drift ({len(drift)} path(s)):"]
    for item in drift:
        lines.append(
            f"  {item['logical_path']}: canonical {item['canonical']}, "
            f"{item.get('reason', 'content differs')} in "
            f"{', '.join(item['mismatched'])}"
        )
    return "\n".join(lines)


def assert_skill_mirror_parity(repository: Path, canonical_edition: str) -> None:
    drift = skill_mirror_drift(repository, canonical_edition)
    if drift:
        raise RetrievalError(format_skill_mirror_drift(drift))


# ---------------------------------------------------------------------------
# Full mirror parity - executes MIRROR_RULES over this edition, covering
# every mirrored class (skills including non-markdown files, hooks, commands,
# agents, governance documents). The transform implementations deliberately
# match `scripts/build_mirrors.py` at the monorepo root (the regenerator);
# this copy exists so a standalone edition can verify its own mirrors without
# the monorepo. `skill_mirror_drift` above stays the light check on the
# SessionStart/indexing hot path; this one backs `context.py parity` for
# CLI and CI use.
# ---------------------------------------------------------------------------

_MIRROR_IGNORED_NAMES = frozenset({"__pycache__", ".DS_Store"})
_MIRROR_IGNORED_SUFFIXES = (".pyc",)
_MIRROR_FRONTMATTER_DROP = re.compile(r"^(model|invokes|phase):")


def _mirror_split_frontmatter(text: str) -> tuple[Optional[str], str]:
    if text.startswith("---\n"):
        end = text.find("\n---\n", 4)
        if end != -1:
            return text[4 : end + 1], text[end + 5 :]
    return None, text


def _mirror_apply_replacements(text: str, spec: dict[str, Any]) -> str:
    for old, new in spec.get("replacements", []):
        text = text.replace(old, new)
    return text


def _mirror_transform_copy(rel: str, text: str, spec: dict[str, Any]) -> str:
    return _mirror_apply_replacements(text, spec)


def _mirror_transform_cursor_command(rel: str, text: str, spec: dict[str, Any]) -> str:
    frontmatter, body = _mirror_split_frontmatter(text)
    body = _mirror_apply_replacements(body, spec)
    if frontmatter is None:
        return body
    if re.search(r"^name:", frontmatter, re.M):
        # Already Cursor-compatible (name/description); keep it as-is.
        return f"---\n{frontmatter}---\n{body}"
    name = rel.rsplit("/", 1)[-1]
    stem = name[:-3] if name.endswith(".md") else name
    overrides = spec.get("description_overrides", {})
    if name in overrides:
        description = overrides[name]
    else:
        lines = [
            line.strip()
            for line in body.splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        ]
        if not lines:
            raise RetrievalError(f"{rel}: cannot derive a command description")
        description = lines[0]
    if spec.get("quote_description", True):
        description = f'"{description}"'
    return f"---\nname: {stem}\ndescription: {description}\n---\n{body}"


def _mirror_transform_cursor_agent(rel: str, text: str, spec: dict[str, Any]) -> str:
    frontmatter, body = _mirror_split_frontmatter(text)
    body = _mirror_apply_replacements(body, spec)
    if frontmatter is None:
        return body
    kept = [
        line
        for line in frontmatter.splitlines()
        if not _MIRROR_FRONTMATTER_DROP.match(line)
    ]
    joined = "".join(f"{line}\n" for line in kept)
    return f"---\n{joined}---\n{body}"


_MIRROR_TRANSFORMS = {
    "copy": _mirror_transform_copy,
    "cursor-command": _mirror_transform_cursor_command,
    "cursor-agent": _mirror_transform_cursor_agent,
}


def _mirror_is_ignored(rel_parts: tuple[str, ...]) -> bool:
    if any(part in _MIRROR_IGNORED_NAMES for part in rel_parts):
        return True
    return rel_parts[-1].endswith(_MIRROR_IGNORED_SUFFIXES)


def _mirror_is_skipped(rel: str, skips: list[str]) -> bool:
    for entry in skips:
        if entry.endswith("/"):
            if rel.startswith(entry):
                return True
        elif rel == entry:
            return True
    return False


def _mirror_class_skips(
    cls: dict[str, Any], mirror_spec: dict[str, Any], framework: str
) -> list[str]:
    skips = list(cls.get("skip", [])) + list(mirror_spec.get("skip", []))
    if framework:
        skips += cls.get("skip_by_framework", {}).get(framework, [])
    return skips


def _mirror_iter_canonical(canonical: Path, only: Optional[list[str]]):
    if only is not None:
        for rel in only:
            path = canonical / rel
            if path.is_file():
                yield rel, path
        return
    for path in sorted(canonical.rglob("*")):
        if not path.is_file() or path.is_symlink():
            continue
        rel_parts = path.relative_to(canonical).parts
        if _mirror_is_ignored(rel_parts):
            continue
        yield "/".join(rel_parts), path


def full_mirror_drift(repository: Path) -> list[dict[str, str]]:
    """Report every mirrored file whose bytes break the MIRROR_RULES contract.

    A class whose canonical directory is absent is skipped rather than
    reported, and so is an absent mirror directory as a whole: an edition
    copied into a host project may legitimately carry only part of the tree
    (for example only the Claude side), which is the same present-trees-only
    rule the light skills checker applies. Once a mirror directory exists,
    every derived file in it must match, and a mirror file with no canonical
    source is drift too.
    """
    framework = str(load_config(repository).get("framework") or "")
    drift: list[dict[str, str]] = []
    for cls in MIRROR_RULES["classes"]:
        canonical = repository / cls["canonical"]
        if not canonical.is_dir():
            continue
        for mirror_rel, mirror_spec in cls["mirrors"].items():
            mirror = repository / mirror_rel
            if not mirror.is_dir():
                continue
            transform = _MIRROR_TRANSFORMS[mirror_spec.get("transform", "copy")]
            skips = _mirror_class_skips(cls, mirror_spec, framework)
            expected: set[str] = set()
            for rel, path in _mirror_iter_canonical(canonical, cls.get("only")):
                if _mirror_is_skipped(rel, skips):
                    continue
                expected.add(rel)
                raw = path.read_bytes()
                try:
                    text = raw.decode("utf-8")
                except UnicodeDecodeError:
                    if mirror_spec.get("transform", "copy") != "copy" or mirror_spec.get(
                        "replacements"
                    ):
                        raise RetrievalError(
                            f"{cls['canonical']}/{rel}: binary file in a "
                            "transformed mirror class"
                        )
                    derived = raw
                else:
                    derived = transform(rel, text, mirror_spec).encode("utf-8")
                target = mirror / rel
                actual = target.read_bytes() if target.is_file() else None
                if actual == derived:
                    continue
                drift.append(
                    {
                        "path": f"{mirror_rel}/{rel}",
                        "class": cls["name"],
                        "reason": (
                            "missing from mirror"
                            if actual is None
                            else f"differs from canon ({cls['canonical']}/{rel})"
                        ),
                    }
                )
            if cls.get("only") is None:
                # Reverse pass: a file only the mirror carries is drift too.
                for path in sorted(mirror.rglob("*")):
                    if not path.is_file() or path.is_symlink():
                        continue
                    rel_parts = path.relative_to(mirror).parts
                    if _mirror_is_ignored(rel_parts):
                        continue
                    rel = "/".join(rel_parts)
                    if rel in expected or _mirror_is_skipped(rel, skips):
                        continue
                    drift.append(
                        {
                            "path": f"{mirror_rel}/{rel}",
                            "class": cls["name"],
                            "reason": f"no source in {cls['canonical']}",
                        }
                    )
    return drift


def format_full_mirror_drift(drift: list[dict[str, str]]) -> str:
    lines = [f"Mirror parity drift ({len(drift)} file(s)):"]
    for item in drift:
        lines.append(f"  [{item['class']}] {item['path']}: {item['reason']}")
    return "\n".join(lines)


def monorepo_root(repository: Path) -> Optional[Path]:
    """The parent directory, when it carries at least two PHP editions."""
    parent = repository.resolve().parent
    present = [name for name in CROSS_EDITION_SIBLINGS if (parent / name).is_dir()]
    return parent if len(present) >= 2 else None


def _cross_edition_allowed(rel: str) -> Optional[str]:
    for entry, reason in CROSS_EDITION_ALLOWED_DRIFT.items():
        if rel == entry or (entry.endswith("/") and rel.startswith(entry)):
            return reason
    return None


def cross_edition_drift(repository: Path) -> Optional[list[dict[str, object]]]:
    """Compare the cross-edition core byte-for-byte across monorepo siblings.

    Returns ``None`` outside a monorepo (a standalone, copied-out edition has
    nothing to compare against), else a - possibly empty - drift list.
    """
    root = monorepo_root(repository)
    if root is None:
        return None
    editions = [name for name in CROSS_EDITION_SIBLINGS if (root / name).is_dir()]
    digests: dict[str, dict[str, str]] = {}
    for name in editions:
        base = root / name
        for pattern in CROSS_EDITION_CORE_MANIFEST:
            for path in sorted(base.glob(pattern)):
                if not path.is_file() or path.is_symlink():
                    continue
                rel_parts = path.relative_to(base).parts
                if _mirror_is_ignored(rel_parts):
                    continue
                rel = "/".join(rel_parts)
                if _cross_edition_allowed(rel):
                    continue
                digests.setdefault(rel, {})[name] = hashlib.sha256(
                    path.read_bytes()
                ).hexdigest()
    drift: list[dict[str, object]] = []
    for rel, copies in sorted(digests.items()):
        missing = sorted(set(editions) - set(copies))
        if missing:
            drift.append({"path": rel, "reason": "missing", "editions": missing})
        if len(set(copies.values())) > 1:
            drift.append(
                {
                    "path": rel,
                    "reason": "content differs",
                    "editions": sorted(copies),
                }
            )
    return drift


def format_cross_edition_drift(drift: list[dict[str, object]]) -> str:
    lines = [f"Cross-edition core drift ({len(drift)} finding(s)):"]
    for item in drift:
        lines.append(
            f"  {item['path']}: {item['reason']} "
            f"({', '.join(str(name) for name in item['editions'])})"
        )
    return "\n".join(lines)


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
        # A record's title and goal are its declared subject; the body carries
        # progress and evidence that describe the work rather than the topic.
        documents.append(
            (
                relative, "semantic", f"brain-{record['type']}", title,
                " ".join(filter(None, (title, str(record.get("goal") or "")))),
                body,
            )
        )
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
                (
                    relative, "semantic", "brain-handoff",
                    f"Handoff {task['external_id']}",
                    f"Handoff {task['external_id']}", body,
                )
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


def _layer_counts(connection: sqlite3.Connection) -> tuple[int, dict[str, int]]:
    layers = {layer: 0 for layer in ("procedural", "semantic", "episodic")}
    for row in connection.execute(
        "SELECT layer, COUNT(*) AS count FROM documents GROUP BY layer"
    ):
        layers[row[0]] = row[1]
    total = connection.execute("SELECT COUNT(*) FROM documents").fetchone()[0]
    return total, layers


def _drop_indexed_paths(connection: sqlite3.Connection, paths: list[str]) -> None:
    """Drop FTS and metadata rows in chunks that stay inside SQLite's limits."""
    for start in range(0, len(paths), DELETE_CHUNK):
        chunk = paths[start : start + DELETE_CHUNK]
        placeholders = ", ".join("?" for _ in chunk)
        connection.execute(
            f"DELETE FROM documents WHERE path IN ({placeholders})", chunk
        )
        connection.execute(
            f"DELETE FROM document_metadata WHERE path IN ({placeholders})", chunk
        )


def index_documents(
    connection: sqlite3.Connection,
    repository: Path,
    legacy_documents: list[DocumentRow],
    *,
    retained: Optional[SourceState] = None,
    source_state: Optional[SourceState] = None,
    fingerprints: Optional[dict[str, str]] = None,
) -> dict[str, object]:
    """Replace the index, reusing rows the caller proved unchanged.

    ``retained`` holds repository documents whose stat still matches the last
    successful index; their rows survive untouched and ``legacy_documents``
    then carries only the new or changed ones. ``None`` rebuilds everything.

    Project Brain records are always rebuilt: their eligibility depends on
    configuration, lifecycle, and cross-record conflict state rather than on
    the record file alone.
    """
    config = load_config(repository)
    stored = load_index_state(connection)
    if (
        fingerprints is not None
        and stored.get(INDEX_SKILL_KEY) == fingerprints[INDEX_SKILL_KEY]
        and INDEX_PARITY_KEY in stored
    ):
        parity_drift = json.loads(stored[INDEX_PARITY_KEY])
    else:
        # Deliberately the light, skills-only check: indexing sits on the
        # SessionStart/prompt hot path (working-memory-read.sh -> refresh),
        # and it is fingerprint-cached above. The full MIRROR_RULES check
        # (full_mirror_drift) runs from `context.py parity` for CLI/CI.
        parity_drift = skill_mirror_drift(repository, str(config["canonical_edition"]))
    brain_documents, brain_metadata, excluded = _brain_documents(repository, config)
    documents = [*legacy_documents, *brain_documents]
    metadata = [
        _legacy_metadata(path, kind, content)
        for path, _, kind, _, _, content in legacy_documents
    ] + brain_metadata
    state = source_state or {}
    ensure_metadata_tables(connection)
    existing = {
        row[0] for row in connection.execute("SELECT path FROM documents").fetchall()
    }
    keep = set(retained or {})
    missing = keep - existing
    if missing:
        raise RetrievalError(
            f"Index cache is inconsistent for {len(missing)} path(s); "
            "a full refresh is required"
        )
    cached_rows = connection.execute(
        "SELECT COUNT(*) FROM document_source_state"
    ).fetchone()[0]
    if (
        retained is not None
        and not documents
        and existing == keep
        and cached_rows == len(keep)
        and fingerprints is not None
        and INDEX_PARITY_KEY in stored
        and all(stored.get(key) == value for key, value in fingerprints.items())
    ):
        # Nothing observable changed, so a per-request refresh costs reads only.
        total, layers = _layer_counts(connection)
        return {
            "documents": total,
            "removed": 0,
            "reused": len(keep),
            "incremental": True,
            "layers": layers,
            "brain": 0,
            "excluded": excluded,
            "canonical_edition": config["canonical_edition"],
            "parity_drift": parity_drift,
        }
    with connection:
        if not keep:
            # Nothing to reuse, so drop the tables outright rather than paying
            # for a per-path delete of every row.
            connection.execute("DELETE FROM documents")
            connection.execute("DELETE FROM document_metadata")
        else:
            _drop_indexed_paths(connection, sorted(existing - keep))
        connection.execute("DELETE FROM document_source_state")
        connection.executemany(
            "INSERT INTO documents(path, layer, kind, title, summary, content) "
            "VALUES (?, ?, ?, ?, ?, ?)",
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
        connection.executemany(
            "INSERT INTO document_source_state(path, mtime_ns, size) VALUES (?, ?, ?)",
            [(path, value[0], value[1]) for path, value in sorted(state.items())],
        )
        store_index_state(
            connection,
            {
                INDEX_CONFIG_KEY: config_fingerprint(repository),
                INDEX_SKILL_KEY: skill_tree_fingerprint(repository),
                **(fingerprints or {}),
                INDEX_PARITY_KEY: json.dumps(parity_drift, sort_keys=True),
            },
        )
        total, layers = _layer_counts(connection)
    return {
        "documents": total,
        "removed": len(existing - (keep | {item[0] for item in documents})),
        "reused": len(keep),
        "incremental": retained is not None,
        "layers": layers,
        "brain": len(brain_documents),
        "excluded": excluded,
        "canonical_edition": config["canonical_edition"],
        "parity_drift": parity_drift,
    }


def query_tokens(query: str) -> list[str]:
    tokens: list[str] = []
    seen: set[str] = set()
    for token in re.findall(r"\w+", query, flags=re.UNICODE):
        folded = token.casefold()
        if folded in seen:
            continue
        seen.add(folded)
        tokens.append(token)
    if not tokens:
        raise RetrievalError("Search query must contain a word")
    return tokens


def fts_query(query: str) -> str:
    return " OR ".join(f'"{token}"' for token in query_tokens(query))


def _document_frequency(connection: sqlite3.Connection, token: str) -> Optional[int]:
    try:
        return connection.execute(
            "SELECT COUNT(*) FROM documents WHERE documents MATCH ?", (f'"{token}"',)
        ).fetchone()[0]
    except sqlite3.OperationalError:
        return None


def informative_tokens(
    connection: sqlite3.Connection, tokens: list[str]
) -> list[str]:
    """Drop tokens too common in this corpus to carry signal.

    A term matched by most documents contributes nothing but noise: a question
    phrased "how is X specified for Y" otherwise matches every document
    containing "is" or "for". Commonness is measured against the index rather
    than a fixed stopword list, so it adapts to whatever languages and jargon
    the repository actually documents itself in.
    """
    total = connection.execute("SELECT COUNT(*) FROM documents").fetchone()[0]
    if not total:
        return tokens
    frequencies = [
        (token, _document_frequency(connection, token)) for token in tokens
    ]
    matched = [(token, count) for token, count in frequencies if count]
    informative = [
        token for token, count in matched if count <= total * STOPWORD_DOCUMENT_RATIO
    ]
    # Never discard every term: a query built only from common words must still
    # return its best matches rather than nothing at all.
    return informative or [token for token, _ in matched] or tokens


def token_coverage(
    connection: sqlite3.Connection, tokens: list[str]
) -> tuple[dict[str, int], set[str]]:
    """Count distinct query terms per document, and note distinctive matches.

    Counting terms equally punishes exactly the wrong document. A focused note
    that contains only the one term that matters scores 1, while a document
    sharing two unremarkable words scores 2 — so the answer loses to the noise.
    A term rare in this corpus is treated as evidence on its own.
    """
    total = connection.execute("SELECT COUNT(*) FROM documents").fetchone()[0] or 1
    coverage: dict[str, int] = {}
    distinctive: set[str] = set()
    for token in tokens:
        try:
            rows = connection.execute(
                "SELECT path FROM documents WHERE documents MATCH ?", (f'"{token}"',)
            ).fetchall()
        except sqlite3.OperationalError:
            continue
        rare = len(rows) <= total * DISTINCTIVE_DOCUMENT_RATIO
        for row in rows:
            coverage[row[0]] = coverage.get(row[0], 0) + 1
            if rare:
                distinctive.add(row[0])
    return coverage, distinctive


def is_relevant(
    path: str, coverage: dict[str, int], distinctive: set[str], minimum: int
) -> bool:
    """A document qualifies on distinctive evidence or on breadth of match.

    Admitting a distinctive single match lets some noise back in on queries no
    document covers. That is the cheaper error: a spurious result wastes a
    slot, while a hidden one denies the agent an answer the project already
    holds.
    """
    return path in distinctive or coverage.get(path, 0) >= minimum


def required_coverage(tokens: list[str]) -> int:
    """How many distinct terms a document must contain to count as relevant.

    One shared word is not evidence of relevance. Demanding two, once the query
    offers two, is what separates a document about the subject from one that
    merely mentions a word from it.
    """
    return MIN_TOKEN_COVERAGE if len(tokens) >= MIN_TOKEN_COVERAGE else 1


def _estimate_tokens(value: str) -> int:
    return max(1, (len(value) + 3) // 4)


def _candidates(
    connection: sqlite3.Connection, query: str, limit: int = 100
) -> list[dict[str, Any]]:
    ensure_metadata_tables(connection)
    tokens = informative_tokens(connection, query_tokens(query))
    coverage, distinctive = token_coverage(connection, tokens)
    minimum = required_coverage(tokens)
    rows = connection.execute(
        """
        SELECT
            d.path, d.layer, d.kind, d.title,
            snippet(documents, 5, '[', ']', ' … ', 32) AS snippet,
            m.category, m.privacy, m.owner, m.authority, m.lifecycle,
            m.source_hash, m.record_id, m.conflicts,
            m.source_fingerprints,
            bm25(documents, ?, ?, ?, ?, ?, ?) AS score
        FROM documents AS d
        JOIN document_metadata AS m ON m.path = d.path
        WHERE documents MATCH ?
        ORDER BY score, d.path
        LIMIT ?
        """,
        (*BM25_WEIGHTS, " OR ".join(f'"{token}"' for token in tokens), limit),
    ).fetchall()
    result = []
    for row in rows:
        if not is_relevant(row["path"], coverage, distinctive, minimum):
            continue
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
        content: Optional[str] = None
        if reason is None:
            path = repository / candidate["path"]
            try:
                content = path.read_text(encoding="utf-8") if path.is_file() else None
            except OSError:
                content = None
            if content is None or _content_hash(content) != candidate["source_hash"]:
                reason = "stale"
        if reason is None and candidate["kind"] == "codebase" and content is not None:
            # A map that no longer describes the code is worse than no map:
            # an agent acts on it instead of reading the source.
            drift = codebase_map_drift(repository, content)
            limit = config.get("codebase_map_max_drift", CODEBASE_MAP_MAX_DRIFT)
            if drift is None:
                reason = "map-unverifiable"
            elif drift > limit:
                reason = "map-drift"
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


def _prune_local_manifests(directory: Path) -> None:
    """Bound ignored local manifests; the governed store has its own policy."""
    manifests = [path for path in directory.glob("*.json") if path.is_file()]
    if len(manifests) <= LOCAL_MANIFEST_RETENTION:
        return
    manifests.sort(key=lambda path: (path.stat().st_mtime_ns, path.name))
    for path in manifests[: len(manifests) - LOCAL_MANIFEST_RETENTION]:
        try:
            path.unlink()
        except OSError:
            continue


def retrieve(
    connection: sqlite3.Connection,
    repository: Path,
    query: str,
    task_identifier: str,
    *,
    limit: int,
    provider: Optional[str] = None,
    manifest_scope: str = "governed",
) -> dict[str, Any]:
    if limit < 1:
        raise RetrievalError("--limit must be a positive integer")
    if manifest_scope not in MANIFEST_SCOPES:
        raise RetrievalError(
            f"Manifest scope must be one of {', '.join(MANIFEST_SCOPES)}"
        )
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
    if manifest_scope == "governed":
        manifest_directory = brain_root(repository) / "control" / "retrieval-manifests"
    else:
        manifest_directory = repository / "memory-bank" / "local" / "retrieval-manifests"
    manifest_path = manifest_directory / f"{manifest_id}.json"
    validate_schema_file(
        repository, "retrieval-manifest.schema.json", manifest
    )
    if manifest_scope == "governed":
        with mutation_lock(repository):
            atomic_json(manifest_path, manifest)
    else:
        # Ignored local provenance for automated retrieval: the same validated
        # record, kept out of shared history and out of the runtime lock.
        atomic_json(manifest_path, manifest)
        _prune_local_manifests(manifest_directory)
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
            "task_id": task["external_id"], "goal": task["goal"],
            # Manual progress first, the automatic checkpoint as a labelled
            # supplement; a task without a checkpoint renders as it always did.
            "progress": render_current_state(
                task["progress"], task.get("auto_checkpoint")
            ),
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
        "manifest_scope": manifest_scope,
    }
