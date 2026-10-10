#!/usr/bin/env python3
"""Governed FTS5 indexing, budgeting, and retrieval manifests."""

from __future__ import annotations

import hashlib
from bisect import bisect_left, bisect_right
import json
import math
import re
import sqlite3
import stat
import subprocess
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Iterable, Optional

import workspace_roots
from brain_runtime import (
    BrainError,
    LIFECYCLES,
    atomic_json,
    brain_root,
    find_task,
    fingerprint,
    handoff_path,
    iter_records,
    load_config,
    mutation_lock,
    new_uuid,
    parse_markdown_record,
    record_attestation,
    record_is_eligible,
    render_current_state,
    source_changes,
    utc_now,
    validate_handoff,
    validate_record,
    validate_schema_file,
)
from automatic_query import source_path_problem
from validate import secret_policy_fingerprint


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
# What a candidate the query ranked quotes of its body (_quote_candidates).
# The governed capsule keeps 320 characters of every snippet
# (enforce_governed_capsule_contract), so the window is chosen at that size
# instead of being cut to it mid-sentence.
SNIPPET_WINDOW_CHARS = 320
# What a document whose cited file changed since verification keeps of its
# relevance: it still ranks, below fresh knowledge of equal fit.
SOURCE_CHANGED_WEIGHT = 0.5
# One: see `procedural_ranked` in `retrieve`.
CAPSULE_PROCEDURAL_LIMIT = 1
CAPSULE_SEMANTIC_LIMIT = 3
CAPSULE_EPISODIC_LIMIT = 1
SKILL_EDITIONS = (".agents", ".claude", ".cursor", ".codex")
# The one file of a skill a host lists and invokes. Everything else indexed from
# a skills tree - references/, agents/, rules/, an AGENTS.md inside a skill, a
# note at the tree's root such as `SKILL FLOW.md` - is material a SKILL.md sends
# the agent to. It stays indexed for `search`, but never holds the capsule's
# procedural slot: on 121 graded prompts such files were useful 1 time in 185
# judgments against 71 in 627 for SKILL.md, and on the 19 turns one held the
# slot the next skill down was useful on none.
SKILL_ENTRY_FILENAME = "SKILL.md"


def procedural_slot_eligible(kind: str, path: str) -> bool:
    """Whether a procedural document may hold the capsule's procedural slot.

    Root policy (AGENTS.md, CLAUDE.md) may; a skill document only if it is a
    skill's entry file. Keys are POSIX on every platform and an attached
    tooling key is absolute, so the basename is the last segment.
    """
    if kind != "skill":
        return True
    directory, _, name = path.rpartition("/")
    # A SKILL.md lying directly in a skills tree belongs to no skill.
    return name == SKILL_ENTRY_FILENAME and not any(
        directory == f"{tool}/skills" or directory.endswith(f"/{tool}/skills")
        for tool in SKILL_EDITIONS
    )
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

# Working-memory delivery: Claude and Codex receive the Task Capsule at
# prompt time through working-memory-read.sh (UserPromptSubmit); Cursor has
# no equivalent event, so its mirrors of the Stop and sessionStart hooks
# render the freshest capsule into .cursor/rules/working-memory.mdc - an
# alwaysApply rule Cursor attaches to every prompt. The rendered file is one
# turn stale by design, says so in its header, and lives in ignored local
# state (the edition .gitignore lists it). The canonical hooks carry the
# short marker comments below; the Cursor mirror swaps in the render steps.
# The render is silent on stdout, degrades to a no-op on any failure, and
# replaces the previous rule only when a fresh render succeeds.
#
# The rule is the one surface in this product that is re-sent on every prompt,
# so its content must vary only when the context genuinely varies. Two former
# sources of gratuitous per-turn variation are therefore excluded by design:
#   - no render timestamp. The header already states the rule is as of the end
#     of the previous turn; a clock reading added nothing and changed every
#     turn.
#   - the capsule is rendered, not serialized (`hook-context` without --json).
#     The JSON form embeds `manifest`, a fresh UUID path per call, and carries
#     the same documents in four parallel views (categories / procedural /
#     semantic / selected). Both were re-sent every turn. The rendered form is
#     what Claude and Codex already receive, so all three clients now agree.
# Anything added here is paid once per turn for the life of the session -
# weigh it against that, not against a single prompt.
_WM_DELIVERY_STOP = r'''# Capsule delivery: this client receives the Task Capsule at prompt time
# through working-memory-read.sh, so the turn checkpoint above is all that
# runs here.
'''
_WM_DELIVERY_STOP_CURSOR = r'''# Capsule delivery: Cursor's prompt-time event (beforeSubmitPrompt) cannot
# add context to a prompt, so Cursor reads the Task Capsule from an
# alwaysApply rule, which it sends with every request. working-memory-read.sh
# renders the rule for each prompt; here, after the turn checkpoint, the
# branch's capsule replaces any rule that hook did not render for this task
# (an install without it, a prompt without a task, a switched branch). The
# file is ignored local state, replaced only when a fresh render succeeds.
# Attached, .cursor/rules is the shared clone's own rule folder, which every
# project using the clone reads; the attaching launcher delivers the capsule
# in the prompt instead. A host that put the capsule into the prompt itself
# (the Harness) gets no second, branch-built one.
[ "$STATE_DIR" = "$ROOT_DIR" ] || exit 0
[ "${CONTEXT_CAPSULE_DELIVERED:-}" = "1" ] && exit 0
RULES_DIR="$ROOT_DIR/.cursor/rules"
RULE_FILE="$RULES_DIR/working-memory.mdc"
# The prompt hook's rule for this task holds what was retrieved for the
# conversation's latest prompt. Rebuilt here from the task alone it would lose
# that, and when Cursor reads its rules before the prompt hook has run, it is
# what the next request carries.
grep -qxF "Session context retrieved for a recent prompt (task: $TASK_ID)." "$RULE_FILE" 2>/dev/null && exit 0
CAPSULE_STATUS=1
if command -v timeout > /dev/null 2>&1; then
  CAPSULE=$(timeout "$BUDGET_SECONDS" python3 "$CONTEXT_CLI" hook-context \
    --host cursor --task-id "$TASK_ID" 2>/dev/null)
  CAPSULE_STATUS=$?
else
  CAPSULE=$(python3 "$CONTEXT_CLI" hook-context \
    --host cursor --task-id "$TASK_ID" 2>/dev/null)
  CAPSULE_STATUS=$?
fi
# The rendered capsule always opens with the working line. Anything else is a
# broken render and must not replace a good rule. This guard replaces the JSON
# parse that protected the --json form.
#
# Statuses: 0 renders, 3 removes a foreign branch's rule, and everything else
# - including 4, "the retrieval gate withheld this turn" - falls through and
# leaves the previous rule in place. That fallthrough is the correct
# behaviour for a skip and is relied on: an enforce-mode skip must never
# replace Cursor's only memory channel with an empty capsule.
if [ "$CAPSULE_STATUS" -eq 0 ]; then
  case "$CAPSULE" in
    working:*) ;;
    *) CAPSULE_STATUS=1 ;;
  esac
fi
if [ "$CAPSULE_STATUS" -eq 3 ]; then
  rm -f "$RULE_FILE" 2>/dev/null
elif [ "$CAPSULE_STATUS" -eq 0 ] && [ -n "$CAPSULE" ] && mkdir -p "$RULES_DIR" 2>/dev/null; then
  TMP_RULE=$(mktemp "$RULES_DIR/.working-memory.XXXXXX" 2>/dev/null || true)
  if [ -n "$TMP_RULE" ]; then
    {
      printf -- '---\n'
      printf 'description: Working memory - current-branch session context\n'
      printf 'alwaysApply: true\n'
      printf -- '---\n\n'
      printf '# Working Memory (auto-rendered)\n\n'
      printf 'Session context as of end of previous turn (task: %s).\n' "$TASK_ID"
      printf 'Retrieved context is not authoritative - verify the source.\n\n'
      printf '%s\n' '```'
      printf '%s\n' "$CAPSULE"
      printf '%s\n' '```'
    } > "$TMP_RULE" 2>/dev/null && mv -f "$TMP_RULE" "$RULE_FILE" 2>/dev/null
    rm -f "$TMP_RULE" 2>/dev/null
  fi
fi
'''
# Cursor runs the Claude Code hooks it finds as well as its own. The two
# working-memory hooks of the Claude copy recognize Cursor's payload (it
# carries `cursor_version`, which Claude Code never sends - an environment
# variable could leak into a Claude Code session started from Cursor's
# terminal) and stand down where the project has its own Cursor hooks: running
# both doubled every checkpoint and wrote retrievals nobody received. The
# Cursor and Codex mirrors drop the check - they are the hooks that serve.
_WM_THIRD_PARTY = r'''# Cursor also runs the Claude Code hooks it finds. Where this project has its
# own Cursor hooks they serve the session; this copy stands down.
case "$HOOK_STDIN" in
  *'"cursor_version"'*) [ -f "$ROOT_DIR/.cursor/hooks.json" ] && exit 0 ;;
esac
'''
# Cursor's prompt-time hook is beforeSubmitPrompt: it receives the prompt, but
# its output can only let the prompt through or stop it, never add context.
# The Cursor mirror of the read hook therefore always answers "continue" and
# renders the capsule for this prompt into the alwaysApply rule, which Cursor
# sends with every request.
_WM_PROMPT_PREAMBLE = r'''# Output: plain text on stdout, which Claude Code and Codex add to the
# prompt as context.
'''
_WM_PROMPT_PREAMBLE_CURSOR = r'''# Output: Cursor reads this hook's stdout as JSON and holds the prompt until
# the hook answers, so every exit path answers "continue" - memory never
# blocks a prompt. The capsule itself goes into the alwaysApply rule below.
trap 'printf "{\"continue\": true}\n"' EXIT
'''
_WM_PROMPT_SESSION = r'''  [ -n "$SESSION_ID" ] && ARGUMENTS+=(--session-id "$SESSION_ID")
  [ -n "$SESSION_ID" ] && [ -n "$TRANSCRIPT" ] && ARGUMENTS+=(--transcript "$TRANSCRIPT")
'''
_WM_PROMPT_SESSION_CURSOR = r'''  # The rule is re-sent whole with every request, so nothing may be left out
  # of it as "handed earlier": no --session-id. JSON, for capsule_text.
  ARGUMENTS+=(--json)
'''
_WM_DELIVERY_PROMPT = r'''[ -n "$REPORT" ] || exit 0

# The capsule names itself: its memory section says the text is reference
# data to check against the cited file, so no banner precedes it.
echo "$REPORT"
'''
_WM_DELIVERY_PROMPT_CURSOR = r'''# Capsule delivery: the rule is rendered from this prompt before the request
# leaves - in the same turn when Cursor reads its rules after this hook, on
# the next prompt otherwise. Cursor puts rules at the start of the model's
# context, so every change costs the conversation its cached prefix: the rule
# is replaced only when this prompt retrieved an item it does not hold yet.
# Attached, .cursor/rules is the shared clone's own folder; the launcher
# delivers the capsule in the prompt instead.
[ "$STATE_DIR" = "$ROOT_DIR" ] || exit 0
[ -n "$REPORT" ] || exit 0
RULES_DIR="$ROOT_DIR/.cursor/rules"
RULE_FILE="$RULES_DIR/working-memory.mdc"
RULE_HEADER="Session context retrieved for a recent prompt (task: $TASK_ID)."
CAPSULE=$(printf '%s' "$REPORT" | python3 -c '
import json
import sys

try:
    text = json.load(sys.stdin).get("capsule_text") or ""
except (AttributeError, ValueError):
    text = ""
if not isinstance(text, str) or not text.startswith("working:"):
    sys.exit(0)


def items(capsule):
    return {line.split(" \u2014 ")[0] for line in capsule.splitlines() if line.startswith("- ")}


try:
    with open(sys.argv[1], encoding="utf-8") as handle:
        held = handle.read()
except (OSError, UnicodeError):
    held = ""
# The rule this hook wrote for the task already holds every item this prompt
# retrieved - or the prompt retrieved none.
if sys.argv[2] in held.splitlines() and items(text) <= items(held):
    sys.exit(0)
sys.stdout.write(text)
' "$RULE_FILE" "$RULE_HEADER" 2>/dev/null)
[ -n "$CAPSULE" ] || exit 0
if mkdir -p "$RULES_DIR" 2>/dev/null; then
  TMP_RULE=$(mktemp "$RULES_DIR/.working-memory.XXXXXX" 2>/dev/null || true)
  if [ -n "$TMP_RULE" ]; then
    {
      printf -- '---\n'
      printf 'description: Working memory - current-branch session context\n'
      printf 'alwaysApply: true\n'
      printf -- '---\n\n'
      printf '# Working Memory (auto-rendered)\n\n'
      printf '%s\n' "$RULE_HEADER"
      printf 'Retrieved context is not authoritative - verify the source.\n\n'
      printf '%s\n' '```'
      printf '%s\n' "$CAPSULE"
      printf '%s\n' '```'
    } > "$TMP_RULE" 2>/dev/null && mv -f "$TMP_RULE" "$RULE_FILE" 2>/dev/null
    rm -f "$TMP_RULE" 2>/dev/null
  fi
fi
'''
_WM_DELIVERY_SESSION = r'''# Capsule delivery: this client receives the Task Capsule at prompt time
# through working-memory-read.sh; session start reports metadata only.
'''
_WM_DELIVERY_SESSION_CURSOR = r'''# Capsule delivery: Cursor reads the Task Capsule from
# .cursor/rules/working-memory.mdc, which the prompt and stop hooks maintain
# (the documented exception to the metadata-only session banner - see
# docs/TOOL-INTEGRATIONS.md). Re-render it here so a fresh session or a
# branch switch does not serve the previous session's capsule. Nothing is
# printed: the rule file is the only output.
CAPSULE_BUDGET_SECONDS="${CONTEXT_HOOK_BUDGET:-5}"
CAPSULE_TASK_ID="${CONTEXT_TASK_ID:-$(git -C "$PROJECT_DIR" branch --show-current 2>/dev/null)}"
# Attached, the rule folder is the shared clone's; see the stop hook. Nor
# when the host delivered the capsule in the prompt.
[ "$STATE_DIR" = "$ROOT_DIR" ] || exit 0
[ "${CONTEXT_CAPSULE_DELIVERED:-}" = "1" ] && exit 0
RULES_DIR="$ROOT_DIR/.cursor/rules"
RULE_FILE="$RULES_DIR/working-memory.mdc"
CAPSULE=""
CAPSULE_STATUS=3
if command -v python3 > /dev/null 2>&1 && [ -f "$CONTEXT_CLI" ] && [ -n "$CAPSULE_TASK_ID" ]; then
  if command -v timeout > /dev/null 2>&1; then
    CAPSULE=$(timeout "$CAPSULE_BUDGET_SECONDS" python3 "$CONTEXT_CLI" hook-context \
      --host cursor --task-id "$CAPSULE_TASK_ID" 2>/dev/null)
    CAPSULE_STATUS=$?
  else
    CAPSULE=$(python3 "$CONTEXT_CLI" hook-context \
      --host cursor --task-id "$CAPSULE_TASK_ID" 2>/dev/null)
    CAPSULE_STATUS=$?
  fi
fi
# See the stop hook: the working line is the render marker that replaces the
# JSON parse.
#
# Statuses: 0 renders, 3 removes a foreign branch's rule, and everything else
# - including 4, "the retrieval gate withheld this turn" - falls through and
# leaves the previous rule in place. That fallthrough is the correct
# behaviour for a skip and is relied on: an enforce-mode skip must never
# replace Cursor's only memory channel with an empty capsule.
if [ "$CAPSULE_STATUS" -eq 0 ]; then
  case "$CAPSULE" in
    working:*) ;;
    *) CAPSULE_STATUS=1 ;;
  esac
fi
if [ "$CAPSULE_STATUS" -eq 3 ]; then
  rm -f "$RULE_FILE" 2>/dev/null
elif [ "$CAPSULE_STATUS" -eq 0 ] && [ -n "$CAPSULE" ] && mkdir -p "$RULES_DIR" 2>/dev/null; then
  TMP_RULE=$(mktemp "$RULES_DIR/.working-memory.XXXXXX" 2>/dev/null || true)
  if [ -n "$TMP_RULE" ]; then
    {
      printf -- '---\n'
      printf 'description: Working memory - current-branch session context\n'
      printf 'alwaysApply: true\n'
      printf -- '---\n\n'
      printf '# Working Memory (auto-rendered)\n\n'
      printf 'Session context as of end of previous turn (task: %s).\n' \
        "$CAPSULE_TASK_ID"
      printf 'Retrieved context is not authoritative - verify the source.\n\n'
      printf '%s\n' '```'
      printf '%s\n' "$CAPSULE"
      printf '%s\n' '```'
    } > "$TMP_RULE" 2>/dev/null && mv -f "$TMP_RULE" "$RULE_FILE" 2>/dev/null
    rm -f "$TMP_RULE" 2>/dev/null
  fi
fi
'''

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
                        # Cursor's read path: the Stop and sessionStart
                        # mirrors render the capsule into the alwaysApply
                        # rule .cursor/rules/working-memory.mdc (see the
                        # _WM_DELIVERY_* constants above).
                        [_WM_DELIVERY_STOP, _WM_DELIVERY_STOP_CURSOR],
                        [_WM_DELIVERY_SESSION, _WM_DELIVERY_SESSION_CURSOR],
                        # The read hook runs on beforeSubmitPrompt and renders
                        # this prompt's capsule into the rule (see the
                        # _WM_PROMPT_* constants above).
                        [
                            "# Hook type: UserPromptSubmit",
                            "# Cursor hook event: beforeSubmitPrompt",
                        ],
                        ["--host claude", "--host cursor"],
                        [_WM_PROMPT_PREAMBLE, _WM_PROMPT_PREAMBLE_CURSOR],
                        [_WM_PROMPT_SESSION, _WM_PROMPT_SESSION_CURSOR],
                        [_WM_DELIVERY_PROMPT, _WM_DELIVERY_PROMPT_CURSOR],
                        [_WM_THIRD_PARTY, ""],
                    ],
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
                        ["--host claude", "--host codex"],
                        [" .claude; do", " .agents .codex; do"],
                        [
                            "- /debugger to investigate the root cause",
                            "- systematic-debugger to investigate the root cause",
                        ],
                        [
                            "consider using /debugger.",
                            "consider using systematic-debugger.",
                        ],
                        [_WM_THIRD_PARTY, ""],
                    ],
                },
            },
            "skip": [
                # Each tool documents its own registration model (settings.json
                # vs hooks.json vs config.toml), so every hooks README is
                # mirror-owned.
                "README.md",
                # Tool-owned: each host exposes a different subagent-gate
                # contract (Claude PreToolUse exit codes, Cursor subagentStart
                # permission JSON, Codex spawn_agent deny) and reads a
                # different roster source, so the three copies are separate
                # generations by design.
                "subagent-gate.sh",
            ],
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
# (Laravel, Symfony, PHP Core, WordPress) and MUST stay that way: a fix that lands in
# one edition and not the others silently forks the engine. `context.py
# parity --cross-edition` walks this manifest against every sibling edition
# it can find next to this one; a standalone (copied-out) edition has no
# siblings and skips the check.
# ---------------------------------------------------------------------------

CROSS_EDITION_PATHS = {
    "Laravel": Path("Laravel"),
    "Symfony": Path("Symfony"),
    "PHP Core": Path("PHP Core"),
    "WordPress": Path("Cms/wordpress"),
}

# Glob patterns, relative to an edition root, of files that must be
# byte-identical in every sibling edition. Composition verified by direct
# md5 comparison of the editions before the manifest was frozen.
CROSS_EDITION_CORE_MANIFEST = (
    "memory-bank/scripts/*.py",
    "memory-bank/MCP.md",
    "memory-bank/tests/*.py",
    # The chunk template is the shape every durable memory is written to, and
    # nothing framework-specific appears in it. It sat outside every gate
    # until two editions were found still handing engineers the retired
    # MEM-0000 identifier scheme that the same bank's validator had moved on
    # from.
    "memory-bank/templates/*",
    "project-brain/scripts/*.py",
    "project-brain/schemas/**/*",
    "project-brain/PROTOCOL.md",
    "project-brain/tests/*.py",
    "project-brain/tests/fixtures/memory-probes.json",
    # The hooks are the only automatic entry into the memory core, so a hook
    # that forks silently forks the engine as surely as a module would: an
    # edition whose UserPromptSubmit hook truncates the prompt asks the core a
    # different question than its siblings and gets a different capsule back.
    # Two of them speak the framework and are exempted below; the rest are
    # engine and are held byte-identical here.
    ".claude/hooks/*.sh",
)

# Legitimate cross-edition differences, each with its justification. Entries
# ending in "/" match a whole subtree. The hook entries below are the only
# ones that intersect the manifest above, and deliberately so: they carve the
# two framework-shaped files out of an otherwise byte-identical hook surface.
# The rest are recorded so a future manifest extension cannot accidentally
# turn known-deliberate divergence into reported drift.
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
    # Two hooks are framework surface rather than engine: bash-validator.sh
    # blocks the destructive commands of this framework's CLI (artisan
    # migrate:fresh against doctrine:schema:drop), and local-context.sh
    # detects this framework's stack at session start. Every other hook -
    # working-memory-read/write, subagent-gate, subagent-dispatch,
    # loop-detection, file-naming-validator - is engine and MUST match.
    ".claude/hooks/bash-validator.sh": "framework-specific destructive-command rules",
    ".claude/hooks/local-context.sh": "framework-specific session-start detection",
}

MANIFEST_SCOPES = ("governed", "local")
# Version 3 adds the host and retrieval entry point, version 4 the counters
# of automatic source-linked expansion. Older manifests stay valid: the
# validator keys its strict key set off the declared version.
MANIFEST_SCHEMA_VERSION = 4
# Where the query that produced a retrieval came from. `prompt` is the user's
# own request, `task` the goal and state of the active task, `task-id` a bare
# identifier or branch name with no task text behind it, and `explicit` an
# operator-supplied query on the CLI.
QUERY_SOURCES = ("prompt", "task", "task-id", "explicit")
RETRIEVAL_HOSTS = ("cli", "claude", "codex", "cursor")
RETRIEVAL_ENTRY_POINTS = ("context", "retrieve", "refresh", "hook-context")
# Instruction files a host loads into the model's context by itself, before
# any hook runs. A capsule slot pointing at one asks the agent to read what it
# already has: on two real installations CLAUDE.md took a procedural slot on
# 88 of 114 and 150 of 158 Claude Code turns. Cursor is absent on purpose -
# what it loads unprompted is its own `.cursor/rules`, which is not indexed.
HOST_LOADED_INSTRUCTIONS = {
    "claude": ("CLAUDE.md", ".claude/CLAUDE.md", "CLAUDE.local.md"),
    "codex": ("AGENTS.md",),
}
# Claude Code (v2.1.277+) reads AGENTS.md itself only while none of these
# exists; any one of them makes it read the CLAUDE.md files instead, and
# AGENTS.md then loads only through an `@` import - which is why every
# edition ships `.claude/CLAUDE.md` importing `@../AGENTS.md`.
CLAUDE_INSTRUCTION_FILES = ("CLAUDE.md", ".claude/CLAUDE.md", "CLAUDE.local.md")
# Claude Code also loads what CLAUDE.md imports with `@path`, recursively and
# at most five hops deep; an import inside a code span or block is not one.
CLAUDE_IMPORT_DEPTH = 5
CLAUDE_IMPORT_PATTERN = re.compile(r"(?<![\w@])@((?:\.{1,2}/)?[\w-][\w./-]*)")
CODE_FENCE_PATTERN = re.compile(r"^\s*(```|~~~)")
CODE_SPAN_PATTERN = re.compile(r"`+[^`]*`+")
# Whether a turn retrieves at all. `off` never decides; `shadow` decides and
# records the decision but always retrieves; `enforce` acts on it. The default
# is `shadow` on purpose: the project rejected an embedding similarity floor on
# measured evidence, and the same standard applies here - the mechanism is
# built, observed, and only then switched on, which is H3-05's job and not
# this one's.
RETRIEVAL_GATE_MODES = ("off", "shadow", "enforce")
RETRIEVAL_GATE_DEFAULT = "shadow"
# Every retrieval expands declared source links, one hop from at most two
# strong, eligible semantic matches. There is no graph activation setting.
GRAPH_ANCHOR_LIMIT = 2
GRAPH_ROW_LIMIT = 32
GRAPH_SOURCE_BYTE_LIMIT = 8 * 1024 * 1024
GRAPH_CANDIDATE_LIMIT = CAPSULE_SEMANTIC_LIMIT
# One index_state row holding a JSON map of task UUID -> last retrieval, not
# one row per task: nothing prunes index_state (it is upsert-only), so a key
# per task would grow for the life of the database.
LAST_RETRIEVAL_KEY = "last-retrieval"
# What each conversation was already handed, so a later turn of the same
# conversation does not hand it again: 48% of delivered items were repeats
# within the hour on real installations, every one of them paid for again.
# An item may come back after this many turns - a long conversation that was
# compacted has lost it by then - and the map keeps the newest sessions only.
SESSION_DELIVERIES_KEY = "session-deliveries"
SESSION_NOVELTY_TURNS = 4
SESSION_DELIVERIES_RETENTION = 50
SESSION_ID_PATTERN = re.compile(r"^[A-Za-z0-9._:-]{1,128}$")
LAST_RETRIEVAL_RETENTION = 200
REFRESH_HEALTH_RETENTION = 500
STOPWORD_DOCUMENT_RATIO = 0.5
# Words that never count as evidence a document is relevant, in the two
# languages the projects' prompts arrive in: function words and the words of
# conversation ("thanks", "ok", "please", "need"). Rarity alone could not
# tell them apart - a corpus that rarely says "thanks" made "thanks, looks
# good" retrieve a review skill, a grader and the changelog. They still rank;
# they just never admit a document, and a turn made of nothing else retrieves
# nothing. Measured on the retrieval bench: documents injected on turns with
# no relevant document fell from 3.25 to 2.70 per turn, recall unchanged.
EVIDENCE_STOPWORDS = frozenset(
    """
    a about above after again against all am an and any are aren as at be
    because been before being below between both but by can cannot could
    couldn did didn do does doesn doing don down during each few for from
    further had hadn has hasn have haven having he her here hers herself him
    himself his how i if in into is isn it its itself just let me more most
    mustn my myself no nor not now of off on once only or other ought our
    ours ourselves out over own same shan she should shouldn so some such than
    that the their theirs them themselves then there these they this those
    through to too under until up very was wasn we were weren what when where
    which while who whom why will with won would wouldn you your yours
    yourself yourselves s t ll re ve d m o y also anything everything
    something someone anyone please thanks thank ok okay yes yeah sure get got
    make made want need like one two way thing things still again really
    и в во не что он на я с со как а то все она так его но да ты к у же вы
    за бы по только ее мне было вот от меня еще нет о из ему теперь когда
    даже ну вдруг ли если уже или ни быть был него до вас нибудь опять уж
    вам ведь там потом себя ничего ей может они тут где есть надо ней для мы
    тебя их чем была сам чтоб без будто чего раз тоже себе под будет ж тогда
    кто этот того потому этого какой совсем ним здесь этом один почти мой тем
    чтобы нее сейчас были куда зачем всех никогда можно при наконец два об
    другой хоть после над больше тот через эти нас про всего них какая много
    разве три эту моя впрочем хорошо свою этой перед иногда лучше чуть том
    нельзя такой им более всегда конечно всю между это эта мои давай
    пожалуйста спасибо ок сделай сделать нужно какие
    good great nice fine cool perfect awesome excellent done look looks
    looking seems seem works working continue proceed go ahead lets right
    correct thx cheers hi hello hey bye agreed approve approved lgtm
    отлично супер класс готово продолжай продолжи продолжить дальше норм
    нормально верно понятно ясно ага угу привет пока согласен
    """.split()
)
# A candidate whose adjusted score is below this share of the best score in
# its own layer is left out: a long tail of partial matches was most of what
# off-topic turns delivered (0.55 fewer documents per such turn on the bench,
# recall unchanged). Per layer, because project records score lower than the
# skills they compete with and must not be cut by a skill's score.
RELATIVE_SCORE_FLOOR = 0.3
MIN_TOKEN_COVERAGE = 2
DISTINCTIVE_DOCUMENT_RATIO = 0.1
MAPPED_COMMIT_PATTERN = re.compile(r"^mapped_commit:\s*([0-9a-fA-F]{7,40})\s*$", re.M)
MAPPED_SCOPE_PATTERN = re.compile(r"^mapped_scope:\s*(\S+)\s*$", re.M)
CODEBASE_MAP_MAX_DRIFT = 25
LOCAL_MANIFEST_RETENTION = 200
DELETE_CHUNK = 500
INDEX_CONFIG_KEY = "config-fingerprint"
INDEX_SKILL_KEY = "skill-tree-fingerprint"
INDEX_SECRET_POLICY_KEY = "secret-policy-fingerprint"
INDEX_PARITY_KEY = "skill-parity-drift"
# A fresh value is stamped whenever index_documents rewrites the tables, so
# anything derived from index content (the token-frequency cache below) can
# tell at a glance whether it still describes the current index.
INDEX_GENERATION_KEY = "index-generation"
TOKEN_FREQUENCY_KEY = "token-frequency-cache"
TOKEN_FREQUENCY_LIMIT = 4096

# Porter wraps unicode61 so "rounding" matches "round" and "review" matches
# "reviewer". Without stemming the correct skill is simply missed: a security
# question did not retrieve the security skill because its title says
# "Reviewer". Non-English tokens pass through the stemmer unchanged. The
# excerpt asks the same tokenizer what a word is (term_forms).
TOKENIZER = "porter unicode61"
# Words term_forms keeps the tokenizer's answer for, per process.
TERM_FORM_CACHE_LIMIT = 4096

# Column weights for bm25(): path, layer, kind, title, summary, content.
# What a document declares itself to be about outranks what its body mentions.
BM25_WEIGHTS = (1.0, 1.0, 1.0, 2.0, 8.0, 1.0)

# Ranking multipliers applied over BM25 relevance in governed retrieval.
# BM25 knows lexical fit only; provenance quality and age are metadata the
# index already stores, so a candidate's relevance is scaled — never zeroed —
# by how trustworthy and how current its record claims to be.
RECENCY_HALF_LIFE_DAYS = 30.0
# Floors keep the multipliers from ever hiding a lexical match outright: an
# old observed record still ranks, it just yields to a fresh verified one.
RECENCY_WEIGHT_FLOOR = 0.5
CONFIDENCE_WEIGHT_FLOOR = 0.7
AUTHORITY_WEIGHTS = {"verified": 1.0, "observed": 0.85}
AUTHORITY_WEIGHT_DEFAULT = 0.85

DocumentRow = tuple[str, str, str, str, str, str]
# path -> (mtime_ns, size, eligible_until). The stat pair answers "has the
# file changed"; the third element answers "is it still in date", which for
# durable memory is a separate question with a separate answer - a chunk
# retires on a calendar boundary without anyone touching the file. ``None``
# means the document has no calendar boundary (everything but memory) or the
# row predates the column.
SourceState = dict[str, tuple[int, int, Optional[str]]]


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
            source_fingerprints TEXT NOT NULL DEFAULT '[]',
            updated_at TEXT NOT NULL DEFAULT '',
            confidence REAL NOT NULL DEFAULT 1.0,
            attestation TEXT NOT NULL DEFAULT ''
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
    # Ranking provenance added later than the table: an old row simply has no
    # recorded freshness ('' — never decayed) and full confidence, which is
    # exactly what it asserted before the columns existed.
    if "updated_at" not in metadata_columns:
        connection.execute(
            "ALTER TABLE document_metadata "
            "ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''"
        )
    if "confidence" not in metadata_columns:
        connection.execute(
            "ALTER TABLE document_metadata "
            "ADD COLUMN confidence REAL NOT NULL DEFAULT 1.0"
        )
    # Who checked a verified claim ('agent' or 'person'; '' when nobody
    # said). `connect` forces one full re-read when the column is new, so a
    # retained row cannot keep claiming it was never attested.
    if "attestation" not in metadata_columns:
        connection.execute(
            "ALTER TABLE document_metadata "
            "ADD COLUMN attestation TEXT NOT NULL DEFAULT ''"
        )
    # Reverse conflict checks visit only the rows that declare a conflict,
    # without JSON extensions or a full scan of ordinary source metadata.
    connection.execute(
        "CREATE INDEX IF NOT EXISTS document_metadata_conflicts "
        "ON document_metadata(conflicts) WHERE conflicts != '[]'"
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
            size INTEGER NOT NULL,
            eligible_until TEXT
        )
        """
    )
    # CREATE TABLE IF NOT EXISTS leaves an existing three-column table alone,
    # so a database written before the calendar boundary existed needs the
    # column added or every insert below fails. An old row reads NULL, which
    # `_cache_entry_is_current` treats as "cannot express expiry, re-validate
    # once" rather than as "no boundary".
    source_state_columns = {
        row["name"]
        for row in connection.execute("PRAGMA table_info(document_source_state)")
    }
    if "eligible_until" not in source_state_columns:
        connection.execute(
            "ALTER TABLE document_source_state ADD COLUMN eligible_until TEXT"
        )
    # The reverse index: which documents declare which source. Retrieval is
    # lexical, so two chunks that share nothing but a `sources[]` entry are
    # unreachable from each other and from the words of the source itself —
    # measured at 0 of 2 (docs/CONTEXT-AND-MEMORY.md). This table is the edge
    # those two chunks already assert but that nothing could read.
    #
    # Derived and disposable: every row is rebuilt from the documents, so it is
    # dropped and repopulated rather than migrated, and no validation depends
    # on it. That is deliberate — carrying the same link durably on the chunk
    # would put it under `chunk_source_digests` and `validate_metadata`, where
    # editing or deleting the referenced file becomes a bank-validation error.
    connection.execute(
        """
        CREATE TABLE IF NOT EXISTS document_links(
            path TEXT NOT NULL,
            ref_path TEXT NOT NULL,
            ref_kind TEXT NOT NULL,
            PRIMARY KEY(path, ref_path, ref_kind)
        )
        """
    )
    # The whole point is the reverse direction: given a source, who cites it.
    connection.execute(
        "CREATE INDEX IF NOT EXISTS document_links_ref ON document_links(ref_path)"
    )
    connection.execute(
        "CREATE INDEX IF NOT EXISTS document_links_source_path "
        "ON document_links(ref_path, ref_kind, path)"
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
        "git", "-C", str(workspace_roots.project_root(repository)), "rev-list", "--count",
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


SkillStat = tuple[str, str, int, int]


def skill_tree_fingerprint(
    repository: Path, skill_stats: Optional[Iterable[SkillStat]] = None
) -> str:
    """Fingerprint every mirrored skill file from stat metadata alone.

    Parity compares all editions, but only the first discovered copy of a skill
    reaches the index, so the per-document stat cache cannot notice a drifting
    mirror. This fingerprint can, without reading any file.

    ``skill_stats`` — (edition, path-under-skills, mtime_ns, size) tuples — lets
    a caller that already walked the skill trees (document discovery stats the
    same files) reuse that work instead of statting every mirror a second time.
    Entries are canonically sorted so both computations produce one digest.
    """
    if skill_stats is None:
        entries: list[SkillStat] = []
        layout = workspace_roots.roots(repository)
        trees = [
            base / edition / "skills"
            for base in dict.fromkeys((layout.tooling, layout.project))
            for edition in SKILL_EDITIONS
        ]
        for root in trees:
            edition = root.parent.name
            if not root.is_dir():
                continue
            for path in sorted(root.glob("**/*.md")):
                if not path.is_file() or path.is_symlink():
                    continue
                status = path.stat()
                entries.append(
                    (
                        edition,
                        path.relative_to(root).as_posix(),
                        status.st_mtime_ns,
                        status.st_size,
                    )
                )
    else:
        entries = list(skill_stats)
    digest = hashlib.sha256()
    for edition, relative, mtime_ns, size in sorted(entries):
        digest.update(
            f"{edition}\0{relative}\0{mtime_ns}\0{size}\n".encode("utf-8")
        )
    return digest.hexdigest()


def index_fingerprints(
    repository: Path, skill_stats: Optional[Iterable[SkillStat]] = None
) -> dict[str, str]:
    """The fingerprints that decide whether cached index state is current."""
    return {
        INDEX_CONFIG_KEY: config_fingerprint(repository),
        INDEX_SKILL_KEY: skill_tree_fingerprint(repository, skill_stats),
        INDEX_SECRET_POLICY_KEY: secret_policy_fingerprint(),
    }


def reusable_source_state(
    connection: sqlite3.Connection,
    repository: Path,
    fingerprints: Optional[dict[str, str]] = None,
) -> tuple[SourceState, dict[str, str]]:
    """Return the retainable stat cache plus the fingerprints that gate it."""
    ensure_metadata_tables(connection)
    if fingerprints is None:
        fingerprints = index_fingerprints(repository)
    stored = load_index_state(connection)
    if stored.get(INDEX_CONFIG_KEY) != fingerprints[INDEX_CONFIG_KEY]:
        # Runtime configuration decides eligibility for every indexed record,
        # so a configuration change invalidates the whole cache.
        return {}, fingerprints
    if stored.get(INDEX_SECRET_POLICY_KEY) != fingerprints[INDEX_SECRET_POLICY_KEY]:
        # Indexed text is masked by the secret patterns, so a changed pattern
        # set must reach documents whose files did not change.
        return {}, fingerprints
    indexed = {row[0] for row in connection.execute("SELECT path FROM documents")}
    return {
        row[0]: (int(row[1]), int(row[2]), row[3])
        for row in connection.execute(
            "SELECT path, mtime_ns, size, eligible_until FROM document_source_state"
        )
        if row[0] in indexed
    }, fingerprints


def effective_canonical_edition(repository: Path, configured: str) -> str:
    """The skill tree parity compares against in this checkout.

    The configured canonical tree (`.agents`) is absent from a single-tool
    install: `--tool claude` ships only `.claude/skills`, `--tool cursor` only
    `.cursor/skills`. Measuring those against a tree that is not there
    reported every skill as "absent from canonical", so the parity check the
    project-brain skill tells agents to run failed on every such install. The
    first tree present, in SKILL_EDITIONS order, stands in for it; the
    configuration itself is left alone.
    """
    if (repository / configured / "skills").is_dir():
        return configured
    for edition in SKILL_EDITIONS:
        if (repository / edition / "skills").is_dir():
            return edition
    return configured


def skill_mirror_drift(repository: Path, canonical_edition: str) -> list[dict[str, object]]:
    canonical_edition = effective_canonical_edition(repository, canonical_edition)
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

    A mirror directory that holds none of a class's files did not receive
    that class. A `--tool claude` install carries `.cursor/README.md` and
    `.codex/README.md` and nothing else of those tools, and reading those
    directories as installed mirrors reported every governance document as
    missing from them.
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
            sources = [
                (rel, path)
                for rel, path in _mirror_iter_canonical(canonical, cls.get("only"))
                if not _mirror_is_skipped(rel, skips)
            ]
            if sources and not any((mirror / rel).is_file() for rel, _ in sources):
                continue
            expected: set[str] = set()
            for rel, path in sources:
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
    """Nearest ancestor carrying at least two maintained PHP editions."""
    resolved = repository.resolve()
    for candidate in resolved.parents:
        present = [
            name for name, path in CROSS_EDITION_PATHS.items()
            if (candidate / path).is_dir()
        ]
        if len(present) >= 2:
            return candidate
    return None


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
    editions = [
        name for name, path in CROSS_EDITION_PATHS.items()
        if (root / path).is_dir()
    ]
    digests: dict[str, dict[str, str]] = {}
    for name in editions:
        base = root / CROSS_EDITION_PATHS[name]
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


def _chunk_frontmatter(content: str) -> dict[str, Any]:
    """A durable chunk's metadata block, parsed from the content in hand.

    Read from the content the indexer already holds rather than from the file:
    this runs on the prompt hot path, once per document, and re-opening every
    chunk to parse frontmatter a second time is exactly the cost the single
    filesystem pass was built to avoid.
    """
    if not content.startswith("---\n"):
        return {}
    try:
        raw, _ = content[4:].split("\n---\n", 1)
        metadata = json.loads(raw)
    except (ValueError, TypeError):
        return {}
    return metadata if isinstance(metadata, dict) else {}


def _chunk_digests(content: str) -> str:
    """The declared source digests, as the metadata column wants them."""
    digests = _chunk_frontmatter(content).get("source_digests")
    return json.dumps(digests if isinstance(digests, list) else [], sort_keys=True)


def _link_rows(path: str, sources: object) -> list[tuple[str, str, str]]:
    """One `document_links` row per source a document declares.

    `ref_kind` carries a single value, `source`, and the two candidates for a
    second were both rejected on evidence rather than left for later:

    * `fingerprint` is dead by construction — `sources_are_fresh` requires the
      fingerprint path set to equal the sources path set, so it can never name
      anything `source` does not already name.
    * `file` (a task's `files[]`) is Git churn in the wrong frame. Every live
      task in this repository records twenty entries led by phpunit cache,
      vendored JavaScript and dev container dumps, and `changed_paths` writes
      them relative to the Git toplevel while every other path in the index is
      relative to the repository root. Linking them would fill the table with
      build artifacts that resolve to nothing.
    """
    if not isinstance(sources, list):
        return []
    rows = []
    for source in sources:
        if not isinstance(source, str):
            continue
        # Anchored the way `fingerprint()` anchors: a link is to the document,
        # not to a line range inside it.
        reference = source.split("#", 1)[0].strip()
        if reference:
            rows.append((path, reference, "source"))
    return sorted(set(rows))


def _legacy_metadata(
    path: str, kind: str, content: str, source_hash: Optional[str] = None
) -> tuple[object, ...]:
    # Repository documents carry no provenance timestamp or confidence of
    # their own: '' means "never decayed" and 1.0 keeps them rank-neutral.
    # A chunk promoted from a claim only its agent checked says so in its tags.
    attested = (
        "agent"
        if kind == "memory"
        and "agent-attested" in (_chunk_frontmatter(content).get("tags") or [])
        else ""
    )
    return (
        path, category_for(kind), "public", "*", "verified", "active",
        source_hash or _content_hash(content), None, "[]",
        _chunk_digests(content) if kind == "memory" else "[]",
        "", 1.0, attested,
    )


def _brain_documents(
    repository: Path, config: dict[str, Any]
) -> tuple[
    list[tuple[str, str, str, str, str]],
    list[tuple[object, ...]],
    list[dict[str, str]],
    list[tuple[str, str, str]],
]:
    documents: list[tuple[str, str, str, str, str]] = []
    metadata_rows: list[tuple[object, ...]] = []
    excluded: list[dict[str, str]] = []
    links: list[tuple[str, str, str]] = []
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
                relative,
                # `event` is the one record type that reports that something
                # happened rather than what to do about it, which is what the
                # episodic layer is for. `incident` deliberately stays
                # semantic: an open incident is active, urgent, verified
                # content that belongs in the three semantic slots, and it is
                # promotable, which nothing episodic is.
                "episodic" if record["type"] == "event" else "semantic",
                f"brain-{record['type']}", title,
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
                str(record.get("updated_at") or ""),
                float(record.get("confidence", 1.0)),
                record_attestation(record),
            )
        )
        links.extend(_link_rows(relative, record.get("sources")))
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
                    str(handoff.get("updated_at") or ""),
                    float(task.get("confidence", 1.0)),
                    "",
                )
            )
            # From the handoff's own frontmatter, not the task's: the link
            # describes what this document declares.
            links.extend(_link_rows(relative, handoff.get("sources")))
    return documents, metadata_rows, excluded, links


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
        connection.execute(
            f"DELETE FROM document_links WHERE path IN ({placeholders})", chunk
        )


def index_documents(
    connection: sqlite3.Connection,
    repository: Path,
    legacy_documents: list[DocumentRow],
    *,
    retained: Optional[SourceState] = None,
    source_state: Optional[SourceState] = None,
    fingerprints: Optional[dict[str, str]] = None,
    source_hashes: Optional[dict[str, str]] = None,
) -> dict[str, object]:
    """Replace the index, reusing rows the caller proved unchanged.

    ``retained`` holds repository documents whose stat still matches the last
    successful index; their rows survive untouched and ``legacy_documents``
    then carries only the new or changed ones. ``None`` rebuilds everything.

    ``source_hashes`` gives the file digest of a document whose indexed text
    is not the file's text (masked secret values). Governed retrieval
    re-hashes the file on disk, so the stored hash must be the file's, or the
    document is excluded as stale on every turn.

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
        parity_drift = skill_mirror_drift(
            workspace_roots.tooling_root(repository), str(config["canonical_edition"])
        )
    brain_documents, brain_metadata, excluded, brain_links = _brain_documents(
        repository, config
    )
    documents = [*legacy_documents, *brain_documents]
    metadata = [
        _legacy_metadata(path, kind, content, (source_hashes or {}).get(path))
        for path, _, kind, _, _, content in legacy_documents
    ] + brain_metadata
    # Only durable chunks carry `sources` among repository documents; the rest
    # are prose with no declared provenance to link. Brain records are always
    # rebuilt, so their links are always complete; repository links follow the
    # documents, surviving for retained paths and being dropped with dropped
    # ones, which is why an existing database has to be forced through one full
    # re-read when the table first appears (see `connect`).
    links = [
        row
        for path, _, kind, _, _, content in legacy_documents
        if kind == "memory"
        for row in _link_rows(path, _chunk_frontmatter(content).get("sources"))
    ] + brain_links
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
            connection.execute("DELETE FROM document_links")
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
                record_id, conflicts, source_fingerprints, updated_at, confidence,
                attestation
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            metadata,
        )
        connection.executemany(
            "INSERT OR REPLACE INTO document_links(path, ref_path, ref_kind) "
            "VALUES (?, ?, ?)",
            links,
        )
        connection.executemany(
            "INSERT INTO document_source_state(path, mtime_ns, size, eligible_until) "
            "VALUES (?, ?, ?, ?)",
            [
                (path, value[0], value[1], value[2] if len(value) > 2 else None)
                for path, value in sorted(state.items())
            ],
        )
        store_index_state(
            connection,
            {
                # A caller that walked the sources already computed these; only
                # a full rebuild without one re-derives them from disk.
                **(fingerprints or index_fingerprints(repository)),
                INDEX_PARITY_KEY: json.dumps(parity_drift, sort_keys=True),
                # The index content changed, so every cache derived from it
                # (token frequencies) is invalid from this point on.
                INDEX_GENERATION_KEY: new_uuid(),
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


def token_document_frequencies(
    connection: sqlite3.Connection, tokens: Iterable[str]
) -> dict[str, Optional[int]]:
    """Document frequency per token, cached against the current index.

    Stop-word filtering and prompt distillation both need one COUNT query per
    token, on every prompt, and the answers only change when the index is
    rewritten. The cache lives in ``index_state`` keyed by the index
    generation, so a rebuilt index invalidates every stored frequency at once
    while an unchanged index answers from a single row. FTS matching is
    case-insensitive, so frequencies are cached under the casefolded token.
    """
    ensure_metadata_tables(connection)
    state = load_index_state(connection)
    generation = state.get(INDEX_GENERATION_KEY, "")
    cached: dict[str, int] = {}
    try:
        stored = json.loads(state.get(TOKEN_FREQUENCY_KEY, ""))
    except (TypeError, ValueError):
        stored = None
    if isinstance(stored, dict) and stored.get("generation") == generation:
        frequencies = stored.get("frequencies")
        if isinstance(frequencies, dict):
            cached = {
                key: value
                for key, value in frequencies.items()
                if isinstance(key, str) and isinstance(value, int)
            }
    result: dict[str, Optional[int]] = {}
    learned = False
    for token in tokens:
        folded = token.casefold()
        if folded in cached:
            result[token] = cached[folded]
            continue
        count = _document_frequency(connection, token)
        result[token] = count
        if count is not None:
            cached[folded] = count
            learned = True
    if learned:
        if len(cached) > TOKEN_FREQUENCY_LIMIT:
            # Novel prompts would grow the cache without bound; restarting
            # from this query's tokens is cheaper than per-token recency.
            cached = {
                token.casefold(): count
                for token, count in result.items()
                if count is not None
            }
        payload = json.dumps(
            {"generation": generation, "frequencies": cached}, sort_keys=True
        )
        try:
            # Persisting the cache is an optimisation, never an obligation: a
            # caller mid-transaction or a read-only database keeps its answer
            # and simply recomputes next time.
            if not connection.in_transaction:
                with connection:
                    store_index_state(connection, {TOKEN_FREQUENCY_KEY: payload})
        except sqlite3.Error:
            pass
    return result


def evidence_tokens(tokens: list[str]) -> list[str]:
    """The tokens that may count as evidence of relevance."""
    return [token for token in tokens if token.casefold() not in EVIDENCE_STOPWORDS]


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
    frequencies = token_document_frequencies(connection, tokens)
    matched = [
        (token, frequencies[token]) for token in tokens if frequencies[token]
    ]
    informative = [
        token for token, count in matched if count <= total * STOPWORD_DOCUMENT_RATIO
    ]
    # Never discard every term: a query built only from common words must still
    # return its best matches rather than nothing at all.
    return informative or [token for token, _ in matched] or tokens


def token_coverage(
    connection: sqlite3.Connection, tokens: list[str]
) -> tuple[dict[str, int], dict[str, list[str]]]:
    """Count distinct query terms per document, and note distinctive matches.

    Counting terms equally punishes exactly the wrong document. A focused note
    that contains only the one term that matters scores 1, while a document
    sharing two unremarkable words scores 2 — so the answer loses to the noise.
    A term rare in this corpus may be evidence on its own; ``distinctive``
    maps each document to the rare terms it matched, for is_relevant to judge.
    """
    total = connection.execute("SELECT COUNT(*) FROM documents").fetchone()[0] or 1
    coverage: dict[str, int] = {}
    distinctive: dict[str, list[str]] = {}
    for token in tokens:
        try:
            rows = connection.execute(
                "SELECT path FROM documents WHERE documents MATCH ?", (f'"{token}"',)
            ).fetchall()
        except sqlite3.OperationalError:
            continue
        # At least one document: in an index of fewer than ten, a share of
        # the corpus rounds below one and no term - not even an identifier
        # only one document carries - would count as rare.
        rare = len(rows) <= max(1.0, total * DISTINCTIVE_DOCUMENT_RATIO)
        for row in rows:
            coverage[row[0]] = coverage.get(row[0], 0) + 1
            if rare:
                distinctive.setdefault(row[0], []).append(token)
    return coverage, distinctive


# A term that names one thing: a ticket, version or record number, a class,
# method or constant name.
_ANCHOR_SHAPE = re.compile(r"\d|[a-z][A-Z]|^[A-Z][a-z]+[A-Z]|_")


def is_anchor(token: str, where: Iterable[str] = ()) -> bool:
    """Whether a query term names something rather than being a word that is
    merely rare here: an identifier-shaped term, or a word of the document's
    own path or title (``where``, casefolded words)."""
    return bool(_ANCHOR_SHAPE.search(token)) or token.casefold() in set(where)


def is_relevant(
    path: str,
    coverage: dict[str, int],
    distinctive: dict[str, list[str]],
    minimum: int,
    title: str = "",
    *,
    anchored: bool = True,
) -> bool:
    """A document qualifies on breadth of match, or on one rare term that names
    something.

    A rare word alone used to admit a document, and in an index of a hundred
    documents a word in ten of them is rare: on 61 real prompts such matches
    were noise in six of seven judged cases. One rare term that is an anchor -
    a number, an identifier from code, a word of the document's own path or
    title - still admits it, because a spurious result wastes a slot while a
    hidden one denies the agent an answer the project already holds.
    ``anchored=False`` skips the anchor test - for skills, which a capsule
    never delivers on a weak match and which explicit retrieval still ranks.
    """
    if coverage.get(path, 0) >= minimum:
        return True
    if not anchored:
        return path in distinctive
    where = re.findall(r"\w+", f"{path} {title}".casefold())
    return any(is_anchor(token, where) for token in distinctive.get(path, ()))


def match_strength(path: str, coverage: dict[str, int], minimum: int) -> str:
    """How a document qualified: on breadth of match, or on one rare term.

    `is_relevant` admits a candidate two different ways and then forgets which
    one applied, so a document that answered the whole query and one admitted
    by a single unusual word arrive looking identical. Naming the difference is
    what lets a turn tell "memory found this" from "memory found something
    that shares a rare word with this".

    ``covered`` means the document carried at least the required number of
    distinct query terms; ``distinctive`` means it did not and was admitted on
    rarity alone. When the query offers only one informative term the
    requirement is one, so every match is `covered` - correctly: there is no
    weaker way to match a one-term query.
    """
    return "covered" if coverage.get(path, 0) >= minimum else "distinctive"


def required_coverage(tokens: list[str]) -> int:
    """How many distinct terms a document must contain to count as relevant.

    One shared word is not evidence of relevance. Demanding two, once the query
    offers two, is what separates a document about the subject from one that
    merely mentions a word from it.
    """
    return MIN_TOKEN_COVERAGE if len(tokens) >= MIN_TOKEN_COVERAGE else 1


# The part of a document a capsule quotes. A capsule that names a document
# helps only if the text it carries holds the answer: the section sharing the
# most terms with the request was chosen by literal words and then cut to its
# opening characters, so a plural in the request picked the wrong section and
# an answer at the end of the right one was cut off.
EXCERPT_MARK_OPEN = "\x02"
EXCERPT_MARK_CLOSE = "\x03"
_EXCERPT_MARKED = re.compile("\x02(.*?)\x03", re.S)
_EXCERPT_HEADING = re.compile(r"^#{1,6}\s")
_EXCERPT_LIST_ITEM = re.compile(r"^(?:[-*+]|\d+[.)])\s|^\|")
_EXCERPT_SENTENCE_END = re.compile(r"(?<=[.!?])\s+(?=\S)")


_TERM_FORMS: dict[str, str] = {}


def term_forms(words: Iterable[str]) -> dict[str, str]:
    """Each word as the index's tokenizer reads it: its stems, space-joined.

    The marks come from the index's own Porter tokenizer, so "sessions" is
    marked for a request about a "session", and a query term and the words it
    marked have to count as one term. A suffix rule of our own could not agree
    with Porter: it read "classes" as "class" but "class" as "clas", so the
    rare "class" that answered a question about classes lost that word's
    weight to two common words in another section. The tokenizer is asked
    itself, through the vocabulary of an in-memory table; a SQLite without
    one gets the casefolded word, the same on both sides.
    """
    cached = {word: _TERM_FORMS.get(word) for word in words}
    missing = [word for word, form in cached.items() if form is None]
    fresh = _tokenizer_forms(missing) if missing else {}
    if fresh:
        if len(_TERM_FORMS) + len(fresh) > TERM_FORM_CACHE_LIMIT:
            _TERM_FORMS.clear()
        _TERM_FORMS.update(fresh)
    return {
        word: fresh[word] if form is None else form for word, form in cached.items()
    }


def _tokenizer_forms(words: list[str]) -> dict[str, str]:
    stems: dict[int, list[str]] = {}
    try:
        stemmer = sqlite3.connect(":memory:")
        try:
            stemmer.execute(
                f"CREATE VIRTUAL TABLE words USING fts5(word, tokenize = '{TOKENIZER}')"
            )
            stemmer.execute(
                "CREATE VIRTUAL TABLE forms USING fts5vocab(words, 'instance')"
            )
            stemmer.executemany(
                "INSERT INTO words(rowid, word) VALUES (?, ?)", enumerate(words, 1)
            )
            for row, term in stemmer.execute(
                "SELECT doc, term FROM forms ORDER BY doc, offset"
            ):
                stems.setdefault(int(row), []).append(str(term))
        finally:
            stemmer.close()
    except sqlite3.Error:
        return {word: word.casefold() for word in words}
    return {word: " ".join(stems.get(row, ())) for row, word in enumerate(words, 1)}


def _unmarked(text: str) -> str:
    return _EXCERPT_MARKED.sub(r"\1", text)


def marked_document(
    connection: sqlite3.Connection, path: str, query: str
) -> Optional[str]:
    """The document's text with every match of the query marked, or None.

    FTS5 marks the matches with the same tokenizer retrieval matched them by,
    so the excerpt is chosen by exactly the words that selected the document.
    """
    try:
        tokens = evidence_tokens(informative_tokens(connection, query_tokens(query)))
    except RetrievalError:
        return None
    if not tokens:
        return None
    try:
        row = connection.execute(
            "SELECT highlight(documents, 5, ?, ?) FROM documents "
            "WHERE documents MATCH ? AND path = ?",
            (
                EXCERPT_MARK_OPEN,
                EXCERPT_MARK_CLOSE,
                " OR ".join(f'"{token}"' for token in tokens),
                path,
            ),
        ).fetchone()
    except sqlite3.Error:
        return None
    return str(row[0]) if row is not None and row[0] is not None else None


def _excerpt_sections(text: str) -> list[tuple[str, list[str]]]:
    if text.startswith("---\n"):
        _, separator, remainder = text[4:].partition("\n---\n")
        if separator:
            text = remainder
    sections: list[tuple[str, list[str]]] = [("", [])]
    for line in text.splitlines():
        if _EXCERPT_HEADING.match(line):
            sections.append((line.lstrip("#").strip(), []))
        else:
            sections[-1][1].append(line)
    return sections


def _excerpt_units(lines: list[str]) -> list[str]:
    """Sentences in order - of paragraphs, list items and table rows: what a
    window moves by. A changelog entry is one list item a kilobyte and more
    long, so a window that moved by items could only cut one."""
    units: list[str] = []
    block: list[str] = []

    def flush() -> None:
        if block:
            text = " ".join(" ".join(block).split())
            units.extend(part for part in _EXCERPT_SENTENCE_END.split(text) if part)
            block.clear()

    for line in lines:
        stripped = line.strip()
        if not stripped:
            flush()
        elif _EXCERPT_LIST_ITEM.match(stripped):
            flush()
            block.append(stripped)
            if stripped.startswith("|"):
                flush()
        else:
            block.append(stripped)
    flush()
    return units


def excerpt_weights(connection: sqlite3.Connection, query: str) -> dict[str, float]:
    """What a match of each query term says about a passage, by its rarity.

    A real prompt runs to a thousand characters and shares a dozen words
    with every long section; counting matched terms picked the changelog
    entry with the most common words in it, not the one naming the ticket.
    """
    try:
        tokens = evidence_tokens(informative_tokens(connection, query_tokens(query)))
    except RetrievalError:
        return {}
    if not tokens:
        return {}
    try:
        total = connection.execute("SELECT COUNT(*) FROM documents").fetchone()[0] or 1
    except sqlite3.Error:
        return {}
    frequencies = token_document_frequencies(connection, tokens)
    forms = term_forms(tokens)
    weights: dict[str, float] = {}
    for token in tokens:
        frequency = frequencies.get(token) or 1
        weight = max(0.05, math.log((total + 1) / (frequency + 0.5)))
        for key in forms[token].split():
            weights[key] = max(weights.get(key, 0.0), weight)
    return weights


def _excerpt_score(
    units: list[str], weights: Optional[dict[str, float]] = None
) -> tuple[float, int]:
    found = [word for unit in units for word in _EXCERPT_MARKED.findall(unit)]
    forms = term_forms(found)
    # FTS5 merges adjacent matches into one highlight. Its visual grouping
    # must not turn two query terms into one unknown, low-weight phrase.
    return _excerpt_term_score([term for word in found for term in forms[word].split()], weights)


def _excerpt_term_score(
    found: list[str], weights: Optional[dict[str, float]] = None
) -> tuple[float, int]:
    terms = set(found)
    if weights:
        floor = min(weights.values())
        value = sum(weights.get(term, floor) for term in terms)
    else:
        value = float(len(terms))
    return round(value, 6), len(found)


def section_slug(heading: str) -> str:
    slug = re.sub(r"[^\w\s-]", "", heading.casefold(), flags=re.UNICODE)
    return re.sub(r"[\s_]+", "-", slug).strip("-")[:60]


def quoted_section(
    text: str, title: str = "", weights: Optional[dict[str, float]] = None
) -> Optional[dict[str, Any]]:
    """The section an excerpt quotes: the one carrying most of the query.

    ``text`` is a marked document (see marked_document) or plain text, which
    has no marks and yields the opening prose. Returns the heading (empty for
    the prose before the first heading), its slug, the section's units, and a
    hash of the section's text - what a conversation was handed, for not
    handing it again.
    """
    sections = _excerpt_sections(text)
    # Every marked word asked of the tokenizer at once, not per section.
    term_forms(_EXCERPT_MARKED.findall(text))
    best: Optional[tuple[tuple[float, int, float, int], int]] = None
    first_with_body: Optional[int] = None
    for index, (heading, lines) in enumerate(sections):
        units = _excerpt_units(lines)
        if not units:
            continue
        if first_with_body is None:
            first_with_body = index
        # A heading can identify a section without repeating its words in
        # the prose. Preserve that evidence, resolving exact ties in favour
        # of terms in the text the capsule can actually quote.
        score = (*_excerpt_score([heading, *units], weights), *_excerpt_score(units, weights))
        if best is None or score > best[0]:
            best = (score, index)
    if best is None or first_with_body is None:
        return None
    index = best[1] if best[0][0] > 0 else first_with_body
    heading, lines = sections[index]
    units = _excerpt_units(lines)
    plain_heading = " ".join(_unmarked(heading).split())
    digest = hashlib.sha256(
        "\n".join([plain_heading, *(_unmarked(unit) for unit in units)]).encode("utf-8")
    ).hexdigest()[:16]
    return {
        "heading": plain_heading,
        "slug": section_slug(plain_heading),
        "units": units,
        "hash": digest,
        "shown": plain_heading if plain_heading.casefold() != title.casefold() else "",
    }


def _excerpt_piece(
    unit: str, budget: int, weights: Optional[dict[str, float]]
) -> tuple[tuple[float, int], str, int, int]:
    """A bounded slice of an oversized sentence, scored on visible matches.

    Windows begin around marked words, not every character. Binary searches
    restrict scoring to the marks inside each window; the fixed character
    budget bounds that work even for a very long paragraph.
    """
    text = _unmarked(unit)
    words: list[tuple[int, int, str]] = []
    removed = 0
    for mark in _EXCERPT_MARKED.finditer(unit):
        offset = mark.start() - removed
        for word in re.finditer(r"\w+", mark.group(1)):
            words.append((offset + word.start(), offset + word.end(), word.group(0)))
        removed += 2
    forms = term_forms(word for _, _, word in words)
    starts = [start for start, _, _ in words]
    ends = [end for _, end, _ in words]
    positions = {0}
    for start, end, _ in words:
        positions.update((max(0, start - budget // 3), start, max(0, end - budget)))
    best: Optional[tuple[tuple[float, int], int, int, int]] = None
    for position in sorted(positions):
        left = position
        if left and not text[left - 1].isspace():
            space = text.rfind(" ", 0, left + 1)
            left = space + 1 if space >= 0 else left
        right = min(len(text), left + budget)
        if right < len(text) and not text[right].isspace():
            space = text.rfind(" ", left, right)
            if space > left:
                right = space
        first, last = bisect_left(starts, left), bisect_right(ends, right)
        found = [term for _, _, word in words[first:last] for term in forms[word].split()]
        score = _excerpt_term_score(found, weights)
        # Keep context before the first visible match (a qualifier or a
        # condition), without sacrificing any stronger evidence that fits.
        distance = abs(starts[first] - left - budget // 3) if first < last else 0
        if (best is None or score > best[0]
                or (score == best[0] and score[1] and distance < best[1])):
            best = score, distance, left, right
    assert best is not None
    score, _, left, right = best
    return score, text[left:right].strip(), left, right


def excerpt_window(
    units: list[str], limit: int, weights: Optional[dict[str, float]] = None
) -> str:
    """The stretch of a section that carries most of the query, within limit.

    Whole sentences and list items, starting where the matches are rather
    than at the top; a list item keeps the line that introduces it, which is
    where its condition usually is. Elided text is shown as an ellipsis.
    """
    plain = [_unmarked(unit) for unit in units]
    whole = " ".join(plain)
    if len(whole) <= limit:
        return whole
    budget = max(1, limit - 4)
    best: Optional[tuple[tuple[float, int], int, int, Optional[tuple[str, int, int]]]] = None
    for start in range(len(units)):
        end, size = start, 0
        while end < len(units) and size + len(plain[end]) + (1 if end > start else 0) <= budget:
            size += len(plain[end]) + (1 if end > start else 0)
            end += 1
        piece = None
        if end == start:
            score, text, left, right = _excerpt_piece(units[start], budget, weights)
            piece = text, left, right
        else:
            score = _excerpt_score(units[start:end], weights)
        # Of equal windows the one starting at the match wins: what follows a
        # matching sentence - the rest of a changelog entry, the steps after
        # a heading line - is what the window is for. A section with no match
        # at all has no such window and gives its opening, as quoted_section
        # promises plain text does; the latest start won there too, and
        # quoted the section's last sentence.
        if best is None or score > best[0] or (score == best[0] and score[1]):
            best = (score, start, end, piece)
    assert best is not None
    _, start, end, piece = best
    if piece is not None:
        text, left, right = piece
        prefix = "… " if left or start else ""
        suffix = " …" if right < len(plain[start]) or start + 1 < len(units) else ""
        return (prefix + text + suffix)[:limit]
    if start > 0 and plain[start - 1].endswith(":"):
        lead = len(plain[start - 1]) + 1
        size = sum(len(part) + 1 for part in plain[start:end]) - 1
        while end > start and size + lead > budget:
            end -= 1
            size -= len(plain[end]) + 1
        if size + lead <= budget:
            start -= 1
    text = " ".join(plain[start:end])
    return ("… " if start else "") + text + (" …" if end < len(units) else "")


def delivery_identity(
    connection: sqlite3.Connection, item: dict[str, Any], query: str
) -> tuple[str, str]:
    """What handing this item gives a conversation: (key, revision).

    A quoted item is the section its excerpt comes from, so a later question
    answered by another section of the same document still gets it; anything
    else - a skill, a weak match, a document with no matching section - is
    the document at its revision.
    """
    path = str(item["path"])
    revision = str(item.get("source_hash") or "")
    if item.get("layer") == "procedural" or item.get("match") == "distinctive":
        return path, revision
    marked = marked_document(connection, path, query)
    section = (
        quoted_section(marked, str(item.get("title") or ""), excerpt_weights(connection, query))
        if marked
        else None
    )
    if section is None:
        return path, revision
    return f"{path}#{section['slug']}", section["hash"]


# A host that compacts a conversation writes a record into the transcript it
# names to the hook: Claude Code a compact_boundary, Codex a compacted record.
# After one, the conversation holds a summary of what it was handed, so
# nothing counts as handed any more. A gap too large to scan is treated the
# same way: handing an item twice is the safe error.
COMPACTION_MARKERS = (b'"subtype":"compact_boundary"', b'"type":"compacted"')
TRANSCRIPT_SCAN_BYTES = 8 * 1024 * 1024


def transcript_compacted(
    entry: dict[str, Any], transcript: Optional[str]
) -> tuple[bool, Optional[dict[str, Any]]]:
    """Whether the conversation was compacted since its last turn, and the
    transcript position to compare against next turn."""
    if not transcript:
        return False, None
    try:
        path = Path(transcript)
        if not path.is_absolute() or path.is_symlink() or not path.is_file():
            return False, None
        size = path.stat().st_size
    except (OSError, ValueError):
        return False, None
    file_key = hashlib.sha256(
        str(path).encode("utf-8", "surrogateescape")
    ).hexdigest()[:16]
    state = {"file": file_key, "offset": size}
    previous = entry.get("transcript")
    if (
        not isinstance(previous, dict)
        or previous.get("file") != file_key
        or not isinstance(previous.get("offset"), int)
    ):
        return False, state
    start = previous["offset"]
    if size < start or size - start > TRANSCRIPT_SCAN_BYTES:
        return True, state
    try:
        with path.open("rb") as handle:
            # From the start of the line that was being written last turn, if
            # one was; a record completed before then was seen then.
            begin = start
            if start > 0:
                back = min(start, 65536)
                handle.seek(start - back)
                before = handle.read(back)
                if not before.endswith(b"\n"):
                    newline = before.rfind(b"\n")
                    begin = start - back + newline + 1 if newline >= 0 else start - back
            handle.seek(begin)
            chunk = handle.read(size - begin)
    except OSError:
        return False, state
    return any(marker in chunk for marker in COMPACTION_MARKERS), state


def _estimate_tokens(value: str) -> int:
    return max(1, (len(value) + 3) // 4)


def ranking_weight(
    authority: object, confidence: object, updated_at: object
) -> float:
    """How much of its BM25 relevance a candidate keeps.

    Verified evidence outranks observed at equal lexical fit, declared
    confidence scales linearly, and freshness decays with a half-life over the
    record's ``updated_at``. Every factor is floored: metadata modulates
    ranking, it never erases a lexical match. A document without a timestamp
    (repository files) keeps full freshness — its currency is already policed
    by the source-hash staleness check, not by age.
    """
    weight = AUTHORITY_WEIGHTS.get(str(authority), AUTHORITY_WEIGHT_DEFAULT)
    try:
        declared = float(confidence)  # type: ignore[arg-type]
    except (TypeError, ValueError):
        declared = 1.0
    declared = min(1.0, max(0.0, declared))
    weight *= CONFIDENCE_WEIGHT_FLOOR + (1.0 - CONFIDENCE_WEIGHT_FLOOR) * declared
    timestamp = str(updated_at or "")
    if timestamp:
        try:
            written = datetime.fromisoformat(timestamp)
        except ValueError:
            written = None
        if written is not None:
            if written.tzinfo is None:
                written = written.replace(tzinfo=timezone.utc)
            age_days = max(
                0.0,
                (datetime.now(timezone.utc) - written).total_seconds() / 86400.0,
            )
            decay = 0.5 ** (age_days / RECENCY_HALF_LIFE_DAYS)
            weight *= RECENCY_WEIGHT_FLOOR + (1.0 - RECENCY_WEIGHT_FLOOR) * decay
    return weight


def _candidates(
    connection: sqlite3.Connection, query: str, limit: int = 100
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    """Ranked candidates, plus what the ranking knew about the query.

    The diagnostics are computed here anyway and were discarded at the end of
    the call. A gate that has to decide whether this turn is worth retrieving
    for cannot recompute them without repeating the work.
    """
    ensure_metadata_tables(connection)
    tokens = informative_tokens(connection, query_tokens(query))
    evidence = evidence_tokens(tokens)
    if not evidence:
        # Nothing in the request is about anything: "thanks", "ok, go on".
        return [], {"informative_terms": 0, "distinctive_matches": 0, "top_score": None}
    coverage, distinctive = token_coverage(connection, evidence)
    minimum = required_coverage(evidence)
    rows = connection.execute(
        """
        SELECT
            d.rowid AS document_rowid, d.path, d.layer, d.kind, d.title,
            m.category, m.privacy, m.owner, m.authority, m.lifecycle,
            m.source_hash, m.record_id, m.conflicts,
            m.source_fingerprints, m.updated_at, m.confidence, m.attestation,
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
        if not is_relevant(
            row["path"], coverage, distinctive, minimum, str(row["title"] or ""),
            anchored=row["layer"] != "procedural",
        ):
            continue
        item = dict(row)
        item["conflicts"] = json.loads(item["conflicts"])
        item["source_fingerprints"] = json.loads(item["source_fingerprints"])
        item["match"] = match_strength(row["path"], coverage, minimum)
        # bm25() reports better matches as more negative, so relevance is its
        # negation; provenance quality and freshness then scale it.
        relevance = max(0.0, -float(item["score"]))
        item["adjusted_score"] = relevance * ranking_weight(
            item["authority"], item["confidence"], item["updated_at"]
        )
        result.append(item)
    # Re-rank the BM25 window: the raw score breaks adjusted ties so the
    # ordering stays deterministic even when every weight is neutral.
    result.sort(key=lambda item: (-item["adjusted_score"], item["score"], item["path"]))
    best_by_layer: dict[str, float] = {}
    for item in result:
        best_by_layer.setdefault(item["layer"], item["adjusted_score"])
    result = [
        item
        for item in result
        if item["adjusted_score"]
        >= RELATIVE_SCORE_FLOOR * best_by_layer[item["layer"]]
    ]
    # Rank is stamped here, on the lexically ranked list, rather than after
    # filtering: a position in the post-filter list says where a survivor
    # landed, not how well it answered the query, which is the only reading a
    # retrieval gate can use.
    for position, item in enumerate(result, start=1):
        item["rank"] = position
    _quote_candidates(connection, result, query, evidence)
    diagnostics = {
        "informative_terms": len(evidence),
        # The count of candidates admitted on rarity alone rather than the
        # count of rare TERMS the roadmap asked for: `token_coverage` decides
        # rarity per token but returns paths, so a per-term figure does not
        # exist without changing a signature shared by both retrieval paths.
        # This reading is free after the match strength H1-04 already records,
        # and it answers the same question - how much of this result rests on
        # a single unusual word.
        "distinctive_matches": sum(
            1 for item in result if item.get("match") == "distinctive"
        ),
        # Pre-filter, so a top candidate withheld by policy is still visible
        # to the gate. Corpus-scaled telemetry for H3, never a threshold.
        "top_score": (
            round(float(result[0]["adjusted_score"]), 6) if result else None
        ),
    }
    return result, diagnostics


def _quote_candidates(
    connection: sqlite3.Connection,
    items: list[dict[str, Any]],
    query: str,
    evidence: list[str],
) -> None:
    """Give each ranked candidate the stretch of its body the query found.

    The snippet was FTS snippet() over the whole content column, which centres
    on the densest run of matches: in a promoted chunk that is the JSON
    frontmatter, whose `title` repeats the heading under it, so the JSON
    capsule carried `"supersedes": [], "tags": [...]` where the rendered
    capsule quoted the finding, and the token estimate counted those keys. It
    is now chosen the way the rendered excerpt is (capsule_excerpts): the
    document marked by the words that selected it, the section carrying most
    of them, the window of that section where they are - frontmatter is never
    a section - and the estimate is of that text. One highlight() over the
    survivors replaces a snippet() that ran over every match.
    """
    rowids = [item.pop("document_rowid") for item in items]
    texts: dict[int, str] = {}
    expression = " OR ".join(f'"{token}"' for token in evidence)
    # In batches that stay inside SQLite's parameter limit, as deletes do.
    for start in range(0, len(rowids), DELETE_CHUNK):
        chunk = rowids[start : start + DELETE_CHUNK]
        placeholders = ", ".join("?" for _ in chunk)
        try:
            rows = connection.execute(
                "SELECT rowid, highlight(documents, 5, ?, ?) FROM documents "
                f"WHERE documents MATCH ? AND rowid IN ({placeholders})",
                (EXCERPT_MARK_OPEN, EXCERPT_MARK_CLOSE, expression, *chunk),
            ).fetchall()
        except sqlite3.Error:
            rows = []
        texts.update((row[0], str(row[1] or "")) for row in rows)
        # A candidate the evidence words do not mark is quoted from its plain
        # text, which gives its opening prose.
        missing = [rowid for rowid in chunk if rowid not in texts]
        if missing:
            rows = connection.execute(
                "SELECT rowid, content FROM documents WHERE rowid IN ("
                + ", ".join("?" for _ in missing)
                + ")",
                missing,
            ).fetchall()
            texts.update((row[0], str(row[1] or "")) for row in rows)
    weights = excerpt_weights(connection, query) if items else {}
    for rowid, item in zip(rowids, items):
        text = texts.get(rowid, "")
        section = (
            quoted_section(text, str(item.get("title") or ""), weights) if text else None
        )
        snippet = (
            excerpt_window(section["units"], SNIPPET_WINDOW_CHARS, weights)
            if section is not None
            # No prose at all, only headings: still never the frontmatter.
            else _body_snippet(_unmarked(text))[:SNIPPET_WINDOW_CHARS]
        )
        item["snippet"] = snippet
        item["estimated_tokens"] = _estimate_tokens(item["title"] + snippet)


def _retrieval_signature(
    query: str,
    selections: Iterable[tuple[str, str]],
    task_revision: int,
) -> dict[str, str]:
    """What identifies a retrieval for the purpose of noticing a repeat.

    Not the capsule: the returned packet carries a fresh manifest UUID every
    call, and the rendered form folds in an automatic checkpoint sentence and
    a last-turn summary that both move on their own. The invariant that
    actually holds across a repeated turn is the distilled query plus the set
    of selected identities and content hashes. The task revision is separate:
    Cursor's query can stay unchanged while its rendered working state moves.
    """
    ordered = sorted(set(selections))
    return {
        "query": hashlib.sha256(query.encode("utf-8")).hexdigest(),
        "paths": hashlib.sha256(
            "\n".join(f"{identity}\0{source_hash}" for identity, source_hash in ordered).encode(
                "utf-8"
            )
        ).hexdigest(),
        "task_revision": str(task_revision),
    }


def _retrieval_baseline_key(task_uuid: str, host: str, entry_point: str) -> str:
    return f"{task_uuid}:{host}:{entry_point}"


def _episode_signature(episode: dict[str, Any]) -> tuple[str, str]:
    content = _serialized_episode(episode)
    return (
        f"episode:{episode.get('id')}",
        hashlib.sha256(content.encode("utf-8")).hexdigest(),
    )


def _serialized_episode(episode: dict[str, Any]) -> str:
    return json.dumps(episode, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def _token_usage(
    selected: list[dict[str, Any]], episodes: list[dict[str, Any]]
) -> dict[str, int]:
    usage = {category: 0 for category in BUDGETS}
    for item in selected:
        usage[item["category"]] += item["estimated_tokens"]
    usage["local_episodes"] = sum(
        _estimate_tokens(_serialized_episode(episode)) for episode in episodes
    )
    usage["total"] = sum(usage.values())
    usage["target"] = TARGET_BUDGET
    usage["hard"] = HARD_BUDGET
    return usage


def _shown_items(capsule: dict[str, Any]) -> tuple[set[str], set[Any]]:
    """The documents (by path) and recorded episodes (by id) a capsule shows."""
    paths: set[str] = set()
    episodes: set[Any] = set()
    for layer in ("procedural", "semantic", "episodic"):
        items = capsule.get(layer)
        for item in items if isinstance(items, list) else []:
            if not isinstance(item, dict):
                continue
            if isinstance(item.get("path"), str):
                paths.add(item["path"])
            elif item.get("id") is not None:
                episodes.add(item["id"])
    return paths, episodes


def _load_last_retrievals(connection: sqlite3.Connection) -> dict[str, Any]:
    try:
        raw = load_index_state(connection).get(LAST_RETRIEVAL_KEY)
    except sqlite3.Error:
        return {}
    if not raw:
        return {}
    try:
        stored = json.loads(raw)
    except ValueError:
        return {}
    return stored if isinstance(stored, dict) else {}


def _remember_retrieval(
    connection: sqlite3.Connection, baseline_key: str, signature: dict[str, str]
) -> None:
    """Record this retrieval so the next turn can recognize a repeat.

    Best-effort by design: the store is the disposable index, and a database
    opened read-only must never turn a retrieval into an error. Losing the
    record costs one un-skipped turn, which is the safe direction.
    """
    stored = _load_last_retrievals(connection)
    stored.pop(baseline_key, None)
    stored[baseline_key] = signature
    if len(stored) > LAST_RETRIEVAL_RETENTION:
        for stale in list(stored)[: len(stored) - LAST_RETRIEVAL_RETENTION]:
            stored.pop(stale, None)
    payload = json.dumps(stored, sort_keys=False, separators=(",", ":"))
    try:
        if connection.in_transaction:
            store_index_state(connection, {LAST_RETRIEVAL_KEY: payload})
        else:
            with connection:
                store_index_state(connection, {LAST_RETRIEVAL_KEY: payload})
    except sqlite3.Error:
        return


def _load_session_deliveries(connection: sqlite3.Connection) -> dict[str, Any]:
    """What each conversation was handed, normalised entry by entry.

    The record is disposable bookkeeping: a part of it that is not the shape
    this runtime writes - a list where items belong, a turn that is not a
    number - is dropped, costing at most one repeated item, rather than
    failing the turn's retrieval.
    """
    try:
        raw = load_index_state(connection).get(SESSION_DELIVERIES_KEY)
        stored = json.loads(raw) if raw else {}
    except (sqlite3.Error, ValueError, RecursionError):
        return {}
    if not isinstance(stored, dict):
        return {}
    normalized: dict[str, Any] = {}
    for session, entry in stored.items():
        if not isinstance(session, str) or not isinstance(entry, dict):
            continue
        turn = entry.get("turn")
        items = entry.get("items")
        clean: dict[str, Any] = {
            "turn": turn if type(turn) is int and turn >= 0 else 0,
            "items": {
                key: seen
                for key, seen in (items.items() if isinstance(items, dict) else ())
                if isinstance(key, str)
                and isinstance(seen, list)
                and len(seen) == 2
                and type(seen[1]) is int
            },
        }
        if isinstance(entry.get("transcript"), dict):
            clean["transcript"] = entry["transcript"]
        normalized[session] = clean
    return normalized


def _remember_session_deliveries(
    connection: sqlite3.Connection,
    session_id: str,
    turn: int,
    delivered: dict[str, list[Any]],
    stored: dict[str, Any],
    transcript: Optional[dict[str, Any]] = None,
    *,
    reset: bool = False,
) -> None:
    """Record what the session holds: key -> [revision, turn handed].

    The key is the document, or the section a quoted excerpt came from (see
    delivery_identity). ``reset`` drops what earlier turns were handed: the
    host compacted the conversation since. Best-effort, like the repeat
    record: losing it costs one repeated item, the safe direction.
    """
    entry = stored.pop(session_id, None)
    items = entry.get("items") if isinstance(entry, dict) and not reset else None
    items = {
        path: seen
        for path, seen in (items or {}).items()
        if isinstance(seen, list)
        and len(seen) == 2
        and isinstance(seen[1], int)
        and turn - seen[1] < SESSION_NOVELTY_TURNS
    }
    items.update(delivered)
    stored[session_id] = {"turn": turn, "items": items}
    if transcript is not None:
        stored[session_id]["transcript"] = transcript
    for stale in list(stored)[: max(0, len(stored) - SESSION_DELIVERIES_RETENTION)]:
        stored.pop(stale, None)
    payload = json.dumps(stored, separators=(",", ":"))
    try:
        if connection.in_transaction:
            store_index_state(connection, {SESSION_DELIVERIES_KEY: payload})
        else:
            with connection:
                store_index_state(connection, {SESSION_DELIVERIES_KEY: payload})
    except sqlite3.Error:
        return


def gate_decision(
    mode: str,
    *,
    signature: dict[str, str],
    previous: Optional[dict[str, Any]],
    diagnostics: dict[str, Any],
    no_match: list[str],
    matched_count: int,
    selected_count: int,
    conversation: bool = False,
) -> dict[str, Any]:
    """Decide whether this turn was worth retrieving for, and say why.

    The unit is the turn, not the document. Document-level restraint already
    exists and cannot express "this turn needed nothing"; a pointer that says
    "relevant right now" about the wrong file costs more than no pointer at
    all, which is why the decision is recorded even when it is not acted on.

    Two rules fire today, both deterministic and both computable before any
    document body is opened. Everything else is recorded as a signal for the
    report that will decide whether a third rule is worth having.

    ``conversation`` says the selection has already been held against the
    conversation's own record of what it was handed: what is left, that
    conversation does not hold - another one was handed it, or this one lost
    it to a compaction or to the novelty window. The baseline is the task's,
    shared by every conversation, so its "repeat" would withhold exactly what
    the record decided to hand; for such a turn the record is the one notion
    of "seen", and the baseline only informs the signals.
    """
    query_unchanged = bool(previous) and previous.get("query") == signature["query"]
    selection_identical = (
        bool(previous) and previous.get("paths") == signature["paths"]
    )
    task_revision_unchanged = (
        bool(previous)
        and previous.get("task_revision") == signature["task_revision"]
    )
    signals = {
        "informative_terms": diagnostics.get("informative_terms", 0),
        "distinctive_matches": diagnostics.get("distinctive_matches", 0),
        "top_score": diagnostics.get("top_score"),
        "no_match": list(no_match),
        "query_unchanged_from_previous_turn": query_unchanged,
        "selection_identical_to_previous_turn": selection_identical,
    }
    if mode == "off":
        return {"decision": "retrieve", "mode": mode, "reason": "gate-off", "signals": signals}
    if selected_count == 0:
        # Nothing survived relevance, so retrieving and skipping deliver the
        # same thing. Naming it makes the empty turn countable instead of
        # indistinguishable from a turn that was never gated.
        reason = "no-relevant-match" if matched_count == 0 else "empty-after-filter"
        return {"decision": "skip", "mode": mode, "reason": reason, "signals": signals}
    if query_unchanged and selection_identical and task_revision_unchanged:
        if conversation:
            return {
                "decision": "retrieve", "mode": mode,
                "reason": "not-held-by-conversation", "signals": signals,
            }
        return {"decision": "skip", "mode": mode, "reason": "repeat-retrieval", "signals": signals}
    if query_unchanged and selection_identical and not task_revision_unchanged:
        return {"decision": "retrieve", "mode": mode, "reason": "task-changed", "signals": signals}
    return {"decision": "retrieve", "mode": mode, "reason": "new-selection", "signals": signals}


def _body_snippet(content: str) -> str:
    """A leading excerpt of a document's prose, never of its frontmatter.

    A durable chunk opens with a JSON metadata block that can run past the
    whole snippet allowance, so an unconditional `substr(content, 1, N)`
    delivers a wall of quoted keys and digests where the reader expects the
    first sentence. Candidates selected by the query are quoted from the
    section that matched (_quote_candidates) - FTS `snippet()` centred on the
    match and the match was often in the frontmatter; candidates pulled in by
    record id or by source path have no match to centre on and need this.
    """
    if content.startswith("---\n"):
        _, separator, remainder = content[4:].partition("\n---\n")
        if separator:
            content = remainder
    return content.lstrip()[:MAX_SNIPPET_CHARS]


def _conflict_candidates(
    connection: sqlite3.Connection, record_ids: set[str]
) -> list[dict[str, Any]]:
    if not record_ids:
        return []
    placeholders = ", ".join("?" for _ in record_ids)
    rows = connection.execute(
        f"""
        SELECT
            d.path, d.layer, d.kind, d.title, d.content,
            m.category, m.privacy, m.owner, m.authority, m.lifecycle,
            m.source_hash, m.record_id, m.conflicts,
            m.source_fingerprints, m.updated_at, m.confidence, m.attestation,
            0.0 AS score
        FROM documents AS d
        JOIN document_metadata AS m ON m.path = d.path
        WHERE m.record_id IN ({placeholders})
        ORDER BY d.path
        """,
        tuple(sorted(record_ids)),
    ).fetchall()
    result = []
    for row in rows:
        item = dict(row)
        item["snippet"] = _body_snippet(str(item.pop("content")))
        item["conflicts"] = json.loads(item["conflicts"])
        item["source_fingerprints"] = json.loads(item["source_fingerprints"])
        # A conflict partner is pulled in by record id, not by the query, so
        # it has no match strength of its own: it is here because something
        # else matched and it disagrees with it.
        item["match"] = "conflict"
        # Set explicitly rather than left absent: these rows reach the same
        # manifest projection as ranked candidates, and a lexical score they
        # never earned must read as zero, with no rank at all.
        item["adjusted_score"] = 0.0
        item["rank"] = None
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
            path = workspace_roots.resolve(repository, candidate["path"])
            try:
                content = path.read_text(encoding="utf-8") if path.is_file() else None
            except OSError:
                content = None
            # `source_hash` is the file's digest even when the indexed text is
            # masked, so this compares the file with the file it was indexed from.
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
        if reason is None and candidate["source_fingerprints"]:
            changed, missing = source_changes(
                repository, candidate["source_fingerprints"]
            )
            if missing:
                # The cited file is gone: there is nothing left to check the
                # knowledge against, so it leaves retrieval.
                reason = "source-missing"
            elif changed:
                # A cited file was edited after the knowledge was verified.
                # Any edit used to evict it - a comment, a new method - and
                # on a real project 65 of 70 resolved findings went that way.
                # It stays, marked for checking and ranked below fresh
                # knowledge; the reader decides whether it still holds.
                candidate["source_changed"] = changed
                candidate["adjusted_score"] = (
                    float(candidate.get("adjusted_score") or 0.0)
                    * SOURCE_CHANGED_WEIGHT
                )
        if reason:
            excluded.append({"path": candidate["path"], "reason": reason})
        else:
            selected.append(candidate)
    return selected, excluded


# How many path-linked documents may lead the selection. Set to the semantic
# slot count rather than to a number of its own: naming a path is a claim that
# these documents matter to this turn, so they take the layer the caller asked
# about and the query keeps whatever they leave. A smaller cap would reproduce
# the failure the link index exists to repair — in the measured case the query
# does not reach the linked chunks at all, so any slot reserved for it is a
# slot spent on documents the caller did not ask for.
PATH_LINK_LIMIT = CAPSULE_SEMANTIC_LIMIT


def _path_candidates(
    connection: sqlite3.Connection,
    repository: Path,
    config: dict[str, Any],
    paths: list[str],
    seen: set[str],
) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    """Documents that cite one of `paths`, shaped like ranked candidates.

    They carry no lexical score, because nothing lexical selected them: like a
    conflict partner they read as score zero with no rank, and they are marked
    `selection: path-link` so a manifest reader can tell a document the query
    found from one the caller's path dragged in.
    """
    candidates: list[dict[str, Any]] = []
    excluded: list[dict[str, str]] = []
    for reference in dict.fromkeys(paths):
        linked, withheld = linked_documents(
            connection, repository, config, reference
        )
        excluded.extend(withheld)
        for item in linked:
            if item["path"] in seen:
                # Already selected by the query; it arrived on its own merit
                # and is not relabelled as a path link.
                continue
            seen.add(item["path"])
            candidates.append(item)
    for item in candidates[PATH_LINK_LIMIT:]:
        excluded.append({"path": item["path"], "reason": "path-link-limit"})
    candidates = candidates[:PATH_LINK_LIMIT]
    content = {
        row["path"]: row["content"]
        for row in connection.execute(
            "SELECT path, content FROM documents WHERE path IN ("
            + ", ".join("?" for _ in candidates)
            + ")",
            [item["path"] for item in candidates],
        )
    } if candidates else {}
    for item in candidates:
        item["snippet"] = _body_snippet(str(content.get(item["path"], "")))
        item["match"] = None
        item["selection"] = "path-link"
        item["adjusted_score"] = 0.0
        item["rank"] = None
        item["estimated_tokens"] = _estimate_tokens(item["title"] + item["snippet"])
    return candidates, excluded


def linked_documents(
    connection: sqlite3.Connection,
    repository: Path,
    config: dict[str, Any],
    reference: str,
    *,
    prefix: bool = False,
    max_rows: Optional[int] = None,
) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    """Every eligible document that declares `reference` among its sources.

    Routed through `_runtime_filter` rather than reimplementing its checks,
    because a link query is a retrieval and the same policy has to apply.
    `search_documents` deliberately does not join `document_metadata`, and the
    comment there records why: once governed records entered a layer, a search
    that skipped the join leaked `restricted` bodies. A `links` command
    modelled on `search` would reopen exactly that hole.
    """
    ensure_metadata_tables(connection)
    reference = reference.split("#", 1)[0].strip()
    if not reference:
        return [], []
    if prefix:
        predicate = "l.ref_path = ? OR l.ref_path LIKE ? ESCAPE '\\'"
        pattern = reference.rstrip("/").replace("\\", "\\\\")
        pattern = pattern.replace("%", "\\%").replace("_", "\\_")
        parameters: tuple[str, ...] = (reference, f"{pattern}/%")
    else:
        predicate = "l.ref_path = ?"
        parameters = (reference,)
    if max_rows is not None and (type(max_rows) is not int or max_rows < 1):
        raise RetrievalError("Source-link row limit must be a positive integer")
    rows = connection.execute(
        f"""
        SELECT
            d.path, d.layer, d.kind, d.title,
            l.ref_path, l.ref_kind,
            m.category, m.privacy, m.owner, m.authority, m.lifecycle,
            m.source_hash, m.record_id, m.conflicts, m.source_fingerprints,
            m.updated_at, m.confidence, m.attestation
        FROM document_links AS l
        JOIN documents AS d ON d.path = l.path
        JOIN document_metadata AS m ON m.path = l.path
        WHERE {predicate}
        ORDER BY d.path, l.ref_path
        """ + (" LIMIT ?" if max_rows is not None else ""),
        (*parameters, max_rows) if max_rows is not None else parameters,
    ).fetchall()
    candidates = []
    for row in rows:
        item = dict(row)
        item["conflicts"] = json.loads(item["conflicts"])
        item["source_fingerprints"] = json.loads(item["source_fingerprints"])
        candidates.append(item)
    return _runtime_filter(repository, candidates, config)


def _shared_source_documents(
    connection: sqlite3.Connection,
    repository: Path,
    config: dict[str, Any],
    anchor: dict[str, Any],
    max_rows: int,
    current_digests: dict[str, Optional[str]],
) -> tuple[list[dict[str, Any]], int]:
    """Indexed neighbours declaring the same current canonical source revision.

    The common file need not be indexed. It is evidence for the relation,
    never a delivered item or another traversal seed. Missing digests and
    private/derived paths cannot establish this relation.
    """
    declared = {entry.get("path"): entry.get("sha256")
                for entry in anchor.get("source_fingerprints") or [] if isinstance(entry, dict)}
    references = sorted(key for key, digest in declared.items()
        if isinstance(key, str) and source_path_problem(key) is None
        and isinstance(digest, str) and re.fullmatch(r"[0-9a-f]{64}", digest))[:GRAPH_ROW_LIMIT]
    paths: set[str] = set()
    for reference in references:
        if connection.execute(
            "SELECT 1 FROM document_links WHERE path = ? AND ref_path = ? AND ref_kind = 'source'",
            (anchor["path"], reference),
        ).fetchone() is None:
            continue
        # A fixed-source covering index produces path order without sorting
        # the entire citation fanout. Exclusions and returned paths are both
        # bounded; duplicate references cannot multiply the neighbour rows.
        excluded = sorted(paths | {anchor["path"]})
        placeholders = ",".join("?" for _ in excluded)
        found = connection.execute(
            "SELECT path FROM document_links INDEXED BY document_links_source_path "
            "WHERE ref_path = ? AND ref_kind = 'source' "
            f"AND path NOT IN ({placeholders}) ORDER BY path LIMIT ?",
            (reference, *excluded, max_rows - len(paths)),
        ).fetchall()
        paths.update(row["path"] for row in found)
        if len(paths) >= max_rows:
            break
    if not paths:
        return [], 0
    # Hydrate only the bounded pool, scanning the document table once rather
    # than joining every repeated citation to its full document/metadata.
    rows = connection.execute(
        """
        SELECT d.path, d.layer, d.kind, d.title,
            m.category, m.privacy, m.owner, m.authority, m.lifecycle,
            m.source_hash, m.record_id, m.conflicts, m.source_fingerprints,
            m.updated_at, m.confidence, m.attestation
        FROM documents AS d JOIN document_metadata AS m ON m.path = d.path
        WHERE d.path IN (""" + ",".join("?" for _ in paths) + ") ORDER BY d.path",
        tuple(sorted(paths)),
    ).fetchall()
    candidates = []
    for row in rows:
        item = dict(row)
        item["conflicts"] = json.loads(item["conflicts"])
        item["source_fingerprints"] = json.loads(item["source_fingerprints"])
        peer = {entry.get("path"): entry.get("sha256")
                for entry in item["source_fingerprints"] if isinstance(entry, dict)}
        # The row limit counts distinct neighbours, not duplicate citations.
        # A separate bounded witness check keeps any current shared revision,
        # rather than choosing an arbitrary MIN(ref_path) for the neighbour.
        for reference in references:
            expected = declared[reference]
            if peer.get(reference) != expected:
                continue
            witness = connection.execute(
                "SELECT 1 FROM document_links AS a JOIN document_links AS b ON b.ref_path = a.ref_path "
                "WHERE a.path = ? AND b.path = ? AND a.ref_path = ? "
                "AND a.ref_kind = 'source' AND b.ref_kind = 'source' LIMIT 1",
                (anchor["path"], item["path"], reference),
            ).fetchone()
            if witness is None:
                continue
            if reference not in current_digests:
                digest = None
                try:
                    # Check unresolved components too: fingerprint() resolves
                    # containment, but an in-project symlink can still name a key.
                    source = workspace_roots.roots(repository).project
                    linked = False
                    for component in Path(reference).parts:
                        source /= component
                        status = source.lstat()
                        if (stat.S_ISLNK(status.st_mode)
                                or getattr(status, "st_reparse_tag", 0) in workspace_roots.LINK_REPARSE_TAGS):
                            linked = True
                            break
                    if not linked and source.is_file() and source.stat().st_size <= GRAPH_SOURCE_BYTE_LIMIT:
                        digest = fingerprint(repository, reference)["sha256"]
                except (BrainError, OSError, ValueError):
                    pass
                current_digests[reference] = digest
            if current_digests[reference] == expected:
                candidates.append(item)
                break
    eligible, _ = _runtime_filter(repository, candidates, config)
    return eligible, len(paths)


def source_link_candidates(
    connection: sqlite3.Connection,
    repository: Path,
    anchors: list[dict[str, Any]],
    seen: set[str],
) -> list[dict[str, Any]]:
    """The same policy-checked expansion for lightweight lexical results.

    Those search results lack governed metadata. Rehydrate their first strong
    semantic matches before traversal; a cached row alone is never authority.
    """
    hydrated = []
    for anchor in [item for item in anchors if item.get("match", "covered") == "covered"][:GRAPH_ANCHOR_LIMIT]:
        row = connection.execute(
            """
            SELECT d.path, d.layer, d.kind, d.title,
                m.category, m.privacy, m.owner, m.authority, m.lifecycle,
                m.source_hash, m.record_id, m.conflicts, m.source_fingerprints,
                m.updated_at, m.confidence, m.attestation
            FROM documents AS d JOIN document_metadata AS m ON m.path = d.path
            WHERE d.path = ?
            """, (anchor["path"],),
        ).fetchone()
        if row is None or row["layer"] != "semantic":
            continue
        item = dict(row)
        item["conflicts"] = json.loads(item["conflicts"])
        item["source_fingerprints"] = json.loads(item["source_fingerprints"])
        hydrated.append(item)
    candidates, _ = _source_link_candidates(
        connection, repository, load_config(repository), hydrated, seen,
    )
    return [_public_item(item) for item in candidates]


def _has_incoming_conflict(connection: sqlite3.Connection, record_id: Optional[str]) -> bool:
    """One-way declarations are conflicts too; cached doubt withholds expansion.

    A refresh removes expired/deleted metadata. Until then a cached declaration
    is sufficient to hold the graph addition, without disclosing its partner.
    UUIDs are quoted complete JSON strings, not substring or wildcard matches.
    """
    return record_id is not None and connection.execute(
        "SELECT 1 FROM document_metadata WHERE conflicts != '[]' "
        "AND instr(conflicts, ?) > 0 LIMIT 1", ('"' + record_id + '"',),
    ).fetchone() is not None


def _source_link_candidates(
    connection: sqlite3.Connection,
    repository: Path,
    config: dict[str, Any],
    anchors: list[dict[str, Any]],
    seen: set[str],
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    """Bounded declared relations; every seed and neighbour is runtime-checked.

    Reverse citations and a shared current source revision form relations;
    candidates are never traversed again. Neither is a new graph authority.
    Withheld neighbours never become bridges; diagnostics contain only counts.
    A SQL row cap also bounds work when a source has thousands of citations.
    """
    stats: dict[str, Any] = {
        "anchors": 0, "examined": 0, "eligible": 0,
        "withheld": 0, "truncated": False,
    }
    candidates: list[dict[str, Any]] = []
    current_digests: dict[str, Optional[str]] = {}
    for anchor in anchors[:GRAPH_ANCHOR_LIMIT]:
        if anchor.get("conflicts") or _has_incoming_conflict(connection, anchor.get("record_id")):
            continue
        # Recheck at the traversal boundary, including edits since seed search.
        eligible, _ = _runtime_filter(repository, [dict(anchor)], config)
        if not eligible:
            continue
        remaining = GRAPH_ROW_LIMIT - stats["examined"]
        if remaining <= 0:
            stats["truncated"] = True
            break
        linked, withheld = linked_documents(
            connection, repository, config, anchor["path"], max_rows=remaining,
        )
        stats["anchors"] += 1
        count = len(linked) + len(withheld)
        stats["examined"] += count
        stats["withheld"] += len(withheld)
        # A full row allowance means the walk may be incomplete, even when
        # the last row happened to be the source's last citation.
        stats["truncated"] |= count == remaining
        remaining -= count
        if remaining > 0:
            shared, shared_rows = _shared_source_documents(
                connection, repository, config, anchor, remaining, current_digests,
            )
            linked.extend(shared)
            stats["examined"] += shared_rows
            stats["truncated"] |= shared_rows == remaining
        for item in linked:
            if (item["path"] in seen or item["layer"] != "semantic"
                    or item["category"] not in ("durable", "dynamic")
                    or item["conflicts"]
                    or _has_incoming_conflict(connection, item.get("record_id"))):
                # A graph-only conflict component may need more semantic
                # slots than remain. Until admission is component-aware, do
                # not introduce either side through automatic expansion.
                continue
            seen.add(item["path"])
            stats["eligible"] += 1
            if len(candidates) >= GRAPH_CANDIDATE_LIMIT:
                stats["truncated"] = True
                continue
            content = connection.execute(
                "SELECT content FROM documents WHERE path = ?", (item["path"],),
            ).fetchone()
            item["snippet"] = _body_snippet(str(content["content"]))
            item["match"] = None
            item["selection"] = "source-link"
            item["adjusted_score"] = 0.0
            item["rank"] = None
            item["estimated_tokens"] = _estimate_tokens(item["title"] + item["snippet"])
            candidates.append(item)
    return candidates, stats


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


def _phase_value(phases: Optional[dict[str, Any]], key: str) -> Optional[float]:
    """One index phase duration, or None when this turn did not index.

    Reporting 0.0 for a phase that never ran would claim an instantaneous
    index; null says the turn read an index somebody else built.
    """
    if not phases:
        return None
    value = phases.get(key)
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        return None
    return round(float(value), 6)


def local_manifest_retention(config: dict[str, Any]) -> int:
    """How many ignored local manifests to keep, from config, never below one.

    The depth of this window is the observation window for every measurement
    built on manifests, so it belongs in runtime configuration rather than in
    a module constant. It is floored at one because zero or a negative value
    would delete the manifest the current retrieval just wrote while the
    returned capsule still advertises its path.
    """
    value = config.get("local_manifest_retention", LOCAL_MANIFEST_RETENTION)
    try:
        return max(1, int(value))
    except (TypeError, ValueError):
        return LOCAL_MANIFEST_RETENTION


def refresh_health_retention(config: dict[str, Any]) -> int:
    """How many turn-health records to keep, from config, never below one.

    Beside `local_manifest_retention` and floored the same way, for the reason
    H1-05 recorded: the depth of an observation window is a runtime decision,
    not a module constant, and a zero would delete the record just written.
    """
    value = config.get("refresh_health_retention", REFRESH_HEALTH_RETENTION)
    try:
        return max(1, int(value))
    except (TypeError, ValueError):
        return REFRESH_HEALTH_RETENTION


def _prune_local_manifests(directory: Path, retention: int) -> None:
    """Bound ignored local manifests; the governed store has its own policy."""
    manifests = [path for path in directory.glob("*.json") if path.is_file()]
    if len(manifests) <= retention:
        return
    manifests.sort(key=lambda path: (path.stat().st_mtime_ns, path.name))
    for path in manifests[: len(manifests) - retention]:
        try:
            path.unlink()
        except OSError:
            continue


# Keys of a candidate that may reach the model. `match` is carried only when
# it is not `covered`: the capsule is zero-sum against CAPSULE_CHARACTER_LIMIT
# and every selected item is serialized up to three times, so a key that says
# "nothing unusual" would be paid for out of content on every turn. Absent
# therefore means covered, which is what the schema and the docs record.
_PUBLIC_ITEM_KEYS = (
    "path", "layer", "kind", "title", "snippet", "category",
    "estimated_tokens", "record_id", "conflicts",
)


def _claude_imports(repository: Path) -> set[str]:
    """Repository files CLAUDE.md pulls into Claude Code's context with `@path`.

    Followed the way Claude Code follows them: relative to the importing file,
    recursively up to CLAUDE_IMPORT_DEPTH hops, and never inside a fenced
    block or a code span, where `@` is only a character. A target outside the
    repository cannot be an indexed document, so it cannot take a slot either.
    """
    root = repository.resolve()
    loaded: set[str] = set()
    pending = [(root / name, 0) for name in CLAUDE_INSTRUCTION_FILES]
    while pending:
        path, depth = pending.pop()
        if depth >= CLAUDE_IMPORT_DEPTH:
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        fenced = False
        for line in text.splitlines():
            if CODE_FENCE_PATTERN.match(line):
                fenced = not fenced
                continue
            if fenced:
                continue
            for match in CLAUDE_IMPORT_PATTERN.finditer(CODE_SPAN_PATTERN.sub("", line)):
                target = (path.parent / match.group(1).rstrip(".,;:")).resolve()
                try:
                    relative = target.relative_to(root).as_posix()
                except ValueError:
                    continue
                if relative in loaded or not target.is_file():
                    continue
                loaded.add(relative)
                pending.append((target, depth + 1))
    return loaded


def host_loaded_paths(repository: Path, host: str) -> set[str]:
    """Repository paths the host has already put in front of the model."""
    project = workspace_roots.project_root(repository)
    loaded = set(HOST_LOADED_INSTRUCTIONS.get(host, ()))
    if host == "claude":
        if not any((project / name).is_file() for name in CLAUDE_INSTRUCTION_FILES):
            loaded.add("AGENTS.md")
        loaded |= _claude_imports(project)
    if workspace_roots.is_attached(repository):
        # The attaching launcher hands the accelerator's policy to every host
        # itself, so it never needs a capsule slot.
        loaded.add((workspace_roots.tooling_root(repository) / "AGENTS.md").as_posix())
    return loaded


def _public_item(item: dict[str, Any]) -> dict[str, Any]:
    public = {key: item[key] for key in _PUBLIC_ITEM_KEYS}
    if item.get("match") not in (None, "covered"):
        public["match"] = item["match"]
    if item.get("selection"):
        # Distinguish a declared relation from a lexical query match.
        public["selection"] = item["selection"]
    if item.get("source_changed"):
        # The cited files edited since this knowledge was verified: the
        # capsule says so next to it instead of silently dropping it.
        public["source_changed"] = list(item["source_changed"])
    if item.get("attestation") == "agent":
        # Checked only by the agent that wrote it: worth reading, and worth
        # checking before relying on it.
        public["attestation"] = "agent"
    return public


def retrieve(
    connection: sqlite3.Connection,
    repository: Path,
    query: str,
    task_identifier: Optional[str],
    *,
    limit: int,
    provider: Optional[str] = None,
    manifest_scope: str = "governed",
    query_source: str = "explicit",
    phase_seconds: Optional[dict[str, Any]] = None,
    gate_mode: str = RETRIEVAL_GATE_DEFAULT,
    paths: Optional[list[str]] = None,
    host: str = "cli",
    entry_point: str = "retrieve",
    local_episodes: Optional[list[dict[str, Any]]] = None,
    session_id: Optional[str] = None,
    transcript: Optional[str] = None,
    pack: Optional[Callable[[dict[str, Any]], dict[str, Any]]] = None,
) -> dict[str, Any]:
    """Assemble governed context and record the manifest that justifies it.

    ``query_source`` and ``phase_seconds`` are provenance the caller owns and
    this function cannot infer: whether the query came from a prompt, a task,
    a branch name or an operator, and how long the index phases that fed it
    took. They are written into the manifest because a retrieval decision
    cannot be reviewed later from the selection alone.

    ``task_identifier=None`` retrieves for a branch whose governed task does
    not exist yet. The task is provisioned at the first checkpoint, after
    several file-changing turns, and a read-only session never gets one; the
    capsule was empty for all of that time although the runtime filters do
    not depend on a task. Without a task there is no working state and no
    own record to exclude, and the manifest stays in ignored local state:
    governed history is kept per task.

    ``pack`` turns the result into the capsule as the caller delivers it -
    the layer and character limits of its JSON, the ceiling of its rendered
    text - and returns it. Only what it still shows counts as delivered: the
    conversation's repeat record, the manifest's selection and the token
    estimates are written after it, and what it left out is excluded as
    ``capsule-limit``. Without it the whole selection counts as delivered.
    """
    retrieval_started = time.monotonic()
    if limit < 1:
        raise RetrievalError("--limit must be a positive integer")
    if query_source not in QUERY_SOURCES:
        raise RetrievalError(
            f"Query source must be one of {', '.join(sorted(QUERY_SOURCES))}"
        )
    if manifest_scope not in MANIFEST_SCOPES:
        raise RetrievalError(
            f"Manifest scope must be one of {', '.join(MANIFEST_SCOPES)}"
        )
    if gate_mode not in RETRIEVAL_GATE_MODES:
        raise RetrievalError(
            f"Retrieval gate must be one of {', '.join(RETRIEVAL_GATE_MODES)}"
        )
    if host not in RETRIEVAL_HOSTS:
        raise RetrievalError(
            f"Retrieval host must be one of {', '.join(RETRIEVAL_HOSTS)}"
        )
    if entry_point not in RETRIEVAL_ENTRY_POINTS:
        raise RetrievalError(
            "Retrieval entry point must be one of "
            f"{', '.join(RETRIEVAL_ENTRY_POINTS)}"
        )
    if task_identifier is None:
        task_path, task = None, None
        manifest_scope = "local"
    else:
        task_path, task, _ = find_task(repository, task_identifier)
        validate_record(task)
    config = load_config(repository)
    local_episodes = list(local_episodes or [])
    candidates, diagnostics = _candidates(connection, query, max(20, limit * 10))
    # Taken before any filter runs, so "nothing matched" cannot be confused
    # with "everything that matched was withheld". Only the layers a governed
    # capsule fills from this call are answered for; the episodic layer is
    # assembled by the caller and reports itself.
    matched_layers = {item["layer"] for item in candidates}
    if local_episodes:
        matched_layers.add("episodic")
    no_match = [
        layer
        for layer in ("procedural", "semantic", "episodic")
        if layer not in matched_layers
    ]
    filtered, filter_excluded = _runtime_filter(repository, candidates, config)
    # A candidate whose cited file changed kept a reduced score; ranking it
    # again keeps fresh knowledge of equal fit ahead of it. Stable, so
    # nothing else moves.
    filtered.sort(key=lambda item: -float(item.get("adjusted_score") or 0.0))
    graph_anchors = [
        item for item in filtered
        if item["layer"] == "semantic" and item.get("match") == "covered"
        and (task is None or item.get("record_id") != task["id"])
    ]
    # Injected here and nowhere earlier. `matched_layers` and `no_match` above
    # are claims about the QUERY — the capsule's `no-match:` line and the
    # gate's `signals.no_match` both read them that way — and a path link is
    # not the query matching. A layer can therefore report `no-match` and
    # still deliver a path-linked document on the same turn: the first says
    # the caller's words found nothing there, the second says the caller's
    # path did.
    path_matched_count = 0
    if paths:
        linked, link_excluded = _path_candidates(
            connection,
            repository,
            config,
            paths,
            {item["path"] for item in filtered},
        )
        path_matched_count = len(linked) + len(link_excluded)
        filter_excluded.extend(link_excluded)
        # Ahead of the query's own matches, because `_apply_budgets` and the
        # per-layer ladder both honour input order and the caller named these.
        filtered = [*linked, *filtered]
    graph_candidates, graph_stats = _source_link_candidates(
        connection, repository, config, graph_anchors,
        {item["path"] for item in filtered},
    )
    # Related knowledge fills remaining capacity after direct matches and
    # explicit paths; provenance alone never gives it priority over them.
    filtered.extend(graph_candidates)
    # A promoted chunk and the record it was promoted from say the same thing,
    # and both used to take a slot. The chunk is the durable form, so the
    # record yields to it.
    promoted_from = {
        str(source.get("path"))
        for item in filtered
        if item.get("kind") == "memory" and item.get("selection") != "source-link"
        for source in item.get("source_fingerprints") or []
        if isinstance(source, dict)
    }
    if promoted_from:
        kept = []
        for item in filtered:
            if item.get("kind", "").startswith("brain-") and item["path"] in promoted_from:
                filter_excluded.append({"path": item["path"], "reason": "promoted-to-chunk"})
            else:
                kept.append(item)
        filtered = kept
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
    # The task's own record and handoff are the working state the capsule
    # already leads with, and a host-loaded instruction file is already in the
    # model's context: a slot spent pointing at either is taken from memory.
    # On real installations the task's own record held a semantic slot on
    # 13-20% of turns. They leave as named exclusions, after `no_match`, which
    # stays a claim about what the query matched rather than what survived.
    own_id = task["id"] if task is not None else None
    own_handoff = (
        handoff_path(repository, own_id).relative_to(repository).as_posix()
        if own_id is not None
        else None
    )
    loaded = host_loaded_paths(repository, host)
    kept = []
    for item in filtered:
        if own_id is not None and (
            item.get("record_id") == own_id or item["path"] == own_handoff
        ):
            filter_excluded.append({"path": item["path"], "reason": "working-task"})
        elif item["path"] in loaded:
            filter_excluded.append({"path": item["path"], "reason": "host-loaded"})
        else:
            kept.append(item)
    filtered = kept
    selected, budget_excluded, usage, escalation_reason = _apply_budgets(filtered)
    # Skills and policy, strong matches only: every host loads its own skill
    # catalogue and picks from it, agents opened none of 148 skill pointers
    # they were handed on real installations, and on 61 graded real prompts a
    # skill admitted on one rare word was useful once in 37 deliveries.
    procedural_ranked = [
        item
        for item in selected
        if item["category"] == "policy" and item.get("match") != "distinctive"
    ]
    # A skill's own sub-file at the head of that ranking does not hand its slot
    # down: the next skill is a weaker match for a request whose best
    # procedural match was a detail inside some skill (19 replayed turns:
    # refilling added 0 useful skills and 5 noise).
    head = procedural_ranked[:CAPSULE_PROCEDURAL_LIMIT]
    vacated_paths = {
        item["path"] for item in head
        if not procedural_slot_eligible(item["kind"], item["path"])
    }
    procedural_ranked = [item for item in head if item["path"] not in vacated_paths]
    # The semantic layer is built from categories and the episodic layer from
    # the layer column, and the two taxonomies overlap: `category_for` has no
    # `changelog` branch, so CHANGELOG.md is category 'evidence' AND layer
    # 'episodic'. Without this filter it takes one of the three semantic slots
    # and the single episodic slot at once, and the degradation ladder then
    # drops real content to stay inside the budget. Filtering here rather than
    # after the split matters twice over: the freed slot goes to the next
    # ranked candidate instead of being lost, and the manifest written below
    # keeps describing exactly what the capsule delivers.
    semantic_ranked = [
        item
        for item in selected
        if item["category"] != "policy" and item["layer"] != "episodic"
    ]
    # The episodic layer used to be assembled by the caller from a raw
    # `search_documents` call, which never joins `document_metadata` - so it
    # applied no privacy, owner, authority, lifecycle or freshness filter and
    # never appeared in the manifest. That was harmless while the only
    # episodic document was the changelog; it stops being harmless the moment
    # governed records live there. Ranking it here puts it through the same
    # filters, the same budget and the same audit record as everything else,
    # and makes H1-03's `episodic-layer` exclusion reason literally true.
    episodic_ranked = [item for item in selected if item["layer"] == "episodic"]
    capsule_selected = [
        *procedural_ranked[:CAPSULE_PROCEDURAL_LIMIT],
        *semantic_ranked[:CAPSULE_SEMANTIC_LIMIT],
        *episodic_ranked[:CAPSULE_EPISODIC_LIMIT],
    ]
    local_episode_selected = local_episodes[
        : max(0, CAPSULE_EPISODIC_LIMIT - len(episodic_ranked[:CAPSULE_EPISODIC_LIMIT]))
    ]
    local_episode_tokens = sum(
        _estimate_tokens(_serialized_episode(episode))
        for episode in local_episode_selected
    )
    selected_tokens = sum(item["estimated_tokens"] for item in capsule_selected)
    if local_episode_selected and selected_tokens + local_episode_tokens > TARGET_BUDGET:
        local_episode_selected = []
        budget_excluded.append({"path": "local-episode", "reason": "budget"})
    # The conversation already holds what it was handed in its last few turns:
    # the same item at the same revision is not handed again. Its slot is not
    # refilled - the next candidate down is weaker, and promoting it to fill
    # the gap turned a repeat into noise - and the capsule says how many
    # earlier items still apply.
    session = (
        session_id
        if isinstance(session_id, str) and SESSION_ID_PATTERN.match(session_id)
        else None
    )
    session_deliveries = _load_session_deliveries(connection) if session else {}
    session_turn = 0
    repeated: list[dict[str, Any]] = []
    repeated_episodes: list[dict[str, Any]] = []
    delivery_keys: dict[str, tuple[str, str]] = {}
    # What earlier turns recorded for the items left out as repeats: a repeat
    # keeps the turn it was first handed in.
    still_handed: dict[str, list[Any]] = {}
    transcript_position: Optional[dict[str, Any]] = None
    compacted = False
    if session:
        previous_entry = session_deliveries.get(session)
        previous_entry = previous_entry if isinstance(previous_entry, dict) else {}
        session_turn = int(previous_entry.get("turn") or 0) + 1
        recent = previous_entry.get("items")
        recent = recent if isinstance(recent, dict) else {}
        compacted, transcript_position = transcript_compacted(previous_entry, transcript)
        if compacted:
            recent = {}

        def handed(key: str, revision: str) -> bool:
            seen = recent.get(key)
            if (
                isinstance(seen, list)
                and len(seen) == 2
                and seen[0] == revision
                and isinstance(seen[1], int)
                and session_turn - seen[1] < SESSION_NOVELTY_TURNS
            ):
                still_handed[key] = seen
                return True
            return False

        fresh_selection = []
        for item in capsule_selected:
            key, revision = delivery_identity(connection, item, query)
            delivery_keys[item["path"]] = (key, revision)
            if handed(key, revision):
                repeated.append(item)
            else:
                fresh_selection.append(item)
        capsule_selected = fresh_selection
        # A recorded episode is held by the conversation like a document: by
        # its id at the revision of its content, so an edited one is new.
        fresh_episodes = []
        for episode in local_episode_selected:
            if handed(*_episode_signature(episode)):
                repeated_episodes.append(episode)
            else:
                fresh_episodes.append(episode)
        local_episode_selected = fresh_episodes
    capsule_paths = {item["path"] for item in capsule_selected}
    layer_excluded = [
        {
            "path": item["path"],
            # A document held back because its own layer will carry it is not
            # a document that ran out of room.
            "reason": (
                "episodic-layer" if item["layer"] == "episodic"
                else "skill-subfile" if item["path"] in vacated_paths
                else "layer-limit"
            ),
        }
        for item in selected
        if item["path"] not in capsule_paths
        and item["path"] not in {entry["path"] for entry in repeated}
    ]
    layer_excluded.extend(
        {"path": item["path"], "reason": "delivered-this-session"} for item in repeated
    )
    layer_excluded.extend(
        {"path": _episode_signature(episode)[0], "reason": "delivered-this-session"}
        for episode in repeated_episodes
    )
    selected = capsule_selected
    # Decided here rather than earlier because `selection_identical_to_
    # previous_turn` is a claim about what the capsule delivers, and after the
    # episodic-layer filter and the 2+3 truncation above that is only now
    # known. Deciding earlier would describe a set the capsule never carried.
    signature = _retrieval_signature(
        query,
        [
            *((item["path"], item["source_hash"]) for item in selected),
            *(_episode_signature(episode) for episode in local_episode_selected),
        ],
        task["revision"] if task is not None else 0,
    )
    baseline_key = _retrieval_baseline_key(
        own_id if own_id is not None else "no-task", host, entry_point
    )
    previous = _load_last_retrievals(connection).get(baseline_key)
    gate = gate_decision(
        gate_mode,
        signature=signature,
        previous=previous if isinstance(previous, dict) else None,
        diagnostics=diagnostics,
        no_match=no_match,
        matched_count=len(candidates) + len(local_episodes) + path_matched_count,
        selected_count=len(selected) + len(local_episode_selected),
        # The selection above already left out what this conversation holds.
        conversation=session is not None,
    )
    withheld = gate["mode"] == "enforce" and gate["decision"] == "skip"
    if withheld:
        # The gate saves the turn from a pointer, not the database from a
        # query: the work is already done and cost nothing extra. What is
        # withheld is the claim "this is relevant right now".
        selected = []
        local_episode_selected = []
        layer_excluded = []
    manifest_id = new_uuid()
    if manifest_scope == "governed":
        manifest_directory = brain_root(repository) / "control" / "retrieval-manifests"
    else:
        manifest_directory = repository / "memory-bank" / "local" / "retrieval-manifests"
    manifest_path = manifest_directory / f"{manifest_id}.json"
    groups = {category: [] for category in BUDGETS}
    for item in selected:
        public = _public_item(item)
        groups[item["category"]].append(public)
    procedural = groups["policy"][:CAPSULE_PROCEDURAL_LIMIT]
    episodic = [
        item
        for group in groups.values()
        for item in group
        if item["layer"] == "episodic"
    ][:CAPSULE_EPISODIC_LIMIT]
    episodic.extend(local_episode_selected[: CAPSULE_EPISODIC_LIMIT - len(episodic)])
    semantic = [
        *groups["handoff"], *groups["durable"], *groups["dynamic"], *groups["evidence"]
    ][:CAPSULE_SEMANTIC_LIMIT]
    # Category grouping must not move related knowledge ahead of direct
    # hits when the renderer spends its character allowance.
    semantic.sort(key=lambda item: item.get("selection") == "source-link")
    result: dict[str, Any] = {
        "query": query,
        "task_id": task["external_id"] if task is not None else None,
        "task_uuid": own_id,
        "task_revision": task["revision"] if task is not None else None,
        # Where the full working state lives. The capsule carries a bounded
        # projection of it, and the record no longer competes for a semantic
        # slot, so this is how an agent that needs the rest finds it.
        "task_record": (
            task_path.relative_to(repository).as_posix()
            if task_path is not None
            else None
        ),
        "working": None if task is None else {
            "task_id": task["external_id"], "goal": task["goal"],
            "phase": task.get("phase"),
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
        "episodic": episodic,
        "selected": [_public_item(item) for item in selected],
        # A layer with no candidate at all is a different fact from a layer
        # whose candidates were filtered out downstream, and only the first
        # one means "memory has nothing here". Recorded before any budget or
        # policy filter runs; `excluded` in the manifest explains the rest.
        "no_match": no_match,
        # Relevant items left out because this conversation was handed them
        # in its last few turns; the rendered capsule says they still apply.
        "repeated": len(repeated) + len(repeated_episodes),
        "gate": gate,
        "token_estimates": _token_usage(selected, local_episode_selected),
        "manifest": manifest_path.relative_to(repository).as_posix(),
        "manifest_scope": manifest_scope,
    }
    pack_seconds = 0.0
    if pack is not None:
        # The capsule as the host gets it: the limits of its JSON and of its
        # rendered text leave out what does not fit, and what they leave out
        # was not handed. Recorded as handed, it was suppressed on the next
        # turns as an item that "still applies" although the conversation
        # never saw it; so the conversation's record, the manifest and its
        # token estimates are written from what the capsule shows.
        pack_started = time.monotonic()
        result = pack(result)
        pack_seconds = time.monotonic() - pack_started
        shown_paths, shown_episodes = _shown_items(result)
        left_out = [item for item in selected if item["path"] not in shown_paths]
        left_out_episodes = [
            episode
            for episode in local_episode_selected
            if episode.get("id") not in shown_episodes
        ]
        selected = [item for item in selected if item["path"] in shown_paths]
        local_episode_selected = [
            episode
            for episode in local_episode_selected
            if episode.get("id") in shown_episodes
        ]
        layer_excluded.extend(
            {"path": item["path"], "reason": "capsule-limit"} for item in left_out
        )
        layer_excluded.extend(
            {"path": _episode_signature(episode)[0], "reason": "capsule-limit"}
            for episode in left_out_episodes
        )
    usage = _token_usage(selected, local_episode_selected)
    result["token_estimates"] = usage
    if not withheld:
        # Remembered only for a turn that actually delivered, so the next turn
        # compares against the last real retrieval rather than against a skip.
        _remember_retrieval(connection, baseline_key, signature)
    if session:
        # A repeat keeps the turn it was first handed in: once that is
        # SESSION_NOVELTY_TURNS behind, it is handed again, which is what a
        # conversation compacted in the meantime needs.
        handed_now = {
            delivery_keys[item["path"]][0]: [delivery_keys[item["path"]][1], session_turn]
            for item in selected
            if item["path"] in delivery_keys
        }
        for episode in local_episode_selected:
            key, revision = _episode_signature(episode)
            handed_now[key] = [revision, session_turn]
        _remember_session_deliveries(
            connection,
            session,
            session_turn,
            {**still_handed, **handed_now},
            session_deliveries,
            transcript_position,
            reset=compacted,
        )
    manifest = {
        "schema_version": MANIFEST_SCHEMA_VERSION,
        "id": manifest_id,
        "created_at": utc_now(),
        "query": query,
        "task_id": own_id,
        "task_revision": task["revision"] if task is not None else None,
        "local_episode_count": len(local_episode_selected),
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
                # `match` describes how the query matched, so an item the
                # query never matched carries none rather than a fourth
                # reading of a lexical word. Why it is here is a separate
                # question with a separate key.
                **({"match": item["match"]} if item.get("match") else {}),
                **(
                    {"selection": item["selection"]}
                    if item.get("selection")
                    else {}
                ),
                **({"source_changed": True} if item.get("source_changed") else {}),
                # Everything a later gate needs to reason about this turn:
                # how strongly it matched, and where it stood before any
                # budget or layer limit applied.
                "score": round(float(item.get("adjusted_score") or 0.0), 6),
                "rank": item.get("rank"),
            }
            for item in selected
        ],
        "excluded": [*filter_excluded, *budget_excluded, *layer_excluded],
        "token_estimates": usage,
        "provider": provider or config["provider"],
        "source_links": {
            # Raw row/withheld counts would reveal protected citations. Only
            # allowed seeds and the bounded, allowed proposal are public.
            "anchors": graph_stats["anchors"],
            "candidates": len(graph_candidates),
            "delivered": sum(item.get("selection") == "source-link" for item in selected),
        },
        "escalation_reason": escalation_reason,
        # Where the query came from, and what the turn spent getting here.
        # A manifest that records only the selection cannot answer whether a
        # bad retrieval was a bad query or a bad ranking.
        "query_source": query_source,
        "host": host,
        "entry_point": entry_point,
        "gate": gate,
        "phase_seconds": {
            "stat": _phase_value(phase_seconds, "stat"),
            "index": _phase_value(phase_seconds, "index"),
            # The body of retrieve(); packing the capsule is the caller's.
            "retrieval": round(
                max(0.0, time.monotonic() - retrieval_started - pack_seconds), 6
            ),
        },
    }
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
        _prune_local_manifests(
            manifest_directory, local_manifest_retention(config)
        )
    return result
