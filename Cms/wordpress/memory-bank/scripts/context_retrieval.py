#!/usr/bin/env python3
"""Governed FTS5 indexing, budgeting, and retrieval manifests."""

from __future__ import annotations

import copy
import contextlib
import threading
import hashlib
from bisect import bisect_left, bisect_right
import json
import math
import os
import posixpath
import re
import sqlite3
import stat
import subprocess
import sys
import time
import unicodedata
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Iterable, Optional

import workspace_roots
import brain_runtime
from brain_runtime import (
    BrainError,
    BrainRecordReadError,
    _CitationDigestCache,
    LIFECYCLES,
    atomic_json,
    brain_root,
    find_task,
    fingerprint,
    handoff_path,
    iter_records,
    is_uuid4,
    load_config,
    read_runtime_config_text,
    mutation_lock,
    new_uuid,
    parse_markdown_record,
    read_record_text,
    _require_utf8_record_path,
    record_attestation,
    record_is_eligible,
    render_current_state,
    source_changes,
    utc_now,
    validate_handoff,
    validate_record,
    validate_schema_file,
)
from automatic_query import SECRET_PATTERNS, source_path_problem
from validate import indexed_secret_policy_reason, memory_temporal_reason, secret_policy_fingerprint


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
# The one skill or instruction file ranking picks per turn. The capsule does
# not carry it (see `retrieve`): the manifest records the pick as
# `host-listed`, so routing stays measurable.
CAPSULE_PROCEDURAL_LIMIT = 1
CAPSULE_SEMANTIC_LIMIT = 3
# Repository history the main query ranks: the changelog.
CAPSULE_EPISODIC_LIMIT = 1
# Recorded history: up to two Project Brain events (or local episodes) found
# by their own search (history_query). The second event is admitted only when
# the request names a ticket and both events carry it; otherwise the slot keeps
# its single best candidate. It used to share the changelog's slot entirely,
# which the changelog held on every turn of the 121 evaluated prompts that had
# an event - while 19 answers sat in events never delivered.
CAPSULE_EVENT_LIMIT = 2
HISTORY_KIND = "brain-event"
# How many prompt terms survive into a retrieval query. Rarity in the index
# decides which ones, so the cap bounds cost without deciding relevance by
# position the way the old first-N-words hook extraction did.
CAPSULE_PROMPT_TERM_LIMIT = 24
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

# loop-detection.sh header: the Claude copy is wired on PostToolUse with an
# edit-tool matcher, the Codex copy runs on every PostToolUse and filters
# itself, the Cursor copy runs on afterFileEdit - and only Cursor still warns
# with exit 1 (see _LOOP_WARNING below).
_LOOP_HEADER = (
    "# Hook type: PostToolUse (Edit|Write|MultiEdit|NotebookEdit)\n"
    "# Exit codes: 0 = pass (a warning travels as JSON additionalContext),\n"
    "# 2 = block\n"
)
_LOOP_HEADER_CURSOR = (
    "# Cursor hook event: afterFileEdit.\n"
    "# Exit codes: 0 = pass, 1 = warn (continue), 2 = block\n"
)
_LOOP_HEADER_CODEX = (
    "# Codex hook event: PostToolUse (self-filters to file-edit payloads).\n"
    "# Exit codes: 0 = pass (a warning travels as JSON additionalContext),\n"
    "# 2 = block\n"
)

# Warnings that reach the agent. Claude Code and Codex add a hook's
# hookSpecificOutput.additionalContext to the model's context next to the tool
# result, on PreToolUse and PostToolUse alike, so the canonical edit-loop
# warning (loop-detection.sh) and repetition warning (bash-validator.sh) exit
# 0 with that JSON on stdout. Cursor documents no such channel for an edit it
# reports (afterFileEdit) or a shell command it lets run
# (beforeShellExecution's agent_message accompanies a denial), so its mirrors
# keep the user-visible form - text and exit 1, which Cursor shows in its
# hooks output and otherwise ignores - and the agent there meets each guard
# first at its block.
_LOOP_WARNING = r'''if [ -n "$WARNING" ]; then
  # Claude Code and Codex hand a PostToolUse hook's additionalContext to the
  # model next to the tool result. Text with a non-blocking exit code reaches
  # only the user's transcript, so the agent would never see its warning.
  printf '{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"%s"}}\n' "$WARNING"
fi
'''
_LOOP_WARNING_CURSOR = r'''if [ -n "$WARNING" ]; then
  # Cursor's afterFileEdit has no channel back to the agent: the warning
  # reaches only the hooks output, where the user sees it.
  printf '%s\n' "$WARNING"
  exit 1
fi
'''
_GUARD_WARNING = r'''  # Claude Code and Codex add a PreToolUse hook's additionalContext to the
  # model's context next to the tool result, without blocking the call or
  # touching its permission decision. A warning on stderr with a non-blocking
  # exit code reaches only the user's transcript: the agent would meet the
  # guard first at the block.
  printf '{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"%s"}}\n' "$BV_WARNING"
'''
_GUARD_WARNING_CURSOR = r'''  # Cursor documents no channel that shows the agent a message about a shell
  # command it lets run (beforeShellExecution's agent_message accompanies a
  # denial), so here the warning reaches only the user, in the hooks output,
  # and the agent meets the guard first at the block.
  printf '%s\n' "$BV_WARNING" >&2
  exit 1
'''

# Cursor's beforeSubmitPrompt cannot inject context through hook output.
# Native hooks therefore maintain an ignored alwaysApply rule, while a
# Harness capsules retire only the registered generated slot.
# Exact managed bytes and whole historical envelopes recover automatically;
# custom deviations/tracked/foreign-linked files are preserved. Full body
# equality avoids gratuitous rewrites without hiding changed excerpts.
# Loading order remains a native-client measurement, not a hook guarantee.
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
_WM_DELIVERY_STOP_CURSOR = r'''# Cursor keeps working-memory.mdc, a generated alwaysApply rule, for direct-host delivery.
# Attached rules belong to the shared clone and are never maintained here.
[ "$STATE_DIR" = "$ROOT_DIR" ] || exit 0
cursor_rule_run() {
  local CURSOR_MAINTENANCE_STATUS=0 CURSOR_MAINTENANCE_REPORT CURSOR_PROJECTION_STATUS=0
  if command -v timeout >/dev/null 2>&1; then
    CURSOR_MAINTENANCE_REPORT=$(timeout 3 "${PYTHON[@]}" "$CONTEXT_CLI" "$@" --diagnostic 2>/dev/null) || CURSOR_MAINTENANCE_STATUS=$?
  else
    CURSOR_MAINTENANCE_REPORT=$("${PYTHON[@]}" "$CONTEXT_CLI" "$@" --diagnostic 2>/dev/null) || CURSOR_MAINTENANCE_STATUS=$?
  fi
  if [ "$CURSOR_MAINTENANCE_STATUS" -ne 0 ]; then
    printf '%s\n' 'Cursor memory rule was not updated: maintenance_unavailable.' >&2
    return "$CURSOR_MAINTENANCE_STATUS"
  fi
  # 4096 limits projection admission, not the shell capture of trusted CLI
  # stdout. Both subprocess stderr streams are discarded before fixed reports.
  printf '%s' "$CURSOR_MAINTENANCE_REPORT" | "${PYTHON[@]}" -I -S -c '
import json,sys
try:
    raw=sys.stdin.buffer.read(4097)
    if len(raw)>4096: raise ValueError("bounded outcome required")
    value=json.loads(raw)
    state=value.get("state") if isinstance(value,dict) else None
    if state not in ("written","unchanged","kept","removed","absent","attached","preserved","incomplete"):
        raise ValueError("known state required")
    print(json.dumps({"state":state}))
    sys.exit(5 if state=="preserved" and value.get("reason")=="linked_directory"
             else 3 if state=="preserved" else 4 if state=="incomplete" else 0)
except (ValueError,TypeError,RecursionError):
    sys.exit(1)
' 2>/dev/null || CURSOR_PROJECTION_STATUS=$?
  case "$CURSOR_PROJECTION_STATUS" in
    0) return 0 ;;
    3) printf '%s\n' 'Cursor memory rule was not updated: custom_or_unavailable.' >&2 ;;
    5) printf '%s\n' 'Cursor memory rule was not updated: linked_directory.' >&2 ;;
    4) printf '%s\n' 'Cursor memory rule was not updated: maintenance_incomplete.' >&2 ;;
    *) printf '%s\n' 'Cursor memory rule was not updated: maintenance_unavailable.' >&2; return 1 ;;
  esac
  return 0
}
if [ "${CONTEXT_CAPSULE_DELIVERED:-}" = "1" ]; then
  cursor_rule_run cursor-rule-retire >/dev/null || true
  exit 0
fi
# Preserve this task's proven prompt rule after checkpointing. Task alias is
# not chat ownership or current canonical-source authority. This only asks:
# "preserved" means another rule is there, which the write below replaces, so
# it reports nothing - the write, keep or retire that follows says what
# happened to the rule.
HELD_RULE=$(cursor_rule_run cursor-rule --action prompt-held --task-id "$TASK_ID" 2>/dev/null || true)
case "$HELD_RULE" in *'"state": "kept"'*) exit 0 ;; esac
CAPSULE_IDENTITY_ARGUMENTS=()
if [ -z "${CONTEXT_TASK_ID:-}" ] && [ -n "$TASK_ID" ]; then
  CAPSULE_IDENTITY_ARGUMENTS=(--expected-branch "$TASK_ID")
fi
CAPSULE_STATUS=1
if command -v timeout > /dev/null 2>&1; then
  CAPSULE=$(timeout "$BUDGET_SECONDS" "${PYTHON[@]}" "$CONTEXT_CLI" hook-context --host cursor --delivery-profile cursor-rule --task-id "$TASK_ID" "${CAPSULE_IDENTITY_ARGUMENTS[@]}" 2>/dev/null)
  CAPSULE_STATUS=$?
else
  CAPSULE=$("${PYTHON[@]}" "$CONTEXT_CLI" hook-context --host cursor --delivery-profile cursor-rule --task-id "$TASK_ID" "${CAPSULE_IDENTITY_ARGUMENTS[@]}" 2>/dev/null)
  CAPSULE_STATUS=$?
fi
if [ "$CAPSULE_STATUS" -eq 3 ]; then
  cursor_rule_run cursor-rule-retire >/dev/null || true
elif [ "$CAPSULE_STATUS" -eq 0 ]; then
  printf '%s' "$CAPSULE" | cursor_rule_run cursor-rule --action write --phase turn --task-id "$TASK_ID" >/dev/null || true
else
  cursor_rule_run cursor-rule --action keep --task-id "$TASK_ID" >/dev/null || true
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
_WM_PROMPT_SESSION_CURSOR = r'''# The rule is re-sent whole with every request, so nothing may be left out
# of it as "handed earlier": no --session-id. JSON, even without a query.
ARGUMENTS+=(--json)
'''
_WM_DELIVERY_PROMPT = r'''if [ -z "$REPORT" ]; then
  # A refresh that succeeded with nothing to say - a prompt the sanitizer
  # left nothing of - stays silent. One that failed says so, and says what
  # follows from it, instead of leaving the turn to assume memory was read.
  [ "$HOOK_STATUS" -eq 0 ] && exit 0
  if [ "$HOOK_STATUS" -eq 124 ]; then
    echo "Memory refresh unavailable: it exceeded its ${BUDGET_SECONDS}s budget."
    # The first index of a large project can take longer than a prompt's
    # budget, and an index stopped there starts over on the next prompt.
    if [ "$STATE_DIR" = "$ROOT_DIR" ]; then
      echo "If this repeats on every prompt, build the index once outside the hook: ${PYTHON[*]} memory-bank/scripts/context.py index"
    fi
  else
    echo "Memory refresh unavailable: it exited $HOOK_STATUS${DETAIL:+ — $DETAIL}."
  fi
  echo "Working memory was NOT consulted this turn. Read the canonical sources directly."
  exit 0
fi

# The capsule names itself: its memory section says the text is reference
# data to check against the cited file, so no banner precedes it.
echo "$REPORT"
'''
_WM_DELIVERY_PROMPT_CURSOR = r'''# All rule publication/removal uses native registered-slot maintenance.
# Custom deviations, tracked files and foreign links remain untouched.
[ "$STATE_DIR" = "$ROOT_DIR" ] || exit 0
cursor_rule_run() {
  local CURSOR_MAINTENANCE_STATUS=0 CURSOR_MAINTENANCE_REPORT CURSOR_PROJECTION_STATUS=0
  if command -v timeout >/dev/null 2>&1; then
    CURSOR_MAINTENANCE_REPORT=$(timeout 3 "${PYTHON[@]}" "$CONTEXT_CLI" "$@" --diagnostic 2>/dev/null) || CURSOR_MAINTENANCE_STATUS=$?
  else
    CURSOR_MAINTENANCE_REPORT=$("${PYTHON[@]}" "$CONTEXT_CLI" "$@" --diagnostic 2>/dev/null) || CURSOR_MAINTENANCE_STATUS=$?
  fi
  if [ "$CURSOR_MAINTENANCE_STATUS" -ne 0 ]; then
    printf '%s\n' 'Cursor memory rule was not updated: maintenance_unavailable.' >&2
    return "$CURSOR_MAINTENANCE_STATUS"
  fi
  # 4096 limits projection admission, not the shell capture of trusted CLI
  # stdout. Both subprocess stderr streams are discarded before fixed reports.
  printf '%s' "$CURSOR_MAINTENANCE_REPORT" | "${PYTHON[@]}" -I -S -c '
import json,sys
try:
    raw=sys.stdin.buffer.read(4097)
    if len(raw)>4096: raise ValueError("bounded outcome required")
    value=json.loads(raw)
    state=value.get("state") if isinstance(value,dict) else None
    if state not in ("written","unchanged","kept","removed","absent","attached","preserved","incomplete"):
        raise ValueError("known state required")
    print(json.dumps({"state":state}))
    sys.exit(5 if state=="preserved" and value.get("reason")=="linked_directory"
             else 3 if state=="preserved" else 4 if state=="incomplete" else 0)
except (ValueError,TypeError,RecursionError):
    sys.exit(1)
' 2>/dev/null || CURSOR_PROJECTION_STATUS=$?
  case "$CURSOR_PROJECTION_STATUS" in
    0) return 0 ;;
    3) printf '%s\n' 'Cursor memory rule was not updated: custom_or_unavailable.' >&2 ;;
    5) printf '%s\n' 'Cursor memory rule was not updated: linked_directory.' >&2 ;;
    4) printf '%s\n' 'Cursor memory rule was not updated: maintenance_incomplete.' >&2 ;;
    *) printf '%s\n' 'Cursor memory rule was not updated: maintenance_unavailable.' >&2; return 1 ;;
  esac
  return 0
}
printf '%s' "$REPORT" | cursor_rule_run cursor-rule --action write --phase prompt --task-id "$TASK_ID" --report-json >/dev/null || true
'''
# Cursor still acknowledges the prompt exactly once on the early exit.
_WM_HOST_DELIVERED = '[ "${CONTEXT_CAPSULE_DELIVERED:-}" = "1" ] && exit 0\n'
_WM_HOST_DELIVERED_CURSOR = r'''if [ "${CONTEXT_CAPSULE_DELIVERED:-}" = "1" ]; then
  ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
  # The CLI returns before SQLite/attached-state setup. Unknown custody keeps
  # the rule. Harness also attempts retirement before the Cursor process starts.
  if resolve_python; then
    CONTEXT_CLI="$ROOT_DIR/memory-bank/scripts/context.py"
cursor_rule_run() {
  local CURSOR_MAINTENANCE_STATUS=0 CURSOR_MAINTENANCE_REPORT CURSOR_PROJECTION_STATUS=0
  if command -v timeout >/dev/null 2>&1; then
    CURSOR_MAINTENANCE_REPORT=$(timeout 3 "${PYTHON[@]}" "$CONTEXT_CLI" "$@" --diagnostic 2>/dev/null) || CURSOR_MAINTENANCE_STATUS=$?
  else
    CURSOR_MAINTENANCE_REPORT=$("${PYTHON[@]}" "$CONTEXT_CLI" "$@" --diagnostic 2>/dev/null) || CURSOR_MAINTENANCE_STATUS=$?
  fi
  if [ "$CURSOR_MAINTENANCE_STATUS" -ne 0 ]; then
    printf '%s\n' 'Cursor memory rule was not updated: maintenance_unavailable.' >&2
    return "$CURSOR_MAINTENANCE_STATUS"
  fi
  # 4096 limits projection admission, not the shell capture of trusted CLI
  # stdout. Both subprocess stderr streams are discarded before fixed reports.
  printf '%s' "$CURSOR_MAINTENANCE_REPORT" | "${PYTHON[@]}" -I -S -c '
import json,sys
try:
    raw=sys.stdin.buffer.read(4097)
    if len(raw)>4096: raise ValueError("bounded outcome required")
    value=json.loads(raw)
    state=value.get("state") if isinstance(value,dict) else None
    if state not in ("written","unchanged","kept","removed","absent","attached","preserved","incomplete"):
        raise ValueError("known state required")
    print(json.dumps({"state":state}))
    sys.exit(5 if state=="preserved" and value.get("reason")=="linked_directory"
             else 3 if state=="preserved" else 4 if state=="incomplete" else 0)
except (ValueError,TypeError,RecursionError):
    sys.exit(1)
' 2>/dev/null || CURSOR_PROJECTION_STATUS=$?
  case "$CURSOR_PROJECTION_STATUS" in
    0) return 0 ;;
    3) printf '%s\n' 'Cursor memory rule was not updated: custom_or_unavailable.' >&2 ;;
    5) printf '%s\n' 'Cursor memory rule was not updated: linked_directory.' >&2 ;;
    4) printf '%s\n' 'Cursor memory rule was not updated: maintenance_incomplete.' >&2 ;;
    *) printf '%s\n' 'Cursor memory rule was not updated: maintenance_unavailable.' >&2; return 1 ;;
  esac
  return 0
}
    cursor_rule_run cursor-rule-retire >/dev/null || true
  fi
  exit 0
fi
'''
_WM_DELIVERY_SESSION = r'''# Capsule delivery: this client receives the Task Capsule at prompt time
# through working-memory-read.sh; session start reports metadata only.
'''
_WM_DELIVERY_SESSION_CURSOR = r'''# Cursor's direct-host channel is the owned local working-memory.mdc rule.
[ "$STATE_DIR" = "$ROOT_DIR" ] || exit 0
cursor_rule_run() {
  local CURSOR_MAINTENANCE_STATUS=0 CURSOR_MAINTENANCE_REPORT CURSOR_PROJECTION_STATUS=0
  if command -v timeout >/dev/null 2>&1; then
    CURSOR_MAINTENANCE_REPORT=$(timeout 3 "${PYTHON[@]}" "$CONTEXT_CLI" "$@" --diagnostic 2>/dev/null) || CURSOR_MAINTENANCE_STATUS=$?
  else
    CURSOR_MAINTENANCE_REPORT=$("${PYTHON[@]}" "$CONTEXT_CLI" "$@" --diagnostic 2>/dev/null) || CURSOR_MAINTENANCE_STATUS=$?
  fi
  if [ "$CURSOR_MAINTENANCE_STATUS" -ne 0 ]; then
    printf '%s\n' 'Cursor memory rule was not updated: maintenance_unavailable.' >&2
    return "$CURSOR_MAINTENANCE_STATUS"
  fi
  # 4096 limits projection admission, not the shell capture of trusted CLI
  # stdout. Both subprocess stderr streams are discarded before fixed reports.
  printf '%s' "$CURSOR_MAINTENANCE_REPORT" | "${PYTHON[@]}" -I -S -c '
import json,sys
try:
    raw=sys.stdin.buffer.read(4097)
    if len(raw)>4096: raise ValueError("bounded outcome required")
    value=json.loads(raw)
    state=value.get("state") if isinstance(value,dict) else None
    if state not in ("written","unchanged","kept","removed","absent","attached","preserved","incomplete"):
        raise ValueError("known state required")
    print(json.dumps({"state":state}))
    sys.exit(5 if state=="preserved" and value.get("reason")=="linked_directory"
             else 3 if state=="preserved" else 4 if state=="incomplete" else 0)
except (ValueError,TypeError,RecursionError):
    sys.exit(1)
' 2>/dev/null || CURSOR_PROJECTION_STATUS=$?
  case "$CURSOR_PROJECTION_STATUS" in
    0) return 0 ;;
    3) printf '%s\n' 'Cursor memory rule was not updated: custom_or_unavailable.' >&2 ;;
    5) printf '%s\n' 'Cursor memory rule was not updated: linked_directory.' >&2 ;;
    4) printf '%s\n' 'Cursor memory rule was not updated: maintenance_incomplete.' >&2 ;;
    *) printf '%s\n' 'Cursor memory rule was not updated: maintenance_unavailable.' >&2; return 1 ;;
  esac
  return 0
}
if [ "${CONTEXT_CAPSULE_DELIVERED:-}" = "1" ]; then
  cursor_rule_run cursor-rule-retire >/dev/null || true
  exit 0
fi
CAPSULE_BUDGET_SECONDS="${CONTEXT_HOOK_BUDGET:-5}"
CAPSULE_TASK_ID="${CONTEXT_TASK_ID:-$(git -C "$PROJECT_DIR" branch --show-current 2>/dev/null)}"
CAPSULE_IDENTITY_ARGUMENTS=()
if [ -z "${CONTEXT_TASK_ID:-}" ] && [ -n "$CAPSULE_TASK_ID" ]; then
  CAPSULE_IDENTITY_ARGUMENTS=(--expected-branch "$CAPSULE_TASK_ID")
fi
CAPSULE_STATUS=3
if [ -n "${PYTHON[0]:-}" ] && [ -f "$CONTEXT_CLI" ] && [ -n "$CAPSULE_TASK_ID" ]; then
  if command -v timeout >/dev/null 2>&1; then
    CAPSULE=$(timeout "$CAPSULE_BUDGET_SECONDS" "${PYTHON[@]}" "$CONTEXT_CLI" hook-context --host cursor --delivery-profile cursor-rule --task-id "$CAPSULE_TASK_ID" "${CAPSULE_IDENTITY_ARGUMENTS[@]}" 2>/dev/null)
    CAPSULE_STATUS=$?
  else
    CAPSULE=$("${PYTHON[@]}" "$CONTEXT_CLI" hook-context --host cursor --delivery-profile cursor-rule --task-id "$CAPSULE_TASK_ID" "${CAPSULE_IDENTITY_ARGUMENTS[@]}" 2>/dev/null)
    CAPSULE_STATUS=$?
  fi
fi
if [ "$CAPSULE_STATUS" -eq 3 ]; then
  cursor_rule_run cursor-rule-retire >/dev/null || true
elif [ "$CAPSULE_STATUS" -eq 0 ]; then
  printf '%s' "$CAPSULE" | cursor_rule_run cursor-rule --action write --phase session --task-id "$CAPSULE_TASK_ID" >/dev/null || true
else
  cursor_rule_run cursor-rule --action keep --task-id "$CAPSULE_TASK_ID" >/dev/null || true
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
                        [_LOOP_HEADER, _LOOP_HEADER_CURSOR],
                        [_HOOK_PATH_EXTRACT, _HOOK_PATH_EXTRACT_CURSOR],
                        [
                            "/claude-loop-detection-",
                            "/cursor-loop-detection-",
                        ],
                        # The warnings stay user-visible on Cursor (see
                        # _LOOP_WARNING above).
                        [_LOOP_WARNING, _LOOP_WARNING_CURSOR],
                        [_GUARD_WARNING, _GUARD_WARNING_CURSOR],
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
                        ["ARGUMENTS=(refresh --host cursor", "ARGUMENTS=(refresh --host cursor --delivery-profile cursor-rule"],
                        [_WM_PROMPT_PREAMBLE, _WM_PROMPT_PREAMBLE_CURSOR],
                        [_WM_PROMPT_SESSION + "fi\n", "fi\n" + _WM_PROMPT_SESSION_CURSOR],
                        [_WM_DELIVERY_PROMPT, _WM_DELIVERY_PROMPT_CURSOR],
                        [_WM_HOST_DELIVERED, _WM_HOST_DELIVERED_CURSOR],
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
                        [_LOOP_HEADER, _LOOP_HEADER_CODEX],
                        [_HOOK_PATH_EXTRACT, _HOOK_PATH_EXTRACT_CODEX],
                        [
                            "/claude-loop-detection-",
                            "/codex-loop-detection-",
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
                        # bash-validator's repetition guard names the same
                        # escalation.
                        [
                            "escalate to /debugger for a root cause.",
                            "escalate to systematic-debugger for a root cause.",
                        ],
                        [
                            "If it keeps failing, /debugger instead",
                            "If it keeps failing, systematic-debugger instead",
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
QUERY_SOURCES = ("prompt", "task", "task-id", "explicit", "continuation")
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
CLAUDE_IMPORT_PATTERN = re.compile(r"(?<![\w@])@((?:\.{1,2}/)?[\w-][\w\u0300-\u036f./#+-]*)")
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
# File edges. An active Brain task's files[] become `document_links` rows of
# kind `file` - "this task touched that file", never "cites" - and only the
# automatic seed channel reads them: files named in the request, and files the
# current task touched. One such item per capsule, after strong matches.
FILE_LINK_LIMIT = 1
# A file linked from more than five documents is a hub (a root config, a base
# class) and links nothing. On two real installations the measured fan-in had
# a gap between 4 and 7, and the paths at 7 or more were root docs and config;
# Aider's repository map damps identifiers defined in more than five files.
FILE_LINK_HUB_LIMIT = 5
# The newest files[] entries of a task that become rows (a task held 27 at most).
FILE_LINK_RECORD_LIMIT = 50
FILE_LINK_POOL = 8
PROMPT_SEED_LIMIT = 6
TOUCH_SEED_LIMIT = 8
PROMPT_SEED_SCAN_CHARS = 16384
AUTOMATIC_LINK_SELECTIONS = ("prompt-link", "touch-link")
LINK_KINDS = ("source", "file")
FILE_LINK_TRACKED_KEY = "file-link-tracked"
FILE_LINK_STATE_PREFIXES = ("project-brain/", "memory-bank/")
FILE_LINK_EXCLUDED_PARTS = frozenset({
    "var", "node_modules", "vendor", ".phpunit.cache", "coverage", ".idea", ".vscode",
    ".devcontainer", "__pycache__",
})
FILE_LINK_EXCLUDED_PREFIXES = (
    "public/build/", "public/bundles/", "public/hot", "bootstrap/cache/", "storage/framework/",
    "storage/logs/",
)
FILE_LINK_EXCLUDED_NAMES = frozenset({
    "composer.lock", "package-lock.json", "yarn.lock", "pnpm-lock.yaml", "symfony.lock",
    ".phpunit.result.cache",
})
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
# How many documents a name-shaped term may appear in and still anchor a
# distinctive match. `EventSubscriber` in five documents is a directory many
# unrelated records cite, not a name; a method or class name shared by one to
# three documents still names the subject. A term carrying a digit (a ticket,
# version or record number) anchors regardless: it names one thing even when
# several records cite it.
ANCHOR_DOCUMENT_LIMIT = 3
MAPPED_COMMIT_PATTERN = re.compile(r"^mapped_commit:\s*([0-9a-fA-F]{7,40})\s*$", re.M)
MAPPED_SCOPE_PATTERN = re.compile(r"^mapped_scope:\s*(\S+)\s*$", re.M)
CODEBASE_MAP_MAX_DRIFT = 25
LOCAL_MANIFEST_RETENTION = 200
DELETE_CHUNK = 500
INDEX_CONFIG_KEY = "config-fingerprint"
INDEX_SKILL_KEY = "skill-tree-fingerprint"
INDEX_SECRET_POLICY_KEY = "secret-policy-fingerprint"
# The instruction patterns an auto-promoted chunk is screened against: an
# index that admitted a chunk under other patterns has not screened it.
INDEX_PROVENANCE_POLICY_KEY = "provenance-policy-fingerprint"
INDEX_PARITY_KEY = "skill-parity-drift"
# A fresh value is stamped whenever index_documents rewrites the tables, so
# anything derived from index content (the token-frequency cache below) can
# tell at a glance whether it still describes the current index.
INDEX_GENERATION_KEY = "index-generation"
TOKEN_FREQUENCY_KEY = "token-frequency-cache"
# The digest of the Brain rows the tables hold. Brain rows are rebuilt from
# the records on every refresh; when they come out the same, the tables are
# not written and the generation stands.
INDEX_BRAIN_ROWS_KEY = "brain-rows-digest"
# Task IDs that more than one active task record claims, each with its
# records, earliest created first: two copies of a branch that each started
# its task and were merged. Kept by the index pass, read by the capsule.
TASK_TWINS_KEY = "task-twins"
# The ticket prefixes this project files its work under - its tasks' IDs and
# the tasks its completion events name - so that `CMS-741` is a ticket here
# and `GPT-4` is not. Kept by the index pass, read by retrieval.
TICKET_PREFIXES_KEY = "ticket-prefixes"
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
    text = read_runtime_config_text(repository)
    return "absent" if text is None else _content_hash(text)


SkillStat = tuple[str, str, int, int]


def _policy_source_key(key: str) -> str:
    """A malformed policy inventory is unknown, never clean parity."""
    if not isinstance(key, str):
        raise RetrievalError('Policy source invalid-utf8-path')
    try:
        str.encode(key, 'utf-8')
    except UnicodeError as error:
        raise RetrievalError('Policy source invalid-utf8-path') from error
    return key


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
                        _policy_source_key(path.relative_to(root).as_posix()),
                        status.st_mtime_ns,
                        status.st_size,
                    )
                )
    else:
        entries = list(skill_stats)
    # Validate supplied entries too, before sorting or publishing any digest.
    for edition, relative, _, _ in entries:
        _policy_source_key(edition)
        _policy_source_key(relative)
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
        INDEX_PROVENANCE_POLICY_KEY: brain_runtime.provenance_policy_fingerprint(),
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
    if stored.get(INDEX_PROVENANCE_POLICY_KEY) != fingerprints[INDEX_PROVENANCE_POLICY_KEY]:
        # A retained chunk is not screened again (memory_eligibility), so a
        # changed instruction pattern set must reach unchanged chunks too.
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
                key = _policy_source_key(path.relative_to(root).as_posix())
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
            _policy_source_key(rel)
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
        yield _policy_source_key("/".join(rel_parts)), path


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
            # Admit the whole reverse inventory before the partial-install
            # shortcut or any content transform. Reuse it for the reverse pass.
            reverse = (list(_mirror_iter_canonical(mirror, None))
                       if cls.get("only") is None else [])
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
                for rel, path in reverse:
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
                rel = _policy_source_key("/".join(rel_parts))
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
    """One `document_links` row per source a document declares (`source`).

    A task's `files[]` become rows of their own kind, `file`, in
    `_file_link_rows`; they mean "touched", never "cites", and nothing that
    reads citations reads them. `fingerprint` stays out: `sources_are_fresh`
    requires the fingerprint path set to equal the sources path set, so it
    can never name anything `source` does not already name.
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


def _file_link_problem(path: object) -> Optional[str]:
    """Why a path may not become a file edge or a seed, or None.

    A task's files[] used to be refused as links outright: build output,
    vendored code and caches in the wrong frame. They are now screened one by
    one - shape, sensitive and private paths, runtime state, directories,
    build output and lockfiles, anything shaped like a secret - and what
    remains is checked against Git's index.
    """
    if not isinstance(path, str) or not path or any(char in path for char in "\n\r\0"):
        return "shape"
    if path.startswith("./"):
        path = path[2:]
    if posixpath.normpath(path) != path.rstrip("/") or path in (".", ""):
        return "shape"
    if source_path_problem(path):
        return "sensitive"
    if path.startswith(FILE_LINK_STATE_PREFIXES):
        return "state"
    if path.endswith("/"):
        return "directory"
    parts = path.split("/")
    if (
        any(part.casefold() in FILE_LINK_EXCLUDED_PARTS for part in parts)
        or path.startswith(FILE_LINK_EXCLUDED_PREFIXES)
        or parts[-1] in FILE_LINK_EXCLUDED_NAMES
    ):
        return "build"
    if any(pattern.search(path) for pattern in SECRET_PATTERNS.values()):
        return "secret"
    return None


def _cursor_nt_backend():
    """Installed lazy NT operations adapted from scripts/portable_fs.py.

    Handles never follow reparse points; held parents deny delete sharing.
    st_ctime_ns is Windows creation time, not a POSIX change-time token.
    """
    import errno
    import stat as stat_module
    from types import SimpleNamespace
    WINDOWS = True
    O_DIRECTORY, O_NOFOLLOW, O_NONBLOCK = 0x20000000, 0x40000000, 0x08000000
    def _component(name):
        if not isinstance(name, (str, bytes)):
            raise TypeError('A path component must be text or bytes.')
        text = os.fsdecode(name)
        if (not text or text in ('.', '..') or '/' in text or '\\' in text or ':' in text
                or '\x00' in text or text[-1:] in (' ', '.') or any(ord(c) < 32 for c in text)):
            raise ValueError('A filesystem component is unsafe.')
        if WINDOWS and text.rstrip(' .').upper().split('.')[0] in {
                'CON', 'PRN', 'AUX', 'NUL', *(f'COM{i}' for i in range(1, 10)), *(f'LPT{i}' for i in range(1, 10))}:
            raise ValueError('Windows device names are not filesystem components.')
        return text

    import ctypes

    import msvcrt

    import ntpath

    from ctypes import wintypes

    ULONG = wintypes.ULONG

    USHORT = wintypes.USHORT

    NTSTATUS = ctypes.c_long

    HANDLE = wintypes.HANDLE

    ACCESS_MASK = wintypes.DWORD

    class UNICODE_STRING(ctypes.Structure):
        _fields_ = [('Length', USHORT), ('MaximumLength', USHORT), ('Buffer', wintypes.LPWSTR)]

    class OBJECT_ATTRIBUTES(ctypes.Structure):
        _fields_ = [('Length', ULONG), ('RootDirectory', HANDLE),
                    ('ObjectName', ctypes.POINTER(UNICODE_STRING)), ('Attributes', ULONG),
                    ('SecurityDescriptor', wintypes.LPVOID), ('SecurityQualityOfService', wintypes.LPVOID)]

    class IO_STATUS_BLOCK(ctypes.Structure):
        _fields_ = [('Status', NTSTATUS), ('Information', ctypes.c_size_t)]

    class FILE_ATTRIBUTE_TAG_INFO(ctypes.Structure):
        _fields_ = [('FileAttributes', wintypes.DWORD), ('ReparseTag', wintypes.DWORD)]

    class BY_HANDLE_FILE_INFORMATION(ctypes.Structure):
        _fields_ = [('dwFileAttributes', wintypes.DWORD), ('ftCreationTime_dwLowDateTime', wintypes.DWORD),
                    ('ftCreationTime_dwHighDateTime', wintypes.DWORD), ('ftLastAccessTime_dwLowDateTime', wintypes.DWORD),
                    ('ftLastAccessTime_dwHighDateTime', wintypes.DWORD), ('ftLastWriteTime_dwLowDateTime', wintypes.DWORD),
                    ('ftLastWriteTime_dwHighDateTime', wintypes.DWORD), ('dwVolumeSerialNumber', wintypes.DWORD),
                    ('nFileSizeHigh', wintypes.DWORD), ('nFileSizeLow', wintypes.DWORD),
                    ('nNumberOfLinks', wintypes.DWORD), ('nFileIndexHigh', wintypes.DWORD),
                    ('nFileIndexLow', wintypes.DWORD)]

    _ntdll = ctypes.WinDLL('ntdll', use_last_error=True)

    _kernel32 = ctypes.WinDLL('kernel32', use_last_error=True)

    _nt_create = _ntdll.NtCreateFile

    _nt_create.argtypes = [ctypes.POINTER(HANDLE), ACCESS_MASK, ctypes.POINTER(OBJECT_ATTRIBUTES),
                                ctypes.POINTER(IO_STATUS_BLOCK), wintypes.LPVOID, ULONG, ULONG, ULONG,
                                ULONG, wintypes.LPVOID, ULONG]

    _nt_create.restype = NTSTATUS

    _nt_set = _ntdll.NtSetInformationFile

    _nt_set.argtypes = [HANDLE, ctypes.POINTER(IO_STATUS_BLOCK), wintypes.LPVOID, ULONG, ULONG]

    _nt_set.restype = NTSTATUS

    _status_to_error = _ntdll.RtlNtStatusToDosError

    _status_to_error.argtypes = [NTSTATUS]

    _status_to_error.restype = ULONG

    _get_info = _kernel32.GetFileInformationByHandle

    _get_info.argtypes = [HANDLE, ctypes.POINTER(BY_HANDLE_FILE_INFORMATION)]

    _get_info.restype = wintypes.BOOL

    _get_info_ex = _kernel32.GetFileInformationByHandleEx

    _get_info_ex.argtypes = [HANDLE, ctypes.c_int, wintypes.LPVOID, wintypes.DWORD]

    _get_info_ex.restype = wintypes.BOOL

    _close = _kernel32.CloseHandle

    _close.argtypes = [HANDLE]

    _close.restype = wintypes.BOOL

    FILE_READ_DATA = FILE_LIST_DIRECTORY = 0x0001

    FILE_WRITE_DATA = 0x0002

    FILE_APPEND_DATA = 0x0004

    FILE_READ_ATTRIBUTES = 0x0080

    FILE_WRITE_ATTRIBUTES = 0x0100

    FILE_TRAVERSE = 0x0020

    DELETE = 0x00010000

    SYNCHRONIZE = 0x00100000

    FILE_SHARE_READ, FILE_SHARE_WRITE = 0x1, 0x2

    FILE_OPEN, FILE_CREATE, FILE_OPEN_IF = 1, 2, 3

    FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE = 0x1, 0x40

    FILE_SYNCHRONOUS_IO_NONALERT, FILE_OPEN_REPARSE_POINT = 0x20, 0x00200000

    FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT = 0x80, 0x400

    OBJ_CASE_INSENSITIVE = 0x40

    FileAttributeTagInfo = 9

    FileNamesInformation = 12

    FileRenameInformation, FileLinkInformation, FileDispositionInformation = 10, 11, 13

    def _raise(status):
        code = _status_to_error(NTSTATUS(status))
        raise ctypes.WinError(code or errno.EIO)

    def raw_handle(fd):
        return int(msvcrt.get_osfhandle(fd))

    def _unicode(text):
        buffer = ctypes.create_unicode_buffer(text)
        size = len(text.encode('utf-16-le'))
        if size > 0xffff:
            raise ValueError('A filesystem component is too long.')
        return buffer, UNICODE_STRING(size, size, ctypes.cast(buffer, wintypes.LPWSTR))

    def _metadata(handle):
        tag = FILE_ATTRIBUTE_TAG_INFO()
        if not _get_info_ex(HANDLE(handle), FileAttributeTagInfo, ctypes.byref(tag), ctypes.sizeof(tag)):
            raise ctypes.WinError(ctypes.get_last_error())
        if tag.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT:
            raise OSError('A reparse point is unsafe.')
        info = BY_HANDLE_FILE_INFORMATION()
        if not _get_info(HANDLE(handle), ctypes.byref(info)):
            raise ctypes.WinError(ctypes.get_last_error())
        return info

    def _stat_result(handle):
        info = _metadata(handle)
        is_dir = bool(info.dwFileAttributes & 0x10)
        permissions = 0o777 if is_dir else 0o444 if info.dwFileAttributes & 1 else 0o666
        mode = (stat_module.S_IFDIR if is_dir else stat_module.S_IFREG) | permissions
        size = (info.nFileSizeHigh << 32) | info.nFileSizeLow
        index = (info.nFileIndexHigh << 32) | info.nFileIndexLow
        def timestamp(high, low):
            value = (high << 32) | low
            return max(0, (value - 116444736000000000) * 100)
        ctime_ns = timestamp(info.ftCreationTime_dwHighDateTime, info.ftCreationTime_dwLowDateTime)
        atime_ns = timestamp(info.ftLastAccessTime_dwHighDateTime, info.ftLastAccessTime_dwLowDateTime)
        mtime_ns = timestamp(info.ftLastWriteTime_dwHighDateTime, info.ftLastWriteTime_dwLowDateTime)
        # FILETIME is 100ns since 1601.  The exact timestamp is less important than
        # stable identity and size checks; retain nanosecond resolution where possible.
        return os.stat_result((mode, index, info.dwVolumeSerialNumber, info.nNumberOfLinks,
                               0, 0, size, atime_ns // 1_000_000_000,
                               mtime_ns // 1_000_000_000, ctime_ns // 1_000_000_000),
                              {'st_atime_ns': atime_ns, 'st_mtime_ns': mtime_ns, 'st_ctime_ns': ctime_ns})

    def _open_relative(parent_fd, name, access, disposition=FILE_OPEN, options=0,
                       attributes=FILE_ATTRIBUTE_NORMAL, share=None, security_descriptor=None):
        text = _component(name)
        buffer, unicode = _unicode(text)
        attributes_object = OBJECT_ATTRIBUTES(ctypes.sizeof(OBJECT_ATTRIBUTES), HANDLE(raw_handle(parent_fd)),
                                               ctypes.pointer(unicode), OBJ_CASE_INSENSITIVE,
                                               security_descriptor, None)
        output, status = HANDLE(), IO_STATUS_BLOCK()
        result = _nt_create(ctypes.byref(output), access, ctypes.byref(attributes_object), ctypes.byref(status),
                            None, attributes, FILE_SHARE_READ | FILE_SHARE_WRITE if share is None else share, disposition,
                            options | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT, None, 0)
        if result < 0:
            _raise(result)
        try:
            _metadata(output.value)
            crt_flags = os.O_BINARY
            if access & FILE_WRITE_DATA:
                crt_flags |= os.O_RDWR if access & FILE_READ_DATA else os.O_WRONLY
            if access & FILE_APPEND_DATA:
                crt_flags |= os.O_APPEND
            return msvcrt.open_osfhandle(output.value, crt_flags)
        except BaseException:
            try:
                _close(output)
            except BaseException:
                pass  # Preserve abort; no retry after an uncertain close.
            raise

    def _open_root(path, security=False):
        text = os.fspath(path)
        if isinstance(text, bytes):
            text = os.fsdecode(text)
        drive, tail = ntpath.splitdrive(text)
        if not drive or drive.startswith('\\\\') or not ntpath.isabs(text):
            raise ValueError('Use an absolute local Windows path.')
        root = drive + '\\'
        # CreateFileW is used only for the volume root; untrusted components are
        # then opened one-by-one relative to that rooted handle.
        create = _kernel32.CreateFileW
        create.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID,
                           wintypes.DWORD, wintypes.DWORD, HANDLE]
        create.restype = HANDLE
        handle = create('\\\\?\\' + root, FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                        FILE_SHARE_READ | FILE_SHARE_WRITE, None, 3,
                        0x02000000 | 0x00200000, None)
        if handle in (None, ctypes.c_void_p(-1).value):
            raise ctypes.WinError(ctypes.get_last_error())
        fd = None
        try:
            _metadata(handle)
            fd = msvcrt.open_osfhandle(handle, os.O_BINARY)
            handle = None
            parts = [item for item in tail.replace('/', '\\').split('\\') if item]
            for index, part in enumerate(parts):
                access = FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE
                if security and index == len(parts) - 1:
                    access |= 0x00020000 | 0x00040000 | 0x00080000
                previous = fd
                fd = _open_relative(previous, part, access,
                                    options=FILE_DIRECTORY_FILE)
                os.close(previous)
            return fd
        except BaseException:
            try:
                if fd is not None:
                    os.close(fd)
                elif handle is not None:
                    _close(HANDLE(handle))
            except BaseException:
                pass  # Unknown close result: preserve abort, never retry an old fd.
            raise

    def open(path, flags, mode=0o777, *, dir_fd=None):
        if dir_fd is None:
            # Absolute paths are walked from the drive root.  A direct file open
            # uses the verified parent handle rather than a path-based fallback.
            text = os.fspath(path)
            parent, name = ntpath.split(text)
            root = _open_root(parent)
            try:
                return open(name, flags, mode, dir_fd=root)
            finally:
                os.close(root)
        want_dir = bool(flags & O_DIRECTORY)
        writing = bool(flags & (os.O_WRONLY | os.O_RDWR))
        creating = bool(flags & os.O_CREAT)
        exclusive = bool(flags & os.O_EXCL)
        if creating and exclusive:
            disposition = FILE_CREATE
        elif creating:
            disposition = FILE_OPEN_IF
        else:
            disposition = FILE_OPEN
        access = SYNCHRONIZE | FILE_READ_ATTRIBUTES
        if want_dir:
            access |= FILE_LIST_DIRECTORY | FILE_TRAVERSE
        elif writing:
            access |= FILE_WRITE_DATA | FILE_READ_DATA
        else:
            access |= FILE_READ_DATA
        if flags & os.O_APPEND:
            access |= FILE_APPEND_DATA
        options = FILE_DIRECTORY_FILE if want_dir else FILE_NON_DIRECTORY_FILE
        return _open_relative(dir_fd, path, access, disposition, options)

    close = os.close

    dup = os.dup

    def fstat(fd):
        return _stat_result(raw_handle(fd))

    def stat(path, *, dir_fd=None, follow_symlinks=True):
        if follow_symlinks:
            raise ValueError('Portable rooted stat never follows links.')
        if dir_fd is None:
            text = os.fspath(path)
            parent, name = ntpath.split(text)
            root = _open_root(parent)
            try:
                fd = _open_relative(root, name, FILE_READ_ATTRIBUTES | SYNCHRONIZE)
            finally:
                os.close(root)
        else:
            fd = _open_relative(dir_fd, path, FILE_READ_ATTRIBUTES | SYNCHRONIZE)
        try:
            return fstat(fd)
        finally:
            os.close(fd)

    def mkdir(name, mode=0o777, *, dir_fd=None):
        if dir_fd is None:
            raise ValueError('mkdir requires a trusted parent descriptor on Windows.')
        fd = _open_relative(dir_fd, name, FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                            FILE_CREATE, FILE_DIRECTORY_FILE)
        os.close(fd)

    def _disposition(parent_fd, name, directory=False):
        access = DELETE | SYNCHRONIZE | FILE_READ_ATTRIBUTES
        options = FILE_DIRECTORY_FILE if directory else FILE_NON_DIRECTORY_FILE
        fd = _open_relative(parent_fd, name, access, FILE_OPEN, options)
        try:
            deleting = ctypes.c_ubyte(1)
            status = IO_STATUS_BLOCK()
            result = _nt_set(HANDLE(raw_handle(fd)), ctypes.byref(status), ctypes.byref(deleting), 1,
                             FileDispositionInformation)
            if result < 0:
                _raise(result)
        finally:
            os.close(fd)

    def unlink(name, *, dir_fd=None):
        if dir_fd is None:
            raise ValueError('unlink requires a trusted parent descriptor on Windows.')
        _disposition(dir_fd, name)

    def _rename_information(parent_fd, name, replace):
        text = _component(name)
        encoded = text.encode('utf-16-le')
        # BOOLEAN ReplaceIfExists; padding; HANDLE RootDirectory; ULONG length; WCHAR[]
        root_offset = ctypes.sizeof(HANDLE)
        name_length_offset = root_offset + ctypes.sizeof(HANDLE)
        offset = name_length_offset + ctypes.sizeof(ULONG)
        body = ctypes.create_string_buffer(offset + len(encoded))
        body[0] = b'\x01' if replace else b'\x00'
        ctypes.c_void_p.from_address(ctypes.addressof(body) + root_offset).value = raw_handle(parent_fd)
        ctypes.c_ulong.from_address(ctypes.addressof(body) + name_length_offset).value = len(encoded)
        ctypes.memmove(ctypes.addressof(body) + offset, encoded, len(encoded))
        return body

    def atomic_replace_open(source_fd, destination_dir_fd, destination):
        body = _rename_information(destination_dir_fd, destination, True)
        status = IO_STATUS_BLOCK()
        result = _nt_set(HANDLE(raw_handle(source_fd)), ctypes.byref(status), body, len(body), FileRenameInformation)
        if result < 0:
            _raise(result)

    def replace(source, destination, *, src_dir_fd=None, dst_dir_fd=None):
        if src_dir_fd is None or dst_dir_fd is None:
            raise ValueError('replace requires trusted source and destination directories on Windows.')
        fd = _open_relative(src_dir_fd, source, DELETE | SYNCHRONIZE | FILE_READ_ATTRIBUTES,
                            FILE_OPEN, FILE_NON_DIRECTORY_FILE)
        try:
            atomic_replace_open(fd, dst_dir_fd, destination)
        finally:
            os.close(fd)

    def publish_absent(source, destination, *, src_dir_fd, dst_dir_fd):
        fd = _open_relative(src_dir_fd, source, DELETE | SYNCHRONIZE | FILE_READ_ATTRIBUTES,
                            FILE_OPEN, FILE_NON_DIRECTORY_FILE)
        try:
            info = _rename_information(dst_dir_fd, destination, False)
            status = IO_STATUS_BLOCK()
            result = _nt_set(HANDLE(raw_handle(fd)), ctypes.byref(status), info, len(info), FileRenameInformation)
            if result < 0:
                _raise(result)
        finally:
            os.close(fd)

    return SimpleNamespace(WINDOWS=True, open=open, fstat=fstat, stat=stat,
                           mkdir=mkdir, unlink=unlink, replace=replace,
                           publish_absent=publish_absent, open_root=_open_root,
                           O_DIRECTORY=O_DIRECTORY, O_NOFOLLOW=O_NOFOLLOW,
                           O_NONBLOCK=O_NONBLOCK)


def _cursor_backend():
    if os.name == "posix":
        return os
    if os.name == "nt":
        return _cursor_nt_backend()
    return None


CURSOR_RULE_LIMIT = 16384
CURSOR_RECEIPT_LIMIT = 4096
CURSOR_RULE_PROFILE = "cursor-rule-custody-v1"


def _cursor_identity(image: os.stat_result) -> list[int]:
    return [image.st_dev, image.st_ino, image.st_size, image.st_mtime_ns, image.st_ctime_ns]


def _cursor_safe_directory(path: Path, *, files=None) -> list[int]:
    if files is None or files is os:
        image = path.lstat()
    else:
        descriptor = files.open_root(path)
        try:
            image = files.fstat(descriptor)
        finally:
            os.close(descriptor)
    if (not stat.S_ISDIR(image.st_mode) or stat.S_ISLNK(image.st_mode)
            or getattr(image, "st_reparse_tag", 0) in workspace_roots.LINK_REPARSE_TAGS):
        raise OSError("unsupported directory custody")
    return [image.st_dev, image.st_ino]


def _cursor_directory(parent: int, name: str, *, create: bool = False, files=None) -> int:
    files = os if files is None else files
    if create:
        try:
            files.mkdir(name, 0o700, dir_fd=parent)
        except FileExistsError:
            pass
    descriptor = files.open(name, os.O_RDONLY | files.O_DIRECTORY | files.O_NOFOLLOW, dir_fd=parent)
    try:
        image = files.fstat(descriptor)
        named = files.stat(name, dir_fd=parent, follow_symlinks=False)
        if (not stat.S_ISDIR(named.st_mode)
                or (named.st_dev, named.st_ino) != (image.st_dev, image.st_ino)):
            raise OSError("directory custody changed")
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def _cursor_linked_entry(parent: int, name: str, files) -> bool:
    """Whether a directory entry is a link or a junction, not a directory."""
    try:
        image = files.stat(name, dir_fd=parent, follow_symlinks=False)
    except OSError:
        return False
    return (stat.S_ISLNK(image.st_mode)
            or getattr(image, "st_reparse_tag", 0) in workspace_roots.LINK_REPARSE_TAGS)


def _cursor_image(parent: Optional[int], name: str, limit: int, *, files=None, links: int = 1) -> Optional[tuple[bytes, list[int]]]:
    files = os if files is None else files
    if parent is None:
        return None
    try:
        named = files.stat(name, dir_fd=parent, follow_symlinks=False)
    except FileNotFoundError:
        return None
    if not stat.S_ISREG(named.st_mode) or named.st_nlink != links or named.st_size > limit:
        raise OSError("unsupported file custody")
    descriptor = files.open(name, os.O_RDONLY | files.O_NOFOLLOW | files.O_NONBLOCK, dir_fd=parent)
    try:
        opened = files.fstat(descriptor)
        if _cursor_identity(opened) != _cursor_identity(named):
            raise OSError("file custody changed")
        raw = b""
        while len(raw) <= limit:
            part = os.read(descriptor, min(4096, limit + 1 - len(raw)))
            if not part:
                break
            raw += part
        final = files.fstat(descriptor)
        if (len(raw) != opened.st_size or final.st_nlink != links
                or _cursor_identity(final) != _cursor_identity(opened)
                or _cursor_identity(files.stat(name, dir_fd=parent, follow_symlinks=False)) != _cursor_identity(final)):
            raise OSError("file custody changed")
        raw.decode("utf-8", "strict")
        return raw, _cursor_identity(final)
    finally:
        os.close(descriptor)


@contextlib.contextmanager
def _cursor_lock(root: Path, parent: int, *, files=None):
    """Use the native cooperative lock, with anchored I/O and no lock wait.

    The in-process lock/depth registry is the one used by mutation_lock.
    Windows uses the same byte-zero lock as native Brain, with one attempt.
    """
    files = os if files is None else files
    windows = getattr(files, "WINDOWS", False)
    if windows:
        import msvcrt
    else:
        import fcntl
    key = str((root / "project-brain/local/runtime.lock").resolve())
    if not brain_runtime._LOCKS_GUARD.acquire(blocking=False):
        raise OSError("busy")
    try:
        process_lock = brain_runtime._LOCKS.setdefault(key, threading.RLock())
    finally:
        brain_runtime._LOCKS_GUARD.release()
    if not process_lock.acquire(blocking=False):
        raise OSError("busy")
    descriptor = None
    locked = False
    depths = getattr(brain_runtime._LOCK_STATE, "depths", {})
    brain_runtime._LOCK_STATE.depths = depths
    depth = depths.get(key, 0)
    try:
        descriptor = files.open("runtime.lock", os.O_RDWR | os.O_CREAT | files.O_NOFOLLOW | files.O_NONBLOCK,
                             0o600, dir_fd=parent)
        opened = files.fstat(descriptor)
        named = files.stat("runtime.lock", dir_fd=parent, follow_symlinks=False)
        if (not stat.S_ISREG(opened.st_mode) or opened.st_nlink != 1
                or _cursor_identity(named) != _cursor_identity(opened)):
            raise OSError("unsupported lock custody")
        if not depth:
            if windows:
                os.lseek(descriptor, 0, os.SEEK_SET)
                msvcrt.locking(descriptor, msvcrt.LK_NBLCK, 1)
            else:
                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            locked = True
        depths[key] = depth + 1
        try:
            yield
        finally:
            if depth:
                depths[key] = depth
            else:
                depths.pop(key, None)
    finally:
        try:
            if descriptor is not None:
                try:
                    if locked:
                        if windows:
                            os.lseek(descriptor, 0, os.SEEK_SET)
                            msvcrt.locking(descriptor, msvcrt.LK_UNLCK, 1)
                        else:
                            fcntl.flock(descriptor, fcntl.LOCK_UN)
                finally:
                    os.close(descriptor)
        finally:
            process_lock.release()


def _cursor_publish_absent(parent: int, temporary: str, name: str) -> None:
    """Use a no-clobber rename when supported, otherwise the link fallback.

    Failure never means that no effect occurred. The caller observes the named
    image and the next invocation reconciles only a validated reserved slot.
    """
    import ctypes
    import errno
    if sys.platform.startswith("linux"):
        library = ctypes.CDLL(None, use_errno=True)
        rename = getattr(library, "renameat2", None)
        if rename is not None:
            rename.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int,
                               ctypes.c_char_p, ctypes.c_uint]
            rename.restype = ctypes.c_int
            if rename(parent, os.fsencode(temporary), parent, os.fsencode(name), 1) == 0:
                return
            failure = ctypes.get_errno()
            if failure not in (errno.ENOSYS, errno.EINVAL, errno.EOPNOTSUPP):
                raise OSError(failure, os.strerror(failure))
    os.link(temporary, name, src_dir_fd=parent, dst_dir_fd=parent, follow_symlinks=False)


def _cursor_publish(parent: int, name: str, payload: bytes, expected, limit: int, *, files=None) -> None:
    files = os if files is None else files
    if len(payload) > limit:
        raise OSError("publication admission bound")
    temporary = ".cursor-rule-" + new_uuid() + ".tmp"
    descriptor = files.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | files.O_NOFOLLOW,
                         0o600, dir_fd=parent)
    try:
        try:
            pending = memoryview(payload)
            while pending:
                count = os.write(descriptor, pending)
                if count <= 0:
                    raise OSError("incomplete write")
                pending = pending[count:]
        finally:
            os.close(descriptor)
        if _cursor_image(parent, name, limit, **({} if files is os else {"files": files})) != expected:
            raise OSError("file custody changed")
        if expected is None:
            if getattr(files, "WINDOWS", False):
                files.publish_absent(temporary, name, src_dir_fd=parent, dst_dir_fd=parent)
            else:
                _cursor_publish_absent(parent, temporary, name)
        else:
            files.replace(temporary, name, src_dir_fd=parent, dst_dir_fd=parent)
    finally:
        try:
            files.unlink(temporary, dir_fd=parent)
        except FileNotFoundError:
            pass  # replace consumed our temporary name


def _cursor_receipt(raw: bytes) -> dict[str, Any]:
    def unique(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate receipt field")
            result[key] = value
        return result
    value = json.loads(raw, object_pairs_hook=unique)
    fields = {"profile", "root", "root_identity", "rules_identity", "file_identity",
              "sha256", "task", "phase"}
    if (not isinstance(value, dict) or set(value) != fields
            or value["profile"] != CURSOR_RULE_PROFILE
            or not isinstance(value["root"], str)
            or not isinstance(value["task"], str)
            or not re.fullmatch(r"[^\W_][\w\u0300-\u036f./#+-]{0,199}", value["task"])
            or value["phase"] not in ("prompt", "turn", "session")
            or not isinstance(value["sha256"], str)
            or not re.fullmatch(r"[0-9a-f]{64}", value["sha256"])):
        raise ValueError("unsupported receipt")
    for name, width in (("root_identity", 2), ("rules_identity", 2), ("file_identity", 5)):
        image = value[name]
        if (not isinstance(image, list) or len(image) != width
                or any(type(item) is not int or item < 0 for item in image)):
            raise ValueError("unsupported identity")
    return value


def _cursor_git_config_read(arguments: list[str], environment: dict[str, str]) -> Optional[tuple[int, bytes]]:
    """One fixed protected config query with bounded pipe read and cleanup."""
    import queue
    import threading
    process = None
    reader = None
    started = False
    output = queue.Queue(maxsize=1)
    try:
        process = subprocess.Popen(arguments, env=environment, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        def read():
            try:
                output.put_nowait(process.stdout.read(65537))
            except (OSError, ValueError):
                output.put_nowait(None)
            finally:
                process.stdout.close()  # reader owns its buffered stream lock
        reader = threading.Thread(target=read, daemon=True)
        reader.start()
        started = True
        raw = output.get(timeout=1)
        if raw is None or len(raw) > 65536:
            return None
        code = process.wait(timeout=1)
        return code, raw
    except (OSError, ValueError, RuntimeError, queue.Empty, subprocess.TimeoutExpired):
        return None
    finally:
        if process is not None:
            try:
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=1)
            except (OSError, subprocess.TimeoutExpired):
                pass  # unavailable; no claim about a helper's descendants
            if not started and process.stdout is not None:
                process.stdout.close()
        if started:
            reader.join(timeout=1)


def _cursor_protected_safe_root(repository: Path) -> bool:
    """Project trusted system/global policy into one fixed root exception.

    Inherited command/environment config and repository-local values are not
    consulted. A protected wildcard is operator policy, never added by us.
    """
    environment = {k:v for k,v in os.environ.items() if not k.startswith('GIT_')}
    environment.update(GIT_OPTIONAL_LOCKS='0', GIT_TERMINAL_PROMPT='0')
    target = os.path.normcase(str(repository.resolve()))
    # Every file Git reads, each value with its scope, in Git's own order:
    # `config --global` reads only ~/.gitconfig when it exists, and missed a
    # safe.directory that $XDG_CONFIG_HOME/git/config gave - which Git itself
    # honours. Only the protected scopes count, as for Git.
    observed = _cursor_git_config_read(['git','--no-optional-locks','--no-pager', '-C', str(repository),
        'config','--show-scope','--includes','--null','--get-all','safe.directory'], environment)
    if observed is None or observed[0] not in (0,1):
        return False
    raw = observed[1]
    if len(raw)>131072 or raw.count(b'\0')>512 or (raw and not raw.endswith(b'\0')):
        return False
    allowed = False
    try:
        fields = raw.decode('utf-8','strict').split('\0')[:-1] if raw else []
        if len(fields) % 2:
            return False
        for scope, value in zip(fields[0::2], fields[1::2]):
            if scope not in ('system', 'global'):
                continue
            if value == '':
                allowed = False
                continue
            if value == '*':
                allowed = True
                continue
            if any(c in value for c in ('\r','\n','\x00')) or value.startswith('%(prefix)'):
                continue
            subtree = value.endswith('/*')
            candidate = value[:-2] if subtree else value
            if candidate.startswith('~/'):
                candidate = str(Path(candidate).expanduser())
            path = Path(candidate)
            if not path.is_absolute():
                continue
            named = os.path.normcase(str(path.resolve()))
            if named == target or (subtree and target.startswith(named.rstrip(os.sep)+os.sep)):
                allowed = True
    except (OSError, ValueError, UnicodeError, RuntimeError):
        return False
    return allowed


def _cursor_git_untracked(repository: Path, *, extra: tuple[str, ...] = ()) -> bool:
    # An explicitly selected alternate repository/index makes custody ambiguous.
    # Preserve rather than silently ignore that operator-selected authority.
    selectors = {"GIT_DIR", "GIT_COMMON_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE",
                 "GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES",
                 "GIT_SHALLOW_FILE", "GIT_NAMESPACE"}
    if any(name in os.environ for name in selectors):
        return False
    # Otherwise use the selected root's ordinary index in a private environment.
    # Editor/SSH/askpass/config settings neither redirect this read nor disable
    # maintenance. Never mutate ambient env.
    environment = {name: value for name, value in os.environ.items() if not name.startswith("GIT_")}
    environment.update(GIT_OPTIONAL_LOCKS="0", GIT_TERMINAL_PROMPT="0",
                       GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
    trust = []
    for name in (".cursor/rules/working-memory.mdc", "memory-bank/local/cursor-rule.json", *extra):
        # Discard even a directory-shaped index match: no unbounded output buffer.
        result = subprocess.run(
            ["git", "--no-optional-locks", "--no-pager", "--literal-pathspecs", *trust,
             "-c", "core.hooksPath=" + os.devnull, "-c", "core.fsmonitor=false",
             "-C", str(repository), "ls-files", "--error-unmatch", "--", name],
            env=environment, stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False, timeout=1,
        )
        if result.returncode == 128 and not trust and _cursor_protected_safe_root(repository):
            trust = ['-c', 'safe.directory='+str(repository.resolve())]
            result = subprocess.run(
                ['git','--no-optional-locks','--no-pager','--literal-pathspecs', *trust,
                 '-c','core.hooksPath='+os.devnull,'-c','core.fsmonitor=false',
                 '-C',str(repository),'ls-files','--error-unmatch','--',name],
                env=environment,stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,check=False,timeout=1)
        if result.returncode != 1:
            return False
    return True


def _same_file_image(left, right) -> bool:
    """Same file and bytes by stat, without atime.

    Comparing whole stat results compared st_atime too, and reading the
    file - which this very check does - moves atime on strictatime mounts and
    often on relatime ones, so the image never matched and recovery failed.
    The change time is the one a path's stat and an open file's fstat agree
    on: Windows gives them different ones.
    """
    keys = ("st_dev", "st_ino", "st_mode", "st_nlink", "st_size", "st_mtime_ns")
    return (all(getattr(left, key) == getattr(right, key) for key in keys)
            and workspace_roots.change_time_ns(left) == workspace_roots.change_time_ns(right))


def _cursor_registered_slot(root: Path) -> bool:
    """An explicit project ignore directive registers this generated slot.

    A whole machine envelope classifies a reserved output, never its author.
    Ordinary custom rules belong in another path; deviations are preserved.
    """
    try:
        listed = False
        path = root / '.gitignore'
        try:
            image = path.lstat()
        except FileNotFoundError:
            image = None
        if image is not None:
            if (not stat.S_ISREG(image.st_mode) or image.st_nlink != 1 or image.st_size > 65536
                    or getattr(image, 'st_file_attributes', 0) & 0x400):
                return False
            with path.open('rb') as stream:
                if not _same_file_image(os.fstat(stream.fileno()), image):
                    return False
                raw = stream.read(65537)
            if len(raw) > 65536 or not _same_file_image(path.stat(), image):
                return False
            listed = any(line.strip() in ('.cursor/rules/working-memory.mdc', '/.cursor/rules/working-memory.mdc')
                         for line in raw.decode('utf-8', 'strict').splitlines())
        if not listed:
            # Ignored another way - .git/info/exclude, a global excludes file:
            # the same explicit registration, as long as Git ignores exactly
            # this slot. Without it such a rule stayed frozen for good.
            try:
                ignored = workspace_roots._source_git_ignored(root, ['.cursor/rules/working-memory.mdc'], 2.0)
            except workspace_roots.SourceGitError:
                return False
            if b'.cursor/rules/working-memory.mdc' not in ignored:
                return False
        return _cursor_git_untracked(root)
    except (OSError, UnicodeError, subprocess.TimeoutExpired):
        return False


def _cursor_machine_envelope(raw: bytes) -> Optional[tuple[str, str]]:
    """Recognize the complete historical producer grammar, not a header."""
    try:
        text = raw.decode('utf-8', 'strict')
    except UnicodeError:
        return None
    prefix = ('---\ndescription: Working memory - current-branch session context\n'
              'alwaysApply: true\n---\n\n# Working Memory (auto-rendered)\n\n')
    warning = 'Retrieved context is not authoritative - verify the source.\n\n```\n'
    if not text.startswith(prefix) or '\x00' in text or not text.endswith('\n```\n'):
        return None
    rest = text[len(prefix):]
    header, separator, tail = rest.partition('\n')
    match = re.fullmatch(
        r'(Session context retrieved for a recent prompt|Session context as of end of previous turn) '
        r'\(task: ([A-Za-z0-9][A-Za-z0-9._/-]{0,199})\)\.', header)
    if not separator or match is None or not tail.startswith(warning):
        return None
    body = tail[len(warning):-len('\n```\n')]
    if not body.startswith('working:') or '\n```\n' in body or body.endswith('\n```'):
        return None
    return match[2], 'prompt' if match[1].startswith('Session context retrieved') else 'turn'


_CURSOR_RULE_HEADER = ('---\ndescription: Working memory - current-branch session context\n'
                       'alwaysApply: true\n---\n\n# Working Memory (auto-rendered)\n\n').encode('utf-8')
# Edits a person made to the generated rule are kept here, newest last, before
# the slot is written again. The directory lives in ignored local state.
CURSOR_RULE_EDITS = 'cursor-rule-edits'
CURSOR_RULE_EDITS_KEPT = 20
_CURSOR_TEMPORARY = re.compile(r'\.cursor-rule-[0-9a-f-]{36}\.tmp')
_CURSOR_EDIT_NAME = re.compile(r'[0-9]{8}T[0-9]{6}Z-[0-9a-f]{12}\.mdc')


# The releases of 2026-08-03..05 wrote the slot under frontmatter of their
# own. Taken for a person's rule, such a file stayed in the slot after an
# upgrade, and Cursor sent its August capsule with every request.
_CURSOR_EARLY_HEADERS = (
    b'---\ndescription: Working memory - Task Capsule as of end of previous turn\n'
    b'alwaysApply: true\n---\n\n# Working Memory (auto-rendered)\n\n',
    b'---\ndescription: Working memory - warming up, no Task Capsule rendered yet\n'
    b'alwaysApply: true\n---\n\n# Working Memory (warming up)\n\n',
)


def _cursor_edited_generation(raw: bytes) -> bool:
    """A generated rule this release did not write as it stands, as opposed
    to a rule nobody generated.

    The file still opens with a header only this producer writes - the
    current auto-rendered one or a first release's - so a person changed the
    generator's output (typically a note appended below it), or an early
    release rendered it. A file without such a header is someone's own rule,
    whatever the slot held before, and stays untouched.
    """
    return raw.startswith((_CURSOR_RULE_HEADER, *_CURSOR_EARLY_HEADERS))


def _cursor_preserve_edit(edits_fd: int, raw: bytes, *, files=None) -> str:
    """Save an edited generated rule before the slot is regenerated.

    Returns the saved file's project-relative path. Raises on any failure, so
    the caller writes nothing over edits it could not keep. Only the newest
    CURSOR_RULE_EDITS_KEPT copies stay.
    """
    files = os if files is None else files
    name = (time.strftime('%Y%m%dT%H%M%SZ', time.gmtime()) + '-'
            + hashlib.sha256(raw).hexdigest()[:12] + '.mdc')
    options = {} if files is os else {'files': files}
    existing = _cursor_image(edits_fd, name, CURSOR_RULE_LIMIT, **options)
    if existing is None:
        _cursor_publish(edits_fd, name, raw, None, CURSOR_RULE_LIMIT, **options)
    elif existing[0] != raw:
        raise OSError('edit copy custody changed')
    if files is os:
        saved = sorted(entry for entry in os.listdir(edits_fd) if _CURSOR_EDIT_NAME.fullmatch(entry))
        for entry in saved[:-CURSOR_RULE_EDITS_KEPT]:
            try:
                if stat.S_ISREG(os.stat(entry, dir_fd=edits_fd, follow_symlinks=False).st_mode):
                    os.unlink(entry, dir_fd=edits_fd)
            except FileNotFoundError:
                pass
    return 'memory-bank/local/' + CURSOR_RULE_EDITS + '/' + name


def _cursor_sweep_temporaries(parent: Optional[int], *, files=None, age: float = 60.0) -> None:
    """Remove publication temporaries a killed writer left behind.

    A writer stopped between writing its temporary and renaming it left
    `.cursor-rule-<uuid>.tmp` beside the rule - not ignored, so `git add -A`
    picked it up. Only stale regular files of exactly that name go.
    """
    files = os if files is None else files
    if parent is None or files is not os:
        return
    cutoff = time.time() - age
    for entry in os.listdir(parent):
        if not _CURSOR_TEMPORARY.fullmatch(entry):
            continue
        try:
            image = os.stat(entry, dir_fd=parent, follow_symlinks=False)
            if stat.S_ISREG(image.st_mode) and image.st_mtime < cutoff:
                os.unlink(entry, dir_fd=parent)
        except FileNotFoundError:
            pass


def _cursor_reconcile_link(root: Path, parent: int, name: str, limit: int,
                           relative: str, intact, publishing) -> bool:
    """Reconcile only the old producer's exact two-name publication window.

    Caller holds the native lock and anchored directories. Generic hard links,
    unknown aliases and custom rules remain outside this finite profile.
    """
    named = os.stat(name, dir_fd=parent, follow_symlinks=False)
    if named.st_nlink != 2 or not _cursor_registered_slot(root) or not intact():
        return False
    current = _cursor_image(parent, name, limit, links=2)
    if current is None:
        return False
    if name == 'working-memory.mdc':
        if _cursor_machine_envelope(current[0]) is None:
            return False
    else:
        _cursor_receipt(current[0])
    alias = None
    with os.scandir(parent) as entries:
        for count, entry in enumerate(entries, 1):
            if count > 1024:
                return False
            if not re.fullmatch(r'\.cursor-rule-[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\.tmp', entry.name):
                continue
            image = entry.stat(follow_symlinks=False)
            if (image.st_dev, image.st_ino) == tuple(current[1][:2]):
                if alias is not None or not stat.S_ISREG(image.st_mode) or image.st_nlink != 2:
                    return False
                alias = entry.name
    if alias is None or not _cursor_git_untracked(root, extra=(relative + '/' + alias,)):
        return False
    if (not intact() or _cursor_image(parent, name, limit, links=2) != current
            or _cursor_image(parent, alias, limit, links=2) != current or not intact()):
        return False
    publishing()
    os.unlink(alias, dir_fd=parent)
    observed = _cursor_image(parent, name, limit)
    return (observed is not None and observed[0] == current[0]
            and observed[1][:4] == current[1][:4] and intact())



def cursor_rule(repository: Path, action: str, *, task: str = "", phase: str = "prompt",
                capsule: str = "", approved_sha256: Optional[str] = None) -> dict[str, Any]:
    """Maintain the registered generated slot; preserve custom deviations.

    Directory descriptors prevent parent-link redirection. Participating
    writers share the native lock without waiting. Leaf checks are not an
    atomic CAS against unsynchronized foreign writers, and the rule/receipt
    pair is not a crash-durable transaction. Unsupported custody preserves.
    """
    result: dict[str, Any] = {"profile": CURSOR_RULE_PROFILE, "state": "preserved",
                              "bytes_removed": 0}
    if action not in ("write", "keep", "retire", "prompt-held", "migrate"):
        return result
    linked: list[str] = []
    try:
        if workspace_roots.configuration_error() is not None:
            return {**result, "reason": "configuration_unavailable"}
        if workspace_roots.is_attached(repository):
            return {**result, "state": "attached"}
        try:
            files = _cursor_backend()
        except (OSError, ImportError, AttributeError):
            return {**result, "reason": "unsupported_custody"}
        if (files is None or (files is os and (not hasattr(os, "O_NOFOLLOW") or not hasattr(os, "O_DIRECTORY")
                or not all(operation in os.supports_dir_fd for operation in
                           (os.open, os.stat, os.mkdir, os.unlink, os.rename))))):
            return {**result, "reason": "unsupported_custody"}
        options = {} if files is os else {"files": files}
        root_identity = _cursor_safe_directory(repository, **options)
        root = repository.resolve()
        if (root != workspace_roots.TOOLING_ROOT or workspace_roots.project_root(repository).resolve() != root
                or workspace_roots.linked_state_entry(root) is not None):
            return result
        with contextlib.ExitStack() as stack:
            root_fd = (os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW) if files is os
                       else files.open_root(root))
            stack.callback(os.close, root_fd)
            root_image = files.fstat(root_fd)
            if [root_image.st_dev, root_image.st_ino] != root_identity:
                return result
            directories = {}
            def directory(parent, name, create=False):
                if parent is None:
                    return None
                try:
                    fd = _cursor_directory(parent, name, create=create, **options)
                except FileNotFoundError:
                    return None
                except OSError:
                    # A rules folder shared between projects through a link
                    # would carry this project's capsule to all of them: it
                    # is never written through, and the refusal says why.
                    if name in (".cursor", "rules") and _cursor_linked_entry(parent, name, files):
                        linked.append(name)
                    raise
                stack.callback(os.close, fd)
                directories[(parent, name)] = fd
                return fd
            def intact():
                if _cursor_safe_directory(root, **options) != root_identity:
                    return False
                for (parent, name), fd in directories.items():
                    named = files.stat(name, dir_fd=parent, follow_symlinks=False)
                    opened = files.fstat(fd)
                    if (not stat.S_ISDIR(named.st_mode)
                            or (named.st_dev, named.st_ino) != (opened.st_dev, opened.st_ino)):
                        return False
                return True
            cursor_fd = directory(root_fd, ".cursor")
            rules_fd = directory(cursor_fd, "rules")
            bank_fd = directory(root_fd, "memory-bank")
            local_fd = directory(bank_fd, "local")
            def initial_image(parent, name, limit, relative):
                def publishing():
                    result['state'] = 'incomplete'
                try:
                    return _cursor_image(parent, name, limit, **options)
                except OSError:
                    if files is not os or parent is None:
                        raise
                    # A crash in an old link-based producer can leave its one
                    # temporary alias. Recovery remains in the reserved slot.
                    brain = directory(root_fd, 'project-brain', True)
                    brain_local = directory(brain, 'local', True)
                    with _cursor_lock(root, brain_local):
                        if not _cursor_reconcile_link(root, parent, name, limit, relative, intact, publishing):
                            raise OSError('unsupported file custody')
                    return _cursor_image(parent, name, limit)
            current = initial_image(rules_fd, "working-memory.mdc", CURSOR_RULE_LIMIT, '.cursor/rules')
            proof = initial_image(local_fd, "cursor-rule.json", CURSOR_RECEIPT_LIMIT, 'memory-bank/local')
            approved_migration = False
            if action == "migrate":
                # Explicit operator approval binds this one current image,
                # never its header or a digest supplied by the repository.
                if (current is None
                        or not isinstance(approved_sha256, str)
                        or not re.fullmatch(r"[0-9a-f]{64}", approved_sha256)
                        or hashlib.sha256(current[0]).hexdigest() != approved_sha256):
                    return {**result, "reason": "migration_unapproved"}
                approved_migration = True
                action = "write"
            if current is None and proof is None and action != "write":
                return {**result, "state": "absent"}
            note = None
            relocated = False
            reconcile_state = None
            recovered = False
            if proof is not None and not approved_migration:
                try:
                    note = _cursor_receipt(proof[0])
                except (ValueError, UnicodeError, RecursionError):
                    note = None
            matches = note is not None and (current is None or note['sha256'] == hashlib.sha256(current[0]).hexdigest())
            edited = False
            if current is not None and not matches and not approved_migration:
                machine = _cursor_machine_envelope(current[0])
                if machine is None and _cursor_edited_generation(current[0]):
                    # A person edited the generated rule. Left alone it froze
                    # with one task's capsule on every branch; it is saved to
                    # memory-bank/local/cursor-rule-edits/ before the slot is
                    # written or retired, never discarded.
                    machine, edited = ('', ''), True
                if machine is None or not _cursor_registered_slot(root):
                    return {**result, 'reason': 'custody_changed' if proof is not None else 'ownership_unverified'}
                rules_image = files.fstat(rules_fd)
                note = {'profile': CURSOR_RULE_PROFILE, 'root': str(root), 'root_identity': root_identity,
                    'rules_identity': [rules_image.st_dev, rules_image.st_ino], 'file_identity': current[1],
                    'sha256': hashlib.sha256(current[0]).hexdigest(), 'task': machine[0], 'phase': machine[1]}
                recovered = True
            if (current is None and proof is not None and note is None and not approved_migration
                    and action != 'write'):
                # With no rule there is nothing an unreadable receipt could
                # prove or protect: a write replaces it below instead of
                # leaving the slot blocked for good.
                return {**result, 'reason': 'receipt_unavailable'}
            if note is not None:
                rules_image = files.fstat(rules_fd) if rules_fd is not None else None
                relocated = (recovered or note['root'] != str(root) or note['root_identity'] != root_identity
                    or rules_image is None or note['rules_identity'] != [rules_image.st_dev, rules_image.st_ino]
                    or (current is not None and note['file_identity'] != current[1]))
            valid_task = isinstance(task, str) and re.fullmatch(r"[^\W_][\w\u0300-\u036f./#+-]{0,199}", task)
            if action == "prompt-held":
                held = ("kept" if current is not None and note and valid_task
                        and note["task"] == task and note["phase"] == "prompt" else
                        "absent" if current is None else "preserved")
                if not relocated or held != "kept":
                    return {**result, "state": held}
                action, reconcile_state = "reconcile", held
            if action == "keep" or (action == "write" and not valid_task):
                if note is None:
                    return result
                if current is not None and valid_task and note["task"] == task:
                    if not relocated:
                        return {**result, "state": "kept"}
                    action, reconcile_state = "reconcile", "kept"
                else:
                    action = "retire"
            if action == "write":
                if phase not in ("prompt", "turn", "session"):
                    return result
                if not isinstance(capsule, str) or not capsule.startswith("working:") or "\x00" in capsule:
                    return cursor_rule(repository, "keep", task=task)
                if current is not None and note and phase == "turn" and note["task"] == task and note["phase"] == "prompt":
                    if not relocated:
                        return {**result, "state": "kept"}
                    action, reconcile_state = "reconcile", "kept"
                marker = ("Session context retrieved for a recent prompt" if phase == "prompt"
                          else "Session context as of end of previous turn")
                text = ("---\ndescription: Working memory - current-branch session context\n"
                        "alwaysApply: true\n---\n\n# Working Memory (auto-rendered)\n\n"
                        + marker + " (task: " + task + ").\n"
                        "Retrieved context is not authoritative - verify the source.\n\n```\n"
                        + capsule.rstrip("\n") + "\n```\n")
                try:
                    payload = text.encode("utf-8", "strict")
                except UnicodeError:
                    return cursor_rule(repository, "keep", task=task)
                if len(payload) > CURSOR_RULE_LIMIT:
                    return cursor_rule(repository, "keep", task=task)
                if (current is not None and current[0] == payload
                        and note is not None and (relocated or note["file_identity"] == current[1])):
                    if not relocated:
                        return {**result, "state": "unchanged"}
                    if reconcile_state is None:
                        action, reconcile_state = "reconcile", "unchanged"
            if not _cursor_git_untracked(root) or not intact():
                return result
            # Anchor the existing Brain lock's directory chain as well.
            brain_fd = directory(root_fd, "project-brain", True)
            brain_local_fd = directory(brain_fd, "local", True)
            with _cursor_lock(root, brain_local_fd, **options):
                if (not intact() or not _cursor_git_untracked(root)
                        or _cursor_image(rules_fd, "working-memory.mdc", CURSOR_RULE_LIMIT, **options) != current
                        or _cursor_image(local_fd, "cursor-rule.json", CURSOR_RECEIPT_LIMIT, **options) != proof
                        or not intact()):
                    return result
                if action == "reconcile":
                    # Preserve the proven capsule, task and phase, including
                    # a prompt held over a turn. Only repair its stale locator.
                    result = {**result, "state": "incomplete"}
                    if bank_fd is None:
                        bank_fd = directory(root_fd, 'memory-bank', True)
                    if local_fd is None:
                        local_fd = directory(bank_fd, 'local', True)
                    if not intact():
                        return result
                    rules_image = files.fstat(rules_fd)
                    receipt = json.dumps({**note, "root": str(root), 'root_identity': root_identity,
                                          'rules_identity': [rules_image.st_dev, rules_image.st_ino],
                                          "file_identity": current[1]}).encode("utf-8")
                    if (not intact() or _cursor_image(rules_fd, "working-memory.mdc", CURSOR_RULE_LIMIT, **options) != current):
                        return result
                    _cursor_publish(local_fd, "cursor-rule.json", receipt, proof,
                                    CURSOR_RECEIPT_LIMIT, **options)
                    published_proof = _cursor_image(local_fd, "cursor-rule.json", CURSOR_RECEIPT_LIMIT, **options)
                    if (not intact() or _cursor_image(rules_fd, "working-memory.mdc", CURSOR_RULE_LIMIT, **options) != current
                            or published_proof is None or published_proof[0] != receipt):
                        return result
                    return {**result, "state": reconcile_state}
                if action == "retire":
                    if current is not None and edited:
                        if bank_fd is None:
                            bank_fd = directory(root_fd, 'memory-bank', True)
                        if local_fd is None:
                            local_fd = directory(bank_fd, 'local', True)
                        if not intact():
                            return result
                        result = {**result, "preserved_edit": _cursor_preserve_edit(
                            directory(local_fd, CURSOR_RULE_EDITS, True), current[0], **options)}
                    if current is not None:
                        files.unlink("working-memory.mdc", dir_fd=rules_fd)
                        result = {**result, "state": "incomplete", "bytes_removed": len(current[0])}
                    # Only a receipt this operation proved, in the directory it
                    # opened: with no memory-bank/local the descriptor is None,
                    # and unlink(dir_fd=None) would resolve against the process
                    # working directory - a recovered rule has no receipt at all.
                    if local_fd is not None and proof is not None:
                        try:
                            files.unlink("cursor-rule.json", dir_fd=local_fd)
                        except FileNotFoundError:
                            pass
                    return {**result, "state": "removed" if current is not None else "absent"}
                if cursor_fd is None:
                    cursor_fd = directory(root_fd, ".cursor", True)
                if rules_fd is None:
                    rules_fd = directory(cursor_fd, "rules", True)
                if bank_fd is None:
                    bank_fd = directory(root_fd, "memory-bank", True)
                if local_fd is None:
                    local_fd = directory(bank_fd, "local", True)
                if not intact():
                    return result
                _cursor_sweep_temporaries(rules_fd, **options)
                _cursor_sweep_temporaries(local_fd, **options)
                # Errors after an attempted publication are incomplete, not preserved.
                result = {**result, "state": "incomplete"}
                same_body = current is not None and current[0] == payload
                if not same_body:
                    if edited:
                        result["preserved_edit"] = _cursor_preserve_edit(
                            directory(local_fd, CURSOR_RULE_EDITS, True), current[0], **options)
                    _cursor_publish(rules_fd, "working-memory.mdc", payload, current, CURSOR_RULE_LIMIT, **options)
                published = _cursor_image(rules_fd, "working-memory.mdc", CURSOR_RULE_LIMIT, **options)
                if published is None or published[0] != payload or not intact():
                    return result
                if same_body and published != current:
                    return result
                rules_image = files.fstat(rules_fd)
                receipt = json.dumps({"profile": CURSOR_RULE_PROFILE, "root": str(root),
                    "root_identity": root_identity, "rules_identity": [rules_image.st_dev, rules_image.st_ino],
                    "file_identity": published[1], "sha256": hashlib.sha256(payload).hexdigest(),
                    "task": task, "phase": phase}).encode("utf-8")
                _cursor_publish(local_fd, "cursor-rule.json", receipt, proof, CURSOR_RECEIPT_LIMIT, **options)
                return {**result, "state": "unchanged" if same_body else "written"}
    except (OSError, ValueError, UnicodeError, RecursionError, BrainError, subprocess.TimeoutExpired):
        return {**result, "reason": "linked_directory"} if linked else result


FILE_LINK_INDEX_PROFILE = "file-link-index-v1"
FILE_LINK_GIT_TIMEOUT = 3


def _git_stat_token(image: os.stat_result) -> tuple[int, ...]:
    return (image.st_dev, image.st_ino, image.st_size,
            image.st_mtime_ns, image.st_ctime_ns)


def _git_index_image(path: Path, hash_bytes: int, *, shared: bool = False) -> Optional[tuple[Any, ...]]:
    """Bounded generation hint for Git-managed index writes, not hash verification.

    Shared indexes may be timestamp-freshened by an ordinary Git read. Their
    timestamps must be coherent within this read, but are not a reuse key.
    """
    descriptor = None
    try:
        descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NONBLOCK", 0)
                             | getattr(os, "O_NOFOLLOW", 0))
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode) or before.st_size < 12 + hash_bytes:
            return None
        header = os.read(descriptor, 12)
        if (len(header) != 12 or header[:4] != b"DIRC"
                or int.from_bytes(header[4:8], "big") not in (2, 3, 4)):
            return None
        os.lseek(descriptor, -hash_bytes, os.SEEK_END)
        footer = os.read(descriptor, hash_bytes)
        after = os.fstat(descriptor)
        if (len(footer) != hash_bytes or not any(footer)
                or _git_stat_token(before) != _git_stat_token(after)):
            return None
        identity = _git_stat_token(after)
        return (str(path), identity[:3] if shared else identity, header.hex(), footer.hex())
    except OSError:
        return None
    finally:
        if descriptor is not None:
            os.close(descriptor)


def _file_link_index_signature(repository: Path) -> Optional[tuple[Any, ...]]:
    """Actual Git-selected index/frame and supported object-resolution context.

    Unknown disables reuse, leaving the ordinary batched membership probe in
    charge. There is no Git/SQLite atomic snapshot or object-store integrity
    guarantee. Loose replacement refs and reftables are outside this profile;
    native packed-ref generations are included without parsing their bodies.
    """
    if any(name in os.environ for name in (
        "GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES", "GIT_NAMESPACE",
        "GIT_REPLACE_REF_BASE", "GIT_NO_REPLACE_OBJECTS", "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_COUNT",
    )):
        return None
    try:
        project = workspace_roots.project_root(repository).resolve()
        result = subprocess.run(
            ["git", "-C", str(project), "rev-parse", "--path-format=absolute",
             "--git-path", "index", "--git-path", "refs/replace",
             "--git-path", "packed-refs", "--git-path", "reftable",
             "--show-toplevel", "--show-prefix",
             "--show-object-format=storage", "--shared-index-path"],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=False,
            timeout=FILE_LINK_GIT_TIMEOUT,
        )
        if result.returncode != 0 or len(result.stdout) > 16384:
            return None
        lines = result.stdout.decode("utf-8", "strict").splitlines()
        if (len(lines) not in (7, 8) or lines[6] not in ("sha1", "sha256")
                or any((not line and index != 5) or "\x00" in line or "\r" in line
                       for index, line in enumerate(lines))):
            return None
        paths = [Path(line) for line in lines[:4]]
        if not Path(lines[4]).is_absolute():
            return None
        if not all(path.is_absolute() for path in paths):
            return None
        # Presence, including a symlink, is unsupported; errors are unknown.
        for path in (paths[1], paths[3]):
            try:
                path.lstat()
            except FileNotFoundError:
                continue
            return None
        try:
            packed_stat = paths[2].lstat()
        except FileNotFoundError:
            packed = (str(paths[2]), None)
        else:
            if not stat.S_ISREG(packed_stat.st_mode):
                return None
            packed = (str(paths[2]), _git_stat_token(packed_stat))
        width = 20 if lines[6] == "sha1" else 32
        active = _git_index_image(paths[0], width)
        shared = None
        if len(lines) == 8:
            shared_path = Path(lines[7])
            if not shared_path.is_absolute():
                return None
            shared = _git_index_image(shared_path, width, shared=True)
            if shared is None:
                return None
        if active is None:
            return None
        return (FILE_LINK_INDEX_PROFILE, str(project), lines[6], active, shared, packed,
                lines[4], lines[5])
    except (OSError, ValueError, UnicodeError, subprocess.TimeoutExpired):
        return None


def _cached_tracked_paths(raw: str, key: str, union: list[str]) -> Optional[set[str]]:
    # A reusable value cannot be bigger than the full candidate union. Reject
    # malformed/legacy caches instead of treating a scalar or missing list as
    # authoritative absence. No cache body is emitted to diagnostics.
    maximum = len(json.dumps({"key": key, "tracked": union}))
    if not isinstance(raw, str) or len(raw) > maximum:
        return None
    try:
        cached = json.loads(raw or "{}")
    except (ValueError, RecursionError):
        return None
    if (not isinstance(cached, dict) or set(cached) != {"key", "tracked"}
            or cached["key"] != key or not isinstance(cached["tracked"], list)
            or len(cached["tracked"]) > len(union)
            or any(not isinstance(path, str) for path in cached["tracked"])):
        return None
    tracked = set(cached["tracked"])
    if len(tracked) != len(cached["tracked"]) or not tracked <= set(union):
        return None
    return tracked


def _tracked_paths(repository: Path, paths: list[str]) -> Optional[set[str]]:
    """The paths Git's index holds as files, project-relative; None when Git
    cannot say (no checkout, no git): then no file edge is written.

    One `git cat-file --batch-check` over stdin: no argument-length limit on
    Windows, staged files count without a commit, and `:./path` resolves in
    the project's own frame when the project is a subdirectory of the
    checkout. A directory or a submodule is not a blob, so it drops out.
    """
    if not paths:
        return set()
    project = workspace_roots.project_root(repository)
    try:
        result = subprocess.run(
            ["git", "-C", str(project), "cat-file", "--batch-check=%(objecttype)"],
            input="".join(":./" + path + "\n" for path in paths).encode("utf-8"),
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            check=False,
        )
    except OSError:
        return None
    if result.returncode != 0:
        return None
    lines = result.stdout.decode("utf-8", "replace").splitlines()
    if len(lines) != len(paths):
        return None
    return {path for path, line in zip(paths, lines) if line == "blob"}


def _file_link_rows(path: str, files: list[str], tracked: set[str]) -> list[tuple[str, str, str]]:
    return sorted({(path, file, "file") for file in files if file in tracked})


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
    repository: Path, config: dict[str, Any], stored: Optional[dict[str, str]] = None
) -> tuple[
    list[tuple[str, str, str, str, str]],
    list[tuple[object, ...]],
    list[dict[str, str]],
    list[tuple[str, str, str]],
    dict[str, str],
]:
    """The eligible Brain records and handoffs as index rows, their links, and
    index-state entries to store with them (the tracked-file cache)."""
    # Many records cite the same few files; each is looked up once per pass.
    with brain_runtime.cited_file_memo():
        return _brain_rows(repository, config, stored)


def _brain_rows(repository: Path, config: dict[str, Any], stored: Optional[dict[str, str]]):
    file_candidates: dict[str, tuple[str, int, list[str]]] = {}
    state_updates: dict[str, str] = {}
    documents: list[tuple[str, str, str, str, str]] = []
    metadata_rows: list[tuple[object, ...]] = []
    excluded: list[dict[str, str]] = []
    links: list[tuple[str, str, str]] = []
    eligible_tasks: dict[str, dict[str, Any]] = {}
    problems: list[dict[str, Any]] = []
    claims: dict[str, list[tuple[str, str]]] = {}
    prefixes: set[str] = set()
    for path, record, body, source_text in brain_runtime.iter_record_images(repository, problems=problems):
        # Every record read, eligible or not: a completed task still names
        # the prefix its project's work goes under.
        prefixes |= brain_runtime.work_ticket_prefixes(record)
        if record["type"] == "task" and record.get("status") not in brain_runtime.TERMINAL_STATES:
            claims.setdefault(str(record["external_id"]), []).append(
                (str(record.get("created_at") or ""), str(record["id"])))
        # Eligibility, content and hash all describe the one image the
        # iterator read and validated; a record it skipped is reported as
        # invalid below, and every other record keeps its memory.
        relative = brain_runtime._relative_posix(path, repository)
        try:
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
                _content_hash(source_text),
                record["id"],
                json.dumps(record["conflicts"], sort_keys=True),
                json.dumps(record["source_fingerprints"], sort_keys=True),
                str(record.get("updated_at") or ""),
                float(record.get("confidence", 1.0)),
                record_attestation(record),
            )
        )
        links.extend(_link_rows(relative, record.get("sources")))
        if record["type"] == "task":
            files = [
                file[2:] if file.startswith("./") else file
                for file in record["files"][-FILE_LINK_RECORD_LIMIT:]
                if isinstance(file, str)
            ]
            files = [file for file in files if _file_link_problem(file) is None]
            if files:
                file_candidates[relative] = (record["id"], int(record["revision"]), files)
    handoffs = _require_utf8_record_path(brain_root(repository) / "control" / "handoffs")
    if handoffs.is_dir():
        for path in [_require_utf8_record_path(p) for p in sorted(handoffs.glob("*.md"))]:
            relative = path.relative_to(repository).as_posix()
            try:
                source_text = read_record_text(path)
                handoff, body = parse_markdown_record(path, text=source_text)
                if not is_uuid4(handoff.get('task_id')):
                    raise BrainError('Handoff task identity is invalid')
                task = eligible_tasks[handoff["task_id"]]
                validate_handoff(handoff, task)
            except BrainRecordReadError as error:
                excluded.append({'path': relative, 'reason': 'invalid-utf8'
                    if error.reason == 'invalid-utf8' else 'task-filter-or-invalid'})
                continue
            except (BrainError, KeyError, OSError):
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
                    _content_hash(source_text),
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
    if file_candidates:
        union = sorted({file for _, _, files in file_candidates.values() for file in files})
        signature = _file_link_index_signature(repository)
        key = hashlib.sha256(json.dumps([
            signature, union,
            sorted((identifier, revision) for identifier, revision, _ in file_candidates.values()),
        ]).encode("utf-8")).hexdigest()
        tracked = (_cached_tracked_paths((stored or {}).get(FILE_LINK_TRACKED_KEY) or "{}", key, union)
                   if signature is not None else None)
        if tracked is None:
            # Clear old proof on unavailable/failed/unstable capture. Fresh
            # successful membership is usable when no signature was known,
            # but can be cached only after a stable before/after comparison.
            state_updates[FILE_LINK_TRACKED_KEY] = "{}"
            tracked = _tracked_paths(repository, union)
            if signature is not None and tracked is not None:
                if _file_link_index_signature(repository) == signature:
                    state_updates[FILE_LINK_TRACKED_KEY] = json.dumps(
                        {"key": key, "tracked": sorted(tracked)}
                    )
                else:
                    tracked = None
        for relative, (_, _, files) in file_candidates.items():
            links.extend(_file_link_rows(relative, files, tracked or set()))
    for problem in problems:
        try:
            relative = problem["path"].relative_to(repository).as_posix()
        except ValueError:
            continue
        excluded.append({"path": relative, "reason": "invalid"})
    state_updates[TASK_TWINS_KEY] = json.dumps(
        {external: [identifier for _, identifier in sorted(ids)]
         for external, ids in claims.items() if len(ids) > 1}, sort_keys=True)
    state_updates[TICKET_PREFIXES_KEY] = json.dumps(sorted(prefixes))
    return documents, metadata_rows, excluded, links, state_updates


def _brain_rows_digest(documents, metadata, links, state_updates) -> str:
    """One digest of every Brain row a refresh would write, state included."""
    digest = hashlib.sha256()
    for rows in (documents, metadata, sorted(links), sorted(state_updates.items())):
        digest.update(b"\x1e")
        for row in rows:
            digest.update(repr(tuple(row)).encode("utf-8", "surrogatepass"))
            digest.update(b"\x1f")
    return digest.hexdigest()


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


# ---------------------------------------------------------------------------
# Identifier forms
#
# A codebase says `getUserById`; a person asks "get user by id". FTS5's
# tokenizer keeps the camelCase name as one token, so the two never meet. The
# reverse also happens: a prompt names `getUserById` while a spec spells the
# operation out. The derived table below records both directions for the
# documents already in the index. It is an index, never a source: deleting it
# loses only a query expansion, and every admission rule still runs after the
# expanded query returns its candidates.
# ---------------------------------------------------------------------------

IDENTIFIER_TERMS_KEY = "identifier_terms_ready"
IDENTIFIER_ROW_LIMIT = 4096
QUERY_EXPANSION_LIMIT = 48
_IDENTIFIER_TOKEN = re.compile(r"[A-Za-z_\\][A-Za-z0-9_\\]*(?:\.[A-Za-z0-9_]+)*")
_IDENTIFIER_PART = re.compile(r"[A-Z]+(?=[A-Z][a-z]|\d|\b)|[A-Z]?[a-z]+|[A-Z]+|\d+")
_IDENTIFIER_HINT = re.compile(r"[a-z][A-Z]|_|[A-Z]{2,}[a-z]|\\")
_FILE_SUFFIXES = (
    ".php", ".py", ".md", ".json", ".js", ".mjs", ".cjs", ".ts", ".tsx", ".jsx",
    ".java", ".kt", ".rb", ".go", ".rs", ".c", ".h", ".cpp", ".hpp", ".cs",
    ".yaml", ".yml", ".xml", ".sql", ".sh", ".vue", ".svelte", ".css", ".scss",
)


def _identifier_stem(token: str) -> str:
    """A file name's identifier is its stem, not its extension.

    `InvoiceTotal.php` expands to "invoice total php" otherwise, and a query
    for "invoice total" never asks the table for it. The extension is also not
    part of the class name a person searches for.
    """
    folded = token.casefold()
    for suffix in _FILE_SUFFIXES:
        if folded.endswith(suffix) and len(token) > len(suffix):
            return token[: -len(suffix)]
    return token


def _identifier_parts(value: str) -> list[str]:
    parts: list[str] = []
    for segment in re.split(r"[_\\.]+", value):
        if segment:
            parts.extend(_IDENTIFIER_PART.findall(segment))
    return [part.casefold() for part in parts if part]


def identifier_form(token: str) -> Optional[str]:
    """The words a code identifier is made of, or None for an ordinary word.

    Deliberately conservative: only a token that actually carries a boundary
    (camelCase, snake_case, a namespace separator or a dot) earns an
    expansion. Expanding ordinary prose would add every English word to every
    query and turn a precise lexical index into a bag of noise.
    """
    if len(token) < 4 or len(token) > 200:
        return None
    if not re.search(r"[_\\]|\.|[a-z][A-Z]|[A-Z]{2,}[a-z]", token):
        return None
    parts = _identifier_parts(token)
    if len(parts) < 2:
        return None
    expansion = " ".join(parts)
    if expansion == token.casefold():
        return None
    return expansion


def _identifier_text(content: str) -> str:
    """The part of a document that makes a claim, not its citations.

    Frontmatter and generated `Sources`/`References` sections name files and
    URLs; an identifier there is provenance, not prose. Scanning them made a
    query about "aster willow" add `AsterWillow` from a citation footer, and
    the excerpt selector then quoted that footer instead of the fact.
    """
    text = content
    if text.startswith("---\n"):
        end = text.find("\n---\n", 4)
        if end != -1:
            text = text[end + 5:]
    return re.sub(
        r"(?ims)^##[ \t]+(?:Sources|References|Files)[ \t]*\n.*?(?=^##[ \t]+|\Z)",
        "",
        text,
    )


def identifier_terms(path: str, content: str) -> list[tuple[str, str]]:
    """(identifier, expansion) pairs for one indexed document.

    Only content is scanned. A path-derived form would be a citation, not a
    statement: adding a source file's `AsterWillow` to a query about "aster
    willow" made the excerpt selector quote the `Sources` footer instead of
    the fact, because the footer is the only place that path text appears.
    A query that already names the identifier is handled directly, without a
    row, and a file name the prose never repeats was not a retrieval signal.
    """
    if _IDENTIFIER_HINT.search(content) is None:
        # Ordinary prose has no camelCase, snake_case or namespace boundary;
        # scanning every word of every document on the cold-index path cost
        # more than the derived table is worth. One cheap pass decides.
        return []
    text = _identifier_text(content)
    if _IDENTIFIER_HINT.search(text) is None:
        return []
    found: dict[tuple[str, str], None] = {}

    def add(token: str) -> None:
        stem = _identifier_stem(token)
        expansion = identifier_form(stem)
        if expansion is not None:
            found[(stem, expansion)] = None

    for match in _IDENTIFIER_TOKEN.finditer(text):
        add(match.group(0))
        if len(found) >= IDENTIFIER_ROW_LIMIT:
            break
    return list(found)


def ensure_identifier_table(connection: sqlite3.Connection) -> bool:
    """Create the derived identifier table; True when this call created it.

    `IF NOT EXISTS` on both objects is what makes two concurrent refreshes
    safe: each may see the table absent and both may run the DDL, but only one
    creates it. A crash between the table and its index is repaired on the
    next call instead of leaving a permanent full scan.
    """
    if connection.execute(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'identifier_terms'"
    ).fetchone() is not None:
        indexes = {
            str(row[1]) for row in connection.execute(
                "PRAGMA index_list('identifier_terms')"
            )
        }
        if "identifier_terms_expansion" not in indexes:
            connection.execute(
                "CREATE INDEX IF NOT EXISTS identifier_terms_expansion "
                "ON identifier_terms(expansion)"
            )
        return False
    connection.execute(
        """
        CREATE TABLE IF NOT EXISTS identifier_terms(
            path TEXT NOT NULL,
            identifier TEXT NOT NULL,
            expansion TEXT NOT NULL,
            PRIMARY KEY(path, identifier, expansion)
        )
        """
    )
    connection.execute(
        "CREATE INDEX IF NOT EXISTS identifier_terms_expansion "
        "ON identifier_terms(expansion)"
    )
    return True


def _write_identifier_rows(connection: sqlite3.Connection, rows) -> None:
    connection.executemany(
        "INSERT OR REPLACE INTO identifier_terms(path, identifier, expansion) "
        "VALUES (?, ?, ?)",
        (
            (path, identifier, expansion)
            for path, terms in rows
            for identifier, expansion in terms
        ),
    )


def rebuild_identifier_terms(connection: sqlite3.Connection, documents=None) -> None:
    """Rebuild the whole derived table from the index or supplied rows.

    A full index already holds every row in ``documents``; re-reading them
    from SQLite and scanning the same text a second time was the cold-hook
    cost. Only a migration with retained documents, which are not in the
    changed list, falls back to the database.
    """
    connection.execute("DELETE FROM identifier_terms")
    if documents is None:
        rows = connection.execute("SELECT path, content FROM documents").fetchall()
        documents = [(str(row[0]), None, None, None, None, str(row[1] or "")) for row in rows]
    _write_identifier_rows(
        connection,
        [
            (str(row[0]), identifier_terms(str(row[0]), str(row[5] or "")))
            for row in documents
        ],
    )


def update_identifier_terms(
    connection: sqlite3.Connection,
    documents,
    removed: set[str],
) -> None:
    """Replace the rows of changed documents and drop removed ones.

    ``documents`` are the rows just inserted, in the same
    ``(path, layer, kind, title, summary, content)`` shape the index uses.
    """
    changed = sorted({str(row[0]) for row in documents})
    for path in sorted({*removed, *changed}):
        connection.execute("DELETE FROM identifier_terms WHERE path = ?", (path,))
    _write_identifier_rows(
        connection,
        [
            (str(row[0]), identifier_terms(str(row[0]), str(row[5] or "")))
            for row in documents
        ],
    )


def _query_ngrams(tokens: list[str]) -> list[str]:
    """Contiguous phrases, longest first, for identifier-form lookup.

    Longer first matters: an expansion such as "find user by email" is four
    words, and a two/three-gram-only window never asked the table for it. The
    cap keeps a long prompt from issuing hundreds of lookups; the phrases a
    person actually typed are the ones near the front and the full span.
    """
    values = [token.casefold() for token in tokens]
    grams: list[str] = []
    for size in range(min(6, len(values)), 1, -1):
        for start in range(0, len(values) - size + 1):
            grams.append(" ".join(values[start:start + size]))
            if len(grams) >= 64:
                return grams
    return grams


def expand_query_tokens(
    connection: sqlite3.Connection, tokens: list[str]
) -> list[str]:
    """Add the identifier a query phrase spells out, when the index has one.

    A person asks "find user by email"; the code says `findUserByEmail`. The
    table maps the expansion back to the identifier, and the main FTS query
    finds the document that names it. The other direction - expanding an
    identifier into its words - is deliberately not done: it admits every
    document that shares two common subwords, and a precise identifier hit is
    already served by the exact token.

    Expansions are candidates only. They do not admit a document on their
    own: `informative_tokens`, `is_relevant`, authority, privacy and source
    checks run on the expanded list exactly as they did before.
    """
    result = list(tokens)
    seen = {token.casefold() for token in result}
    limit = len(tokens) + QUERY_EXPANSION_LIMIT
    memo = workspace_roots.command_memo("query-expansion")
    key = None
    if memo is not None:
        try:
            generation = load_index_state(connection).get(INDEX_GENERATION_KEY, "")
        except sqlite3.Error:
            memo = None
        else:
            key = (generation, tuple(tokens))
            cached = memo.get(key)
            if isinstance(cached, tuple):
                return list(cached)

    def add(token: str) -> None:
        folded = token.casefold()
        if folded and folded not in seen and len(result) < limit:
            seen.add(folded)
            result.append(token)

    grams = _query_ngrams(tokens)
    if grams:
        try:
            placeholders = ",".join("?" for _ in grams)
            rows = connection.execute(
                "SELECT DISTINCT identifier FROM identifier_terms "
                f"WHERE expansion IN ({placeholders})",
                grams,
            ).fetchall()
        except sqlite3.Error:
            rows = []
        for row in rows:
            add(str(row[0]))
    if memo is not None and key is not None:
        memo[key] = tuple(result)
    return result


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
    brain_documents, brain_metadata, excluded, brain_links, state_updates = _brain_documents(
        repository, config, stored
    )
    brain_digest = _brain_rows_digest(brain_documents, brain_metadata, brain_links, state_updates)
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
    identifier_created = ensure_identifier_table(connection)
    rebuild_identifiers = identifier_created or stored.get(IDENTIFIER_TERMS_KEY) != "1"
    existing = {
        row[0] for row in connection.execute("SELECT path FROM documents").fetchall()
    }
    keep = set(retained or {})
    if not keep:
        # A full rebuild has no retained rows to compare against; the
        # identifier table is derived from the same documents and must not
        # outlive a replaced index with rows for paths that no longer exist.
        rebuild_identifiers = True
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
        and not legacy_documents
        and existing == keep | {row[0] for row in brain_documents}
        and (not brain_documents or stored.get(INDEX_BRAIN_ROWS_KEY) == brain_digest)
        and cached_rows == len(keep)
        and fingerprints is not None
        and INDEX_PARITY_KEY in stored
        and all(stored.get(key) == value for key, value in fingerprints.items())
        and not rebuild_identifiers
    ):
        # Nothing observable changed, so a per-request refresh costs reads only:
        # the Brain rows rebuilt from the records match the stored ones, and the
        # generation - with every cache derived from it - stands.
        total, layers = _layer_counts(connection)
        return {
            "documents": total,
            "removed": 0,
            "reused": len(keep),
            "incremental": True,
            "layers": layers,
            "brain": len(brain_documents),
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
        removed_paths = existing - (keep | {str(row[0]) for row in documents})
        if rebuild_identifiers:
            # A full rebuild has every document in `documents`; a migration
            # with retained rows does not and must read them back.
            rebuild_identifier_terms(connection, documents if not keep else None)
        else:
            update_identifier_terms(connection, documents, removed_paths)
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
                INDEX_BRAIN_ROWS_KEY: brain_digest,
                IDENTIFIER_TERMS_KEY: "1",
                **state_updates,
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


# A seed glossary for the Russian technical questions this project actually
# receives. It is deliberately small, and it only adds to Cyrillic words: an
# English query, a code identifier or a ticket is left untouched, and the
# Russian word stays in the query beside its English term - replacing it lost
# the Russian documents that answer in Russian. Each entry is a pattern over
# the whole folded word: a bare prefix took "логика" and "логин" for log,
# "старт" for legacy and "полезно" for field. This is a bridge, not a
# translation service; a project with a different language should extend
# `_QUERY_ALIASES` rather than expect a model call from the indexing hot path.
_QUERY_ALIASES = tuple(
    (re.compile(pattern), english)
    for pattern, english in (
        (r"предотвращ\w*", "prevent"),
        (r"подделк\w*", "spoof"),
        (r"вложенн\w*", "nested"),
        (r"неизвестн\w*", "unknown"),
        (r"записыва\w*", "written"),
        (r"отбрасыва\w*", "drop"),
        (r"просмотр\w*", "preview"),
        (r"временн\w*", "temporary"),
        # "действительный" and its forms - not "действительно" (indeed) or
        # "действие" (an action).
        (r"действительн(?!о$)\w*", "validity"),
        (r"измен\w*", "change"),
        (r"админк\w*", "admin"),
        # "старый" and its forms - not "старт" or "стараться".
        (r"стар(?:ый|ая|ое|ые|ого|ой|ому|ым|ых|ыми|ую|ом)", "legacy"),
        (r"файл\w*", "files"),
        (r"свойств\w*", "properties"),
        (r"сущност\w*", "entity"),
        (r"строк(?:а|и|у|ой|ою|е|ам|ами|ах)?", "lines"),
        (r"спис(?:ок|ка|ку|ком|ке|ки|ков|кам|ками|ках)", "list"),
        (r"пут(?:ь|и|ем|ём|ям|ями|ях)", "path"),
        # "лог", "логирование", "логгер" - not "логика" or "логин".
        (r"лог(?:и|а|ов|е|у|ом|ам|ами|ах)?|логир\w*|логгер\w*", "log"),
        (r"пол(?:е|я|ей|ю|ям|ем|ями|ях)", "field"),
        (r"им(?:я|ени|енем|ена|ён|ен|енам|енами|енах)", "name"),
        (r"срок\w*", "ttl"),
    )
)


def _query_alias(token: str) -> list[str]:
    """The token, followed by the English term a Russian technical word names."""
    if not re.search(r"[А-Яа-яЁё]", token):
        return [token]
    folded = token.casefold()
    for pattern, english in _QUERY_ALIASES:
        if pattern.fullmatch(folded):
            return [token, *english.split()]
    return [token]


def query_tokens(query: str) -> list[str]:
    tokens: list[str] = []
    seen: set[str] = set()
    for token in re.findall(r"\w+", query, flags=re.UNICODE):
        for candidate in _query_alias(token):
            folded = candidate.casefold()
            if folded in seen:
                continue
            seen.add(folded)
            tokens.append(candidate)
    if not tokens:
        raise RetrievalError("Search query must contain a word")
    return tokens


def fts_query_tokens(tokens: Iterable[str]) -> str:
    """The FTS5 expression for a token list, preserving caller order."""
    return " OR ".join(f'"{token}"' for token in tokens)


def fts_query(query: str) -> str:
    return fts_query_tokens(query_tokens(query))


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


def _ticket_ids(text: str, known: Optional[Iterable[str]] = None) -> set[str]:
    """Ticket identifiers in a request or an event title, case-folded.

    The shape is `CMS-741` - a hyphen, never a space, so "PHP 8.2" is not a
    ticket - and a standard's prefix (`UTF-8`, `PSR-4`, `ISO-8601`) never
    counts. ``known`` are the prefixes the project files its work under:
    when there are any, only they count (see brain_runtime.ticket_ids).
    """
    return {value.lower() for value in brain_runtime.ticket_ids(text, known)}


def _known_ticket_prefixes(
    connection: sqlite3.Connection, task: Optional[dict[str, Any]] = None
) -> frozenset[str]:
    """The ticket prefixes the index found the project's work filed under,
    and the bound task's own; empty when there are none."""
    try:
        stored = json.loads(load_index_state(connection).get(TICKET_PREFIXES_KEY) or "[]")
    except (sqlite3.Error, TypeError, ValueError):
        stored = []
    prefixes = {value for value in stored if isinstance(value, str)} if isinstance(stored, list) else set()
    if isinstance(task, dict):
        prefixes |= brain_runtime.work_ticket_prefixes({**task, "type": "task"})
    return frozenset(prefixes)


def _request_tickets(
    text: str, task: Optional[dict[str, Any]], known: Optional[Iterable[str]] = None
) -> set[str]:
    """Tickets the request names, or the working task names when it names none.

    A prompt often asks a question without repeating its ticket; the governed
    task it continues carries it. The task's external id, title and goal are
    read in memory only.
    """
    tickets = _ticket_ids(text, known)
    if isinstance(task, dict):
        tickets |= _ticket_ids(" ".join(
            str(task.get(key) or "") for key in ("external_id", "title", "goal")
        ), known)
    return tickets


def _ticket_filter(
    items: list[dict[str, Any]], tickets: set[str], *,
    connection: Optional[sqlite3.Connection] = None,
    repository: Optional[Path] = None,
    request_text: str = "",
    known: Optional[Iterable[str]] = None,
) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    """Withhold a durable chunk or finding that names only another ticket.

    A document with no ticket, or with one the request also names, is kept;
    a document whose tickets are all different is evidence about other work.
    Events keep their own slot rule (they may be the same subject without the
    ticket), and canonical project files are not filtered here. ``known`` are
    the project's ticket prefixes, read the same way on both sides.
    """
    if not tickets:
        return items, []
    kept: list[dict[str, Any]] = []
    excluded: list[dict[str, str]] = []
    for item in items:
        path = str(item.get("path") or "")
        kind = item.get("kind")
        selected = item.get("category") == "durable" or kind == "brain-finding"
        changelog = kind == "changelog"
        if not selected and not changelog:
            kept.append(item)
            continue
        image = item.get("_indexed_policy_image")
        content = str(image[2]) if image and len(image) > 2 else ""
        if not content and connection is not None:
            # A path-linked record reached the selection without a lexical
            # quote; its body is still in the index.
            try:
                row = connection.execute(
                    "SELECT content FROM documents WHERE path = ?", (path,)
                ).fetchone()
            except sqlite3.Error:
                row = None
            content = str(row[0] or "") if row is not None else ""
        document_tickets = _ticket_ids(str(item.get("title") or "") + "\n" + content, known)
        if changelog:
            # The changelog is release context, not a ticket record: keep it
            # when the request names it, and otherwise only when it mentions
            # a ticket the request names. Its body is the only place the
            # ticket list lives, and one selected changelog is read, not
            # every candidate.
            if not tickets or "changelog" in request_text.casefold():
                kept.append(item)
                continue
            if document_tickets and not (document_tickets & tickets):
                excluded.append({"path": path, "reason": "ticket-mismatch"})
                continue
            kept.append(item)
            continue
        if not document_tickets and item.get("kind") == "brain-finding" and repository is not None:
            # The indexed body omits the record's frontmatter, where a
            # finding's external id carries its ticket; the tracked file has
            # it. Only a selected finding is read, never every candidate.
            try:
                source = (repository / path)
                root = repository.resolve()
                if source.is_file() and source.resolve().is_relative_to(root):
                    document_tickets = _ticket_ids(
                        source.read_text(encoding="utf-8", errors="ignore"), known)
            except (OSError, ValueError):
                document_tickets = set()
        if document_tickets and not (document_tickets & tickets):
            excluded.append({"path": path, "reason": "ticket-mismatch"})
            continue
        kept.append(item)
    return kept, excluded


def _anchor_names_one_thing(term: str, frequency: Optional[int]) -> bool:
    """Whether a distinctive term is a name rather than a shared label.

    A name appears in one to three documents, or carries a digit (a ticket,
    version or record number). A CamelCase word in five documents is a
    directory many records cite; it admits a neighbouring ticket's document
    on that shared word alone.
    """
    return (
        frequency is None
        or frequency <= ANCHOR_DOCUMENT_LIMIT
        or re.search(r"\d", term) is not None
    )


def _specific_distinctive(
    distinctive: dict[str, list[str]], frequencies: dict[str, Optional[int]]
) -> dict[str, list[str]]:
    """Distinctive terms kept only where they name something."""
    return {
        path: [
            term for term in terms
            if _anchor_names_one_thing(term, frequencies.get(term))
        ]
        for path, terms in distinctive.items()
    }


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


def episode_item(row: Any) -> dict[str, Any]:
    """A local episode as the capsule carries it."""
    return {
        "id": row["id"],
        "layer": "episodic",
        "summary": row["summary"],
        "outcome": row["outcome"],
        "files": json.loads(row["files"]),
        "verification": json.loads(row["verification"]),
        "sources": json.loads(row["sources"]),
        "created_at": row["created_at"],
    }


def history_query(connection: sqlite3.Connection, text: str) -> Optional[dict[str, Any]]:
    """The terms of a request that reach recorded history, and the bar for it.

    Recorded history - Project Brain events and local episodes - is matched by
    its subject only (an event's title and goal, an episode's summary and
    outcome) against every informative term of the whole request, not the 24
    the main query keeps: an event's words also occur in the changelog, the
    README and the task documents, so they are never among the corpus-rarest,
    and the distillation dropped every subject term of all 13 answer-holding
    events on the evaluated prompts.

    An item is admitted when it covers `required_coverage` distinct tokenizer
    concepts and its matched raw terms meet the existing weighted floor.
    Porter variants count once toward coverage; raw weights and ranking stay
    unchanged. The weights are those excerpt_weights uses; no constant is
    tuned. Of the terms that reach any history subject the
    CAPSULE_PROMPT_TERM_LIMIT rarest count, so a long prompt cannot win by
    length. Nothing here is persisted.
    """
    try:
        tokens = evidence_tokens(query_tokens(text))
    except RetrievalError:
        return None
    if not tokens:
        return None
    try:
        documents = connection.execute("SELECT COUNT(*) FROM documents").fetchone()[0]
        events_indexed = connection.execute(
            "SELECT COUNT(*) FROM documents WHERE kind = ?", (HISTORY_KIND,)
        ).fetchone()[0]
    except sqlite3.Error:
        return None
    try:
        episodes = connection.execute("SELECT COUNT(*) FROM episodes").fetchone()[0]
    except sqlite3.Error:
        episodes = 0
    total = documents + episodes
    if not total or not (events_indexed or episodes):
        return None
    frequencies = token_document_frequencies(connection, tokens)
    events: dict[str, set[str]] = {}
    recorded: dict[str, set[int]] = {}
    counts: dict[str, int] = {}
    for token in tokens:
        events[token] = set()
        recorded[token] = set()
        if events_indexed and frequencies.get(token):
            try:
                events[token] = {
                    row[0] for row in connection.execute(
                        "SELECT path FROM documents WHERE documents MATCH ? AND kind = ?",
                        (f'summary : "{token}"', HISTORY_KIND),
                    )
                }
            except sqlite3.Error:
                pass
        if episodes:
            try:
                recorded[token] = {
                    row[0] for row in connection.execute(
                        "SELECT rowid FROM episodes WHERE episodes MATCH ?",
                        (f'{{summary outcome}} : "{token}"',),
                    )
                }
            except sqlite3.Error:
                pass
        # `complete` writes an event and an equivalent episode: counting both
        # would make the event's own words look common, so an episode adds to
        # a term's frequency only where no document has the term.
        counts[token] = frequencies.get(token) or len(recorded[token])
    pool = [token for token in tokens if 0 < counts[token] <= total * STOPWORD_DOCUMENT_RATIO]
    reach = [token for token in pool if events[token] or recorded[token]]
    if not reach:
        return None
    position = {token: index for index, token in enumerate(tokens)}
    terms = sorted(reach, key=lambda token: (counts[token], position[token]))[
        :CAPSULE_PROMPT_TERM_LIMIT
    ]
    terms.sort(key=position.__getitem__)
    minimum = required_coverage(pool)
    # Porter aliases are one concept, even when the query spells it twice.
    # Keep the raw term window and weights for ranking; only the admission
    # rule requires independent concepts. Usually two selected families
    # already prove that the pool has at least two.
    forms = term_forms(terms)
    families = {forms[token] or token.casefold() for token in terms}
    if len(families) < minimum:
        for token in pool:
            families.add(term_forms([token])[token] or token.casefold())
            if len(families) >= minimum:
                break
        minimum = required_coverage(list(families))
    weights = {
        token: max(0.05, math.log((total + 1) / (counts[token] + 0.5))) for token in terms
    }
    # Two floors. An ordinary word match keeps the strict one - the weight of
    # a term unique in the corpus - so a weak event still leaves the slot
    # empty. A match that includes a name-shaped term (a ticket or record
    # number) may clear the coverage floor instead: a ticket in seventeen
    # documents and its number in six must still reach the event that names
    # it, and the old floor made that impossible - six CMS-741 events matched
    # both concepts and none was admitted.
    strict_floor = minimum * max(0.05, math.log((total + 1) / 1.5))
    anchor_floor = sum(sorted(weights.values())[:min(minimum, len(weights))])
    return {
        "terms": terms,
        "weights": weights,
        "minimum": minimum,
        "floor": strict_floor,
        "anchor_floor": anchor_floor,
        "tickets": _ticket_ids(text),
        "events": {token: events[token] for token in terms},
        "episodes": {token: recorded[token] for token in terms},
    }


def _history_candidates(
    connection: sqlite3.Connection, history: dict[str, Any],
    *, trace: Optional[RetrievalTrace] = None,
) -> tuple[list[dict[str, Any]], list[tuple[float, float, dict[str, Any]]]]:
    """Admitted events, ranked like candidates, and admitted episodes as
    (ranked score, matched weight, episode)."""
    forms = term_forms(history["terms"])

    def admitted(members: dict[str, set[Any]]) -> dict[Any, float]:
        concepts: dict[Any, set[str]] = {}
        mass: dict[Any, float] = {}
        anchors: set[Any] = set()
        for token, found in members.items():
            anchored = is_anchor(token)
            for member in found:
                concepts.setdefault(member, set()).add(forms[token] or token.casefold())
                mass[member] = mass.get(member, 0.0) + history["weights"][token]
                if anchored:
                    anchors.add(member)
        # The tolerance keeps a matched weight exactly at the floor on the
        # right side of floating-point rounding.
        return {
            member: mass[member]
            for member in concepts
            if len(concepts[member]) >= history["minimum"]
            and mass[member] + 1e-9 >= (
                history["anchor_floor"] if member in anchors else history["floor"]
            )
        }

    event_mass = admitted(history["events"])
    events: list[dict[str, Any]] = []
    if event_mass:
        marks = ",".join("?" for _ in event_mass)
        rows = connection.execute(
            f"""
            SELECT
                d.rowid AS document_rowid, d.path, d.layer, d.kind, d.title,
                m.category, m.privacy, m.owner, m.authority, m.lifecycle,
                m.source_hash, m.record_id, m.conflicts,
                m.source_fingerprints, m.updated_at, m.confidence, m.attestation
            FROM documents AS d
            JOIN document_metadata AS m ON m.path = d.path
            WHERE d.kind = ? AND d.path IN ({marks})
            """,
            (HISTORY_KIND, *sorted(event_mass)),
        ).fetchall()
        for row in rows:
            item = dict(row)
            item["conflicts"] = json.loads(item["conflicts"])
            item["source_fingerprints"] = json.loads(item["source_fingerprints"])
            item["match"] = "covered"
            item["score"] = -event_mass[item["path"]]
            item["adjusted_score"] = event_mass[item["path"]] * ranking_weight(
                item["authority"], item["confidence"], item["updated_at"]
            )
            events.append(item)
            if trace is not None:
                trace.observe(item, "history", "admitted", score=item["adjusted_score"])
        events.sort(key=lambda item: (-item["adjusted_score"], item["path"]))
        prompt_tickets = history.get("tickets") or set()
        known = history.get("known_tickets")
        for item in events:
            # Internal: the event slot prefers the named ticket when one is
            # admitted, but a neighbouring ticket's event remains available
            # when no event carries it.
            item["_ticket_match"] = bool(
                prompt_tickets & _ticket_ids(str(item.get("title") or ""), known)
            )
        for rank, item in enumerate(events, start=1):
            item["rank"] = rank
        _quote_candidates(connection, events, " ".join(history["terms"]), history["terms"])
    episode_mass = admitted(history["episodes"])
    episodes: list[tuple[float, float, dict[str, Any]]] = []
    if episode_mass:
        marks = ",".join("?" for _ in episode_mass)
        rows = connection.execute(
            "SELECT rowid AS id, summary, outcome, files, verification, sources, created_at "
            f"FROM episodes WHERE rowid IN ({marks})",
            tuple(sorted(episode_mass)),
        ).fetchall()
        for row in rows:
            episode = episode_item(row)
            mass = episode_mass[row["id"]]
            episodes.append((mass * ranking_weight("observed", 1.0, row["created_at"]), mass, episode))
        episodes.sort(key=lambda entry: (-entry[0], -int(entry[2]["id"])))
    return events, episodes


def _history_series_pilot() -> bool:
    """Opt-in pilot: a same-ticket event series may share the history slot."""
    return os.environ.get("CONTEXT_EVENT_SERIES", "").strip().casefold() in {
        "1", "true", "yes",
    }


def _same_ticket_series(
    events: list[dict[str, Any]], known: Optional[Iterable[str]] = None
) -> list[dict[str, Any]]:
    """The largest same-ticket event series, in the existing event order."""
    groups: dict[str, list[dict[str, Any]]] = {}
    for item in events:
        for ticket in _ticket_ids(str(item.get("title") or ""), known):
            groups.setdefault(ticket, []).append(item)
    if not groups:
        return []
    series = max(groups.values(), key=len)
    return series if len(series) > 1 else []


def _best_history_event(
    repository: Path, events: list[dict[str, Any]], config: dict[str, Any],
    *, trace: Optional[RetrievalTrace] = None, limit: int = 1, connection=None,
    known: Optional[Iterable[str]] = None,
) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    """The best admitted events that pass the runtime filters.

    ``limit`` is the event slot's capacity: one, or two events of the ticket
    the request names (and the opt-in series pilot). The filters only remove
    a candidate or lower its score - an event whose cited file changed keeps
    half - so once the next candidate's unfiltered score cannot beat the
    weakest one kept, no later one can: usually one or two files are read,
    however many events exist. Taking the first ``limit`` that passed instead
    let a halved 12.5 (6.3) hold the slot against a fresh 9.6.
    """
    if limit == 1 and _history_series_pilot():
        series = _same_ticket_series(events, known)
        if series:
            events = series
            limit = min(CAPSULE_EVENT_LIMIT, len(series))
    capacity = max(1, limit)
    kept_events: list[dict[str, Any]] = []
    excluded: list[dict[str, str]] = []
    for item in events:
        if (len(kept_events) >= capacity
                and item["adjusted_score"] <= kept_events[-1]["adjusted_score"]):
            break
        kept, dropped = _runtime_filter(
            repository, [item], config, trace=trace, connection=connection
        )
        excluded.extend(dropped)
        if kept:
            kept_events.append(kept[0])
            # Stable: of equal scores the earlier candidate stays ahead.
            kept_events.sort(key=lambda entry: -entry["adjusted_score"])
            del kept_events[capacity:]
    return kept_events, excluded


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


_NO_HELD_INDEX_IMAGE = object()
# FTS5 marks a document by tokenizing all of it, about a third of a second a
# megabyte, and a prompt marks each item it delivers more than once: a 20 MB
# reference under docs/ that a question matched held the prompt for minutes,
# and every such prompt lost its memory to the hook's budget. A document
# longer than this is marked only in the stretch where the query's words
# cluster; it is still matched and ranked over all of its text.
MARKED_DOCUMENT_CHARACTERS = 65536


def _answering_stretch(text: str, tokens: list[str], limit: int = MARKED_DOCUMENT_CHARACTERS) -> str:
    """The part of a long document a query's words point at: the section
    (from its heading) where most distinct words, then most occurrences,
    fall, cut to `limit` characters around its first match."""
    forms = term_forms(tokens)
    stems = sorted({stem for value in forms.values() for stem in value.split() if stem}
                   | {token.casefold() for token in tokens}, key=len, reverse=True)
    if not stems:
        return text[:limit]
    words = re.compile(r"\b(?:" + "|".join(re.escape(stem) for stem in stems) + r")\w*", re.IGNORECASE)
    headings = [0] + [match.start() for match in re.finditer(r"(?m)^#{1,6}[ \t]", text)]
    best = None
    for index, start in enumerate(headings):
        end = headings[index + 1] if index + 1 < len(headings) else len(text)
        if end <= start:
            continue
        found = words.findall(text, start, end)
        if not found:
            continue
        score = (len({word.casefold() for word in set(found)}), len(found), -start)
        if best is None or score > best[0]:
            best = (score, start, end)
    if best is None:
        return text[:limit]
    _, start, end = best
    if end - start <= limit:
        return text[start:end]
    # A section longer than the limit: its heading, then the window where the
    # most distinct words of the query meet.
    heading = text[start:text.find("\n", start) + 1] if text.startswith("#", start) else ""
    body, span = start + len(heading), limit - len(heading)
    found = [(match.start(), match.group(0).casefold())
             for _, match in zip(range(50000), words.finditer(text, body, end))]
    if not found:
        return heading + text[body:body + span]
    counts: dict[str, int] = {}
    best_score, best_begin, tail = (0, 0), found[0][0], 0
    for index, (position, word) in enumerate(found):
        counts[word] = counts.get(word, 0) + 1
        while found[tail][0] < position - span // 2:
            counts[found[tail][1]] -= 1
            if not counts[found[tail][1]]:
                del counts[found[tail][1]]
            tail += 1
        score = (len(counts), index - tail + 1)
        if score > best_score:
            best_score, best_begin = score, found[tail][0]
    begin = max(body, best_begin - span // 4)
    return heading + text[begin:begin + span]


def _marked_stretch(image, tokens) -> str:
    """Marks over the answering stretch of a long document, found once per
    command for its text and words."""
    title, summary, content = image
    memo = workspace_roots.command_memo("answering-stretches")
    key = (hash(content), len(content), tuple(tokens))
    stretch = memo.get(key) if memo is not None else None
    if stretch is None:
        stretch = _answering_stretch(content, tokens)
        if memo is not None:
            memo[key] = stretch
    return _held_marked_document((title, summary, stretch), tokens)


def _held_marked_document(image, tokens):
    """FTS marks over admitted bytes when the disposable cache has moved."""
    local = None
    primary = None
    try:
        local = sqlite3.connect(':memory:')
        local.execute("CREATE VIRTUAL TABLE held USING fts5(path UNINDEXED,layer UNINDEXED,kind UNINDEXED,title,summary,content,tokenize='porter unicode61')")
        local.execute('INSERT INTO held VALUES(?,?,?,?,?,?)', ('', '', '', *image))
        row = local.execute('SELECT highlight(held,5,?,?) FROM held WHERE held MATCH ?',
            (EXCERPT_MARK_OPEN, EXCERPT_MARK_CLOSE, ' OR '.join('"'+token+'"' for token in tokens))).fetchone()
        return str(row[0]) if row is not None else image[2]
    except sqlite3.Error:
        return image[2]
    except BaseException as error:
        primary = error
        raise
    finally:
        if local is not None:
            try:
                local.close()
            except BaseException:
                if primary is None:
                    raise


def marked_document(
    connection: sqlite3.Connection, path: str, query: str,
    *, indexed_image=_NO_HELD_INDEX_IMAGE,
) -> Optional[str]:
    """The document's text with every match of the query marked, or None.

    FTS5 marks the matches with the same tokenizer retrieval matched them by,
    so the excerpt is chosen by exactly the words that selected the document.
    """
    held = indexed_image is not _NO_HELD_INDEX_IMAGE
    if held and (not isinstance(indexed_image, tuple) or len(indexed_image) != 3
                 or not all(isinstance(value, str) for value in indexed_image)):
        return None
    try:
        tokens = evidence_tokens(informative_tokens(
            connection, expand_query_tokens(connection, query_tokens(query))
        ))
    except RetrievalError:
        return None
    if not tokens:
        return None
    # One command marks the same document for the same words several times
    # (identity, excerpt, render); the answer cannot change within it.
    memo = workspace_roots.command_memo("marked-documents")
    key = (path, tuple(tokens), indexed_image if held else None)
    if memo is not None and key in memo:
        return memo[key]
    marked = _marked_document(connection, path, tokens, indexed_image if held else None)
    if memo is not None:
        memo[key] = marked
    return marked


def _marked_document(connection, path, tokens, indexed_image) -> Optional[str]:
    held = indexed_image is not None
    if held and len(indexed_image[2]) > MARKED_DOCUMENT_CHARACTERS:
        return _marked_stretch(indexed_image, tokens)
    if not held:
        try:
            # MATCH first: `path` is not indexed, and a bare path filter reads
            # every row of the table.
            row = connection.execute(
                "SELECT title, summary, content, length(content) FROM documents "
                "WHERE documents MATCH ? AND path = ?",
                (" OR ".join(f'"{token}"' for token in tokens), path),
            ).fetchone()
        except sqlite3.Error:
            row = None
        if row is not None and row[3] is not None and row[3] > MARKED_DOCUMENT_CHARACTERS:
            return _marked_stretch(tuple(str(value or "") for value in row[:3]), tokens)
    if held:
        try:
            row = connection.execute(
                'SELECT title,summary,content,highlight(documents,5,?,?) FROM documents '
                'WHERE documents MATCH ? AND path=?',
                (EXCERPT_MARK_OPEN, EXCERPT_MARK_CLOSE, ' OR '.join('"'+token+'"' for token in tokens), path),
            ).fetchone()
        except sqlite3.Error:
            row = None
        if row is not None and tuple(str(value or '') for value in row[:3]) == indexed_image:
            return str(row[3]) if row[3] is not None else indexed_image[2]
        return _held_marked_document(indexed_image, tokens)
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
        tokens = evidence_tokens(informative_tokens(
            connection, expand_query_tokens(connection, query_tokens(query))
        ))
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


def _quoted_payload(heading: str, lines: list[str], title: str) -> dict[str, Any]:
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


def _first_body_section(text: str) -> Optional[tuple[str, list[str]]]:
    """The newest entry of a chronological document: the first section after
    its title that has prose, without scoring the rest.

    A chronological document can run past a megabyte; its head section is the
    only one a head-preferring quote needs, and parsing every section to pick
    it by score would charge the whole file to the hook. The prose under the
    title is a preamble - Keep a Changelog's "All notable changes to this
    project ..." - not an entry, so it is quoted only when no entry has any.
    An entry whose heading has no prose of its own before a subsection
    (`## [1.2.0] - 2026-10-01`, then `### Fixed`) is named with it:
    "[1.2.0] - 2026-10-01 › Fixed".
    """
    if text.startswith("---\n"):
        _, separator, remainder = text[4:].partition("\n---\n")
        if separator:
            text = remainder
    title, preamble = "", []
    # Headings enclosing the current entry, title excluded, as (level, text).
    trail: list[tuple[int, str]] = []
    lines: Optional[list[str]] = None
    for line in text.splitlines():
        if not _EXCERPT_HEADING.match(line):
            (preamble if lines is None else lines).append(line)
            continue
        if lines is not None and _excerpt_units(lines):
            return " › ".join(name for _, name in trail), lines
        level = len(line) - len(line.lstrip("#"))
        name = line.lstrip("#").strip()
        if lines is None and level == 1 and not title and not re.search(r"\d", name):
            # The document's title. A first heading with a number in it is a
            # version (`# 1.2.0`), the newest entry itself.
            title = name
            continue
        while trail and trail[-1][0] >= level:
            trail.pop()
        trail.append((level, name))
        lines = []
    if lines is not None and _excerpt_units(lines):
        return " › ".join(name for _, name in trail), lines
    return (title, preamble) if _excerpt_units(preamble) else None


def quoted_section(
    text: str, title: str = "", weights: Optional[dict[str, float]] = None,
    *, citation_sources: Optional[set[str]] = None,
    requested_headings: Optional[set[str]] = None,
    prefer_head: bool = False,
) -> Optional[dict[str, Any]]:
    """The section an excerpt quotes: the one carrying most of the query.

    ``text`` is a marked document (see marked_document) or plain text, which
    has no marks and yields the opening prose. With ``prefer_head`` the first
    section that has a body wins instead: a chronological document (the
    changelog) answers from its newest entry, and a query window chosen by
    shared words would quote an older one. Returns the heading (empty for the
    prose before the first heading), its slug, the section's units, and a
    hash of the section's text - what a conversation was handed, for not
    handing it again.
    """
    if prefer_head and requested_headings is None and citation_sources is None:
        found = _first_body_section(text)
        return _quoted_payload(found[0], found[1], title) if found is not None else None
    sections = _excerpt_sections(text)
    # Every marked word asked of the tokenizer at once, not per section.
    term_forms(_EXCERPT_MARKED.findall(text))
    best: Optional[tuple[tuple[float, int, float, int], int]] = None
    requested_best: Optional[tuple[tuple[float, int, float, int], int]] = None
    first_with_body: Optional[int] = None
    for index, (heading, lines) in enumerate(sections):
        requested = _unmarked(heading).casefold() in (requested_headings or ())
        if not requested and citation_sources is not None and _citation_only_section(heading, lines, citation_sources):
            continue
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
        if requested and (requested_best is None or score > requested_best[0]):
            requested_best = (score, index)
    if best is None or first_with_body is None:
        return None
    if requested_best is not None:
        index = requested_best[1]
    elif prefer_head:
        found = _first_body_section(text)
        return _quoted_payload(found[0], found[1], title) if found is not None else None
    elif best[0][0] > 0:
        index = best[1]
    else:
        index = first_with_body
    heading, lines = sections[index]
    return _quoted_payload(heading, lines, title)


def _citation_only_section(heading: str, lines: list[str], sources: set[str]) -> bool:
    # Explicit heading requests are handled separately from FTS highlights:
    # a filename such as src/Sources.py also marks the Sources heading.
    # Explanatory prose and undeclared paths are not a citation-only footer.
    if _unmarked(heading).casefold() not in ("sources", "references"):
        return False
    entries = [line.strip() for line in lines if line.strip()]
    if not entries:
        return False
    for line in entries:
        match = re.fullmatch(r"[-*+]\s+(.+)", _unmarked(line))
        if match is None:
            return False
        reference = match.group(1).strip()
        if reference.startswith("`") and reference.endswith("`"):
            reference = reference[1:-1]
        if reference in sources:
            continue
        base, separator, fragment = reference.partition("#")
        if not separator or base not in sources or re.fullmatch(r"[^\s`]+", fragment) is None:
            return False
    return True


def excerpt_section(
    connection: sqlite3.Connection, item: dict[str, Any], text: str,
    weights: Optional[dict[str, float]] = None,
    *, query: str = "", prefer_head: bool = False,
) -> Optional[dict[str, Any]]:
    """The section delivered and remembered for this selected document.

    Linked records contribute knowledge about the named file: a generated
    citation footer is not that knowledge. Keep query weights across substantive
    sections, and retain original behavior for explicit source requests, prose
    under Sources, source-only records, or unavailable citation metadata.
    """
    title = str(item.get("title") or "")
    prefer_head = prefer_head or item.get("kind") == "changelog"
    if item.get("selection") in ("path-link", "source-link", "source-hint", *AUTOMATIC_LINK_SELECTIONS) and item.get("kind") != "brain-task":
        requested = {
            heading for heading in ("sources", "references")
            if re.search(r"(?<![\w/\\.-])" + heading + r"(?![\w/\\.-])", query, re.IGNORECASE)
        }
        if requested:
            return quoted_section(text, title, weights, requested_headings=requested,
                                  prefer_head=prefer_head)
        try:
            rows = connection.execute(
                "SELECT ref_path FROM document_links WHERE path = ? AND ref_kind = 'source' ORDER BY ref_path LIMIT ?",
                (item["path"], GRAPH_ROW_LIMIT + 1),
            ).fetchall()
        except sqlite3.Error:
            rows = []
        if rows and len(rows) <= GRAPH_ROW_LIMIT:
            section = quoted_section(text, title, weights, citation_sources={str(row[0]) for row in rows})
            if section is not None:
                return section
    return quoted_section(text, title, weights, prefer_head=prefer_head)


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
    units: list[str], limit: int, weights: Optional[dict[str, float]] = None,
    *, prefer_head: bool = False,
) -> str:
    """The stretch of a section that carries most of the query, within limit.

    Whole sentences and list items, starting where the matches are rather
    than at the top; a list item keeps the line that introduces it, which is
    where its condition usually is. With ``prefer_head`` the window starts at
    the section's first unit instead - the newest entry of a chronological
    document, which a query window would displace with an older one that
    happens to share a common word. Elided text is shown as an ellipsis.
    """
    plain = [_unmarked(unit) for unit in units]
    whole = " ".join(plain)
    if len(whole) <= limit:
        return whole
    budget = max(1, limit - 4)
    if prefer_head:
        end, size = 0, 0
        while end < len(units) and size + len(plain[end]) + (1 if end else 0) <= budget:
            size += len(plain[end]) + (1 if end else 0)
            end += 1
        text = " ".join(plain[:end])
        if end < len(units):
            # Fill the rest with the head of the next sentence: a changelog
            # entry is one long sentence, and stopping at the first one would
            # cut the answer the newer entry carries.
            room = budget - len(text) - (1 if text else 0)
            if room > 40:
                piece = plain[end][:room]
                space = piece.rfind(" ")
                if space > room // 2:
                    piece = piece[:space]
                piece = piece.rstrip()
                if piece:
                    text = (text + " " + piece) if text else piece
            text = text.rstrip() + " …"
        return text
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
        lead_end = end
        while lead_end > start and size + lead > budget:
            lead_end -= 1
            size -= len(plain[lead_end]) + 1
        # Carrying a qualifier must not discard the evidence that won the
        # window. Trim only unmatched context; otherwise keep that window.
        if (lead_end > start and size + lead <= budget
                and _excerpt_score(units[start:lead_end], weights) == best[0]):
            start -= 1
            end = lead_end
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
    if (
        item.get("layer") == "procedural" or item.get("match") == "distinctive"
        or item.get("selection") in AUTOMATIC_LINK_SELECTIONS and item.get("kind") == "brain-task"
    ):
        return path, revision
    marked = marked_document(connection, path, query,
                             indexed_image=item.get('_indexed_policy_image'))
    prefer_head = item.get("kind") == "changelog"
    image = item.get('_indexed_policy_image')
    # A long changelog is marked as a stretch around the query, so its head
    # is not in `marked`; the delivery identity must name the section the
    # renderer actually quotes.
    quote_text = str(image[2]) if prefer_head and image and len(image) > 2 else marked
    section = (
        excerpt_section(connection, item, quote_text,
                        excerpt_weights(connection, query), query=query,
                        prefer_head=prefer_head)
        if quote_text
        else None
    )
    if section is None:
        return path, revision
    if item.get("selection") in AUTOMATIC_LINK_SELECTIONS:
        original = quoted_section(marked, str(item.get("title") or ""), excerpt_weights(connection, query))
        if original is not None and original["hash"] == section["hash"]:
            return path, revision
        # A prior renderer handed only the citation footer. A new section
        # identity releases that stale delivery immediately, without changing
        # whole-document suppression for automatic links whose excerpt stayed.
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
    connection: sqlite3.Connection, query: str, limit: int = 100,
    *, trace: Optional[RetrievalTrace] = None,
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    """Ranked candidates, plus what the ranking knew about the query.

    The diagnostics are computed here anyway and were discarded at the end of
    the call. A gate that has to decide whether this turn is worth retrieving
    for cannot recompute them without repeating the work.

    Project Brain events are not candidates here: they reach the capsule only
    through their own search (`history_query`), so they neither take rows of
    this window nor set the episodic floor the changelog is held to.
    """
    ensure_metadata_tables(connection)
    tokens = informative_tokens(
        connection, expand_query_tokens(connection, query_tokens(query))
    )
    evidence = evidence_tokens(tokens)
    if not evidence:
        # Nothing in the request is about anything: "thanks", "ok, go on".
        return [], {"informative_terms": 0, "distinctive_matches": 0, "top_score": None, "coverage": {}}
    coverage, distinctive = token_coverage(connection, evidence)
    minimum = required_coverage(evidence)
    # A rare term admits a document on its own only when it names something.
    # The anchor test below sees the term's shape and the document's path; a
    # term repeated across the corpus is a generic directory or suffix, and
    # five records of different tickets can then win a slot on the shared
    # word alone. Count the terms once and keep only the near-unique names;
    # procedural documents keep their own rule (a weak skill match is never
    # delivered, and search still ranks it).
    anchor_frequencies = (
        token_document_frequencies(
            connection, {term for terms in distinctive.values() for term in terms}
        )
        if distinctive else {}
    )
    specific = _specific_distinctive(distinctive, anchor_frequencies)
    if trace is not None:
        trace.evidence_terms = len(evidence)
        trace.required_terms = minimum
        for path, record in trace.records.items():
            if record["state"] != "unavailable":
                record["evidence_matched_terms"] = coverage.get(path, 0)
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
        WHERE documents MATCH ? AND d.kind != ?
        ORDER BY score, d.path
        LIMIT ?
        """,
        (*BM25_WEIGHTS, " OR ".join(f'"{token}"' for token in tokens), HISTORY_KIND, limit),
    ).fetchall()
    result = []
    observed_rank = 0
    for row in rows:
        if trace is not None:
            observed_rank += int(trace._eligible(dict(row)))
            trace.observe(dict(row), "search", "fetched", rank=observed_rank,
                          matched_terms=coverage.get(row["path"], 0), required_terms=minimum)
        matches = distinctive if row["layer"] == "procedural" else specific
        if not is_relevant(
            row["path"], coverage, matches, minimum, str(row["title"] or ""),
            anchored=row["layer"] != "procedural",
        ):
            if trace is not None:
                trace.observe(dict(row), "admission", "term-coverage")
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
        if trace is not None:
            trace.observe(item, "admission", "admitted", score=item["adjusted_score"])
    # Re-rank the BM25 window: the raw score breaks adjusted ties so the
    # ordering stays deterministic even when every weight is neutral.
    result.sort(key=lambda item: (-item["adjusted_score"], item["score"], item["path"]))
    best_by_layer: dict[str, float] = {}
    for item in result:
        best_by_layer.setdefault(item["layer"], item["adjusted_score"])
    survivors = []
    for item in result:
        threshold = RELATIVE_SCORE_FLOOR * best_by_layer[item["layer"]]
        admitted = item["adjusted_score"] >= threshold
        if trace is not None:
            trace.observe(item, "ranking", "admitted" if admitted else "relative-score-floor",
                          score=item["adjusted_score"])
        if admitted:
            survivors.append(item)
    result = survivors
    # Rank is stamped here, on the lexically ranked list, rather than after
    # filtering: a position in the post-filter list says where a survivor
    # landed, not how well it answered the query, which is the only reading a
    # retrieval gate can use.
    observed_rank = 0
    for position, item in enumerate(result, start=1):
        item["rank"] = position
        if trace is not None:
            observed_rank += int(trace._eligible(item))
            trace.observe(item, "ranking", "authorized-window-rank", rank=observed_rank)
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
        # Internal, never serialized (gate signals name their keys): how many
        # distinct query terms each document holds, for the file-edge channel.
        "coverage": coverage,
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
    quoted: dict[int, tuple[Any, ...]] = {}
    expression = " OR ".join(f'"{token}"' for token in evidence)
    # In batches that stay inside SQLite's parameter limit, as deletes do.
    for start in range(0, len(rowids), DELETE_CHUNK):
        chunk = rowids[start : start + DELETE_CHUNK]
        placeholders = ", ".join("?" for _ in chunk)
        try:
            # A long document is not marked here (see MARKED_DOCUMENT_CHARACTERS);
            # its answering stretch is, below.
            rows = connection.execute(
                "SELECT d.rowid, CASE WHEN length(d.content) <= ? THEN highlight(documents, 5, ?, ?) END, "
                "d.path,d.layer,d.kind,d.title,d.summary,d.content FROM documents AS d "
                f"WHERE documents MATCH ? AND d.rowid IN ({placeholders})",
                (MARKED_DOCUMENT_CHARACTERS, EXCERPT_MARK_OPEN, EXCERPT_MARK_CLOSE, expression, *chunk),
            ).fetchall()
        except sqlite3.Error:
            rows = []
        quoted.update((row[0], tuple(row[1:]) if row[1] is not None else
                       (_marked_stretch(tuple(str(value or "") for value in row[5:8]), evidence),
                        *row[2:]))
                      for row in rows)
        # A candidate the evidence words do not mark is quoted from its plain
        # text, which gives its opening prose.
        missing = [rowid for rowid in chunk if rowid not in quoted]
        if missing:
            rows = connection.execute(
                "SELECT d.rowid,d.content,d.path,d.layer,d.kind,d.title,d.summary,d.content "
                "FROM documents AS d WHERE d.rowid IN ("
                + ", ".join("?" for _ in missing)
                + ")",
                missing,
            ).fetchall()
            quoted.update((row[0], tuple(row[1:])) for row in rows)
    weights = excerpt_weights(connection, query) if items else {}
    for rowid, item in zip(rowids, items):
        row = quoted.get(rowid)
        identity = tuple(item[key] for key in ('path', 'layer', 'kind', 'title'))
        same = row is not None and tuple(row[1:5]) == identity
        # Hold the unhighlighted image from the SAME statement as the quote.
        # A missing/reused/changed row cannot be rebound by a later prime.
        item['_indexed_policy_image'] = tuple(str(value or '') for value in row[4:7]) if same else None
        text = str(row[0] or '') if same else ''
        prefer_head = item.get("kind") == "changelog"
        # A long document is marked as a stretch around the query, not from
        # its first line: the head of a chronological document is outside it.
        # Prefer the full unmarked image the same statement returned.
        image = item.get('_indexed_policy_image')
        head_text = str(image[2]) if image and len(image) > 2 else ''
        quote_text = head_text if prefer_head and head_text else text
        section = (
            quoted_section(quote_text, str(item.get("title") or ""), weights,
                           prefer_head=prefer_head) if quote_text else None
        )
        snippet = (
            excerpt_window(section["units"], SNIPPET_WINDOW_CHARS, weights,
                           prefer_head=prefer_head)
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


def _retrieval_baseline_key(task_uuid: str, host: str, entry_point: str, delivery_profile: str = 'ordinary') -> str:
    key = f"{task_uuid}:{host}:{entry_point}"
    return key + ':cursor-rule' if delivery_profile == 'cursor-rule' else key


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
    return _decode_last_retrievals(raw)


def _decode_last_retrievals(raw: Optional[str]) -> dict[str, Any]:
    if not raw:
        return {}
    try:
        stored = json.loads(raw)
    except (ValueError, RecursionError):
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
    _commit_ordinary_receipts(connection, baseline={"key": baseline_key, "signature": signature})


WINDOW_REFERENCE_LIMIT = 32
WINDOW_BODY_BYTES_LIMIT = 4096


def _window_digest(value: Any) -> bool:
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None


def _window_key(value: str) -> str:
    try:
        return hashlib.sha256(value.encode("utf-8")).hexdigest()
    except UnicodeError:
        # A malformed legacy key still belongs to the existing receipt
        # normalizer; it cannot participate in optional hash metadata.
        return ""


def _window_references(entry: Any, items: dict[str, Any]) -> dict[str, Any]:
    """Normalize observer metadata independently of original novelty receipts."""
    if not isinstance(entry, dict):
        return {}
    observation = entry.get("window_observation")
    if not isinstance(observation, dict) or type(observation.get("version")) is not int or observation["version"] != 1:
        return {}
    refs = observation.get("receipts")
    if not isinstance(refs, dict):
        return {}
    live = {_window_key(key): seen for key, seen in items.items()
            if isinstance(key, str) and isinstance(seen, list) and len(seen) == 2 and type(seen[1]) is int}
    result = {}
    for index, (key, value) in enumerate(refs.items()):
        if index >= WINDOW_REFERENCE_LIMIT:
            break
        if (not _window_digest(key) or not isinstance(value, dict)
                or set(value) != {"revision", "handed_turn", "manifest_id", "doc_ref"}):
            continue
        revision, turn = value["revision"], value["handed_turn"]
        identifier = value["manifest_id"]
        if (not isinstance(revision, str) or re.fullmatch(r"(?:[0-9a-f]{16}|[0-9a-f]{64})", revision) is None
                or type(turn) is not int or turn < 0
                or not isinstance(identifier, str) or TRACE_ID.fullmatch(identifier) is None
                or not _window_digest(value["doc_ref"]) or live.get(key) != [revision, turn]):
            continue
        result[key] = dict(value)
    return result


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
    return _normalize_session_deliveries(stored)


def _normalize_session_deliveries(stored: Any) -> dict[str, Any]:
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
        refs = _window_references(entry, clean["items"])
        if refs:
            clean["window_observation"] = {"version": 1, "receipts": refs}
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
    window_receipts: Optional[dict[str, Any]] = None,
) -> None:
    """Record what the session holds: key -> [revision, turn handed].

    The key is the document, or the section a quoted excerpt came from (see
    delivery_identity). ``reset`` drops what earlier turns were handed: the
    host compacted the conversation since. Best-effort, like the repeat
    record: losing it costs one repeated item, the safe direction.
    """
    status, latest = _commit_ordinary_receipts(connection, session=_ordinary_session_plan(
        session_id, turn, delivered, stored, transcript,
        reset=reset, window_receipts=window_receipts))
    if status in ("committed", "deferred") and latest is not None:
        # Preserve the helper's successful in-place projection, while the
        # supplied snapshot is never authority for unrelated live sessions.
        stored.clear()
        stored.update(latest)


def _session_delivery_projection(
    entry: Optional[dict[str, Any]], turn: int, delivered: dict[str, Any],
    transcript: Optional[dict[str, Any]], *, reset: bool, window_receipts: Any,
) -> dict[str, Any]:
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
    projected: dict[str, Any] = {"turn": turn, "items": items}
    refs = {} if reset else _window_references(entry, items)
    if window_receipts:
        refs.update(_window_references({"window_observation": {"version": 1, "receipts": window_receipts}}, items))
    if refs:
        refs = dict(sorted(refs.items(), key=lambda pair: -pair[1]["handed_turn"])[:WINDOW_REFERENCE_LIMIT])
        projected["window_observation"] = {"version": 1, "receipts": refs}
    if transcript is not None:
        projected["transcript"] = transcript
    return projected


def _receipt_expectation(stored: dict[str, Any], key: str) -> tuple[bool, str]:
    """Freeze normalized target identity, including JSON number types."""
    return (key in stored, json.dumps(stored[key], sort_keys=True, separators=(",", ":"))
            if key in stored else "")


def _ordinary_session_plan(
    session_id: str, turn: int, delivered: dict[str, Any], stored: dict[str, Any],
    transcript: Optional[dict[str, Any]] = None, *, reset: bool = False,
    window_receipts: Any = None, exact_next_turn: bool = False,
) -> dict[str, Any]:
    return {"id": session_id, "turn": turn,
            "expected": _receipt_expectation(_normalize_session_deliveries(stored), session_id),
            "delivered": copy.deepcopy(delivered), "transcript": copy.deepcopy(transcript),
            "reset": reset, "window_receipts": copy.deepcopy(window_receipts),
            "exact_next_turn": exact_next_turn}


class _OrdinaryReceiptConflict(Exception):
    """Disposable bookkeeping cannot accept this prepared projection."""


ORDINARY_RECEIPT_WAIT_SECONDS = 0.15


def _commit_ordinary_receipts(
    connection: sqlite3.Connection, *, baseline: Optional[dict[str, Any]] = None,
    session: Optional[dict[str, Any]] = None,
) -> tuple[str, Optional[dict[str, Any]]]:
    """Merge participating writers under one short SQLite transaction.

    This records ordinary bookkeeping, not external delivery or a supplement
    permit. Conflicts/readonly/SQL errors cost one repeated retrieval, rather
    than damaging the latest state or failing the ordinary response. An
    ambient transaction retains ownership and receives a deferred projection.
    """
    if baseline is None and session is None:
        return "skipped", None
    baseline, session = copy.deepcopy(baseline), copy.deepcopy(session)
    deadline = time.monotonic() + ORDINARY_RECEIPT_WAIT_SECONDS
    ambient = False
    savepoint = "ordinary_receipts_" + new_uuid().replace("-", "")
    begun = False
    timeout = None
    primary = None
    cleaned = False

    def remaining_wait() -> None:
        remaining = int((deadline - time.monotonic()) * 1000)
        if remaining <= 0:
            raise _OrdinaryReceiptConflict()
        connection.execute("PRAGMA busy_timeout = " + str(min(timeout, remaining)))

    def undo() -> None:
        if not begun:
            return
        if ambient:
            connection.execute("ROLLBACK TO SAVEPOINT " + savepoint)
            connection.execute("RELEASE SAVEPOINT " + savepoint)
        elif connection.in_transaction:
            connection.execute("ROLLBACK")

    def finish() -> None:
        try:
            try:
                undo()
            except sqlite3.Error:
                pass
        finally:
            if timeout is not None:
                try:
                    connection.execute("PRAGMA busy_timeout = " + str(timeout))
                except sqlite3.Error:
                    pass

    try:
        try:
            ambient = connection.in_transaction
            timeout = connection.execute("PRAGMA busy_timeout").fetchone()[0]
            remaining_wait()
            # SQL can acquire and then be interrupted before execute returns.
            # Arm cleanup first; undo targets only our possible owned scope.
            begun = True
            connection.execute("SAVEPOINT " + savepoint if ambient else "BEGIN IMMEDIATE")
            # Read failure must skip the whole pair; the best-effort public loaders
            # deliberately swallow errors and are not suitable inside this owner.
            raw = {row[0]: row[1] for row in connection.execute(
                "SELECT key, value FROM index_state WHERE key IN (?, ?)",
                (LAST_RETRIEVAL_KEY, SESSION_DELIVERIES_KEY))}
            baselines = _decode_last_retrievals(raw.get(LAST_RETRIEVAL_KEY))
            try:
                sessions = _normalize_session_deliveries(json.loads(raw.get(SESSION_DELIVERIES_KEY, "{}")))
            except (ValueError, RecursionError):
                sessions = {}
            payloads = {}
            if baseline is not None:
                key = baseline["key"]
                if "expected" in baseline and _receipt_expectation(baselines, key) != baseline["expected"]:
                    raise _OrdinaryReceiptConflict()
                baselines.pop(key, None)
                baselines[key] = baseline["signature"]
                for stale in list(baselines)[:max(0, len(baselines) - LAST_RETRIEVAL_RETENTION)]:
                    baselines.pop(stale, None)
                payloads[LAST_RETRIEVAL_KEY] = json.dumps(baselines, separators=(",", ":"))
            if session is not None:
                identifier = session["id"]
                if _receipt_expectation(sessions, identifier) != session["expected"]:
                    raise _OrdinaryReceiptConflict()
                entry = sessions.get(identifier)
                old_turn = entry.get("turn", 0) if entry else 0
                turn = session["turn"]
                if (type(turn) is not int or turn <= old_turn
                        or (session["exact_next_turn"] and turn != old_turn + 1)):
                    raise _OrdinaryReceiptConflict()
                sessions.pop(identifier, None)
                sessions[identifier] = _session_delivery_projection(
                    entry, turn, session["delivered"], session["transcript"],
                    reset=session["reset"], window_receipts=session["window_receipts"])
                for stale in list(sessions)[:max(0, len(sessions) - SESSION_DELIVERIES_RETENTION)]:
                    sessions.pop(stale, None)
                payloads[SESSION_DELIVERIES_KEY] = json.dumps(sessions, separators=(",", ":"))
            for key, payload in sorted(payloads.items()):
                remaining_wait()
                if key in raw:
                    changed = connection.execute(
                        "UPDATE index_state SET value = ? WHERE key = ? AND value = ?",
                        (payload, key, raw[key]))
                else:
                    changed = connection.execute(
                        "INSERT INTO index_state(key, value) VALUES (?, ?)", (key, payload))
                if changed.rowcount != 1:
                    raise _OrdinaryReceiptConflict()
            remaining_wait()
            if ambient:
                connection.execute("RELEASE SAVEPOINT " + savepoint)
            else:
                # Connection.commit()/rollback() are no-ops with the Python3.12+
                # autocommit=True API, even for an explicitly begun transaction.
                connection.execute("COMMIT")
            begun = False
            return ("deferred" if ambient else "committed"), sessions if session is not None else None
        except (sqlite3.Error, _OrdinaryReceiptConflict):
            return "skipped", None
        except BaseException as error:
            primary = error
            raise
        finally:
            finish()
            cleaned = True
    except BaseException as error:
        if not cleaned:
            # One interruption can happen at cleanup entry or during unwind.
            # A single retry is idempotent and never owns the caller's outer
            # transaction. Driver refusal/repeated cancellation is best effort.
            try:
                finish()
            except BaseException:
                pass
        if primary is not None and primary is not error:
            raise primary
        raise


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
    delivery_profile: str = 'ordinary',
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
        if delivery_profile == 'cursor-rule':
            return {'decision':'retrieve', 'mode':mode, 'reason':'persistent-rule-repeat', 'signals':signals}
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
            d.path, d.layer, d.kind, d.title, d.summary, d.content,
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
        content = str(item.pop('content') or '')
        item['_indexed_policy_image'] = (str(item['title'] or ''), str(item.pop('summary') or ''), content)
        item["snippet"] = _body_snippet(content)
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


SOURCE_HINT_PROFILE = "source-hints-v1"
SOURCE_HINT_LIMIT = 8
SOURCE_HINT_BYTES = 8192
SOURCE_HINT_FILE_BYTES = 131072


def normalize_source_hints(value: Any) -> tuple[list[dict[str, str]], str]:
    """Plain bounded references only; malformed optional input is unavailable."""
    def pairs(entries):
        result = {}
        for key, child in entries:
            if key in result:
                raise ValueError("Duplicate hint field")
            result[key] = child
        return result

    try:
        if type(value) is str:
            if len(value) > SOURCE_HINT_BYTES or len(value.encode("utf-8")) > SOURCE_HINT_BYTES:
                return [], "invalid"
            value = json.loads(value, object_pairs_hook=pairs,
                               parse_constant=lambda _: (_ for _ in ()).throw(ValueError()))
        if type(value) is not dict or set(value) != {"schema_version", "references"}:
            return [], "invalid"
        if type(value["schema_version"]) is not int or value["schema_version"] != 1:
            return [], "invalid"
        references = value["references"]
        if type(references) is not list or len(references) > SOURCE_HINT_LIMIT:
            return [], "invalid"
        clean = []
        for reference in references:
            if type(reference) is not dict or set(reference) != {"path", "source_sha256"}:
                return [], "invalid"
            name, digest = reference["path"], reference["source_sha256"]
            if (type(name) is not str or not name or len(name) > 512
                    or len(name.encode("utf-8")) > 512
                    or name.startswith("/") or any(mark in name for mark in ("\\", ":", "#"))
                    or any(unicodedata.category(char).startswith("C") for char in name)
                    or any(part in ("", ".", "..") for part in name.split("/"))
                    # A discovery hint may name shared derived knowledge too;
                    # canonical citation restrictions are not its authority.
                    or source_path_problem("hints/" + name) is not None
                    or type(digest) is not str or re.fullmatch(r"[0-9a-f]{64}", digest) is None):
                return [], "invalid"
            clean.append({"path": name, "source_sha256": digest})
        encoded = json.dumps({"schema_version": 1, "references": clean}, ensure_ascii=False,
                             separators=(",", ":"), allow_nan=False).encode("utf-8")
        return (clean, "ready") if len(encoded) <= SOURCE_HINT_BYTES else ([], "invalid")
    except (ValueError, TypeError, UnicodeError, RecursionError):
        return [], "invalid"


def _source_hint_read(repository: Path, name: str, expected: str) -> Optional[str]:
    """Contained current raw image, then the native universal-newline digest text.

    No Path.resolve/read_text fallback: unsupported secure primitives abstain.
    Windows reads through the Cursor rule's NT handles, which follow no reparse
    point; every stat compared here comes from that one backend.
    This is a point-in-time check, not a filesystem snapshot or source authority.
    """
    try:
        files = _cursor_backend()
    except (OSError, ImportError, AttributeError):
        return None
    if files is None or (files is os and (
            not hasattr(os, "O_NOFOLLOW") or not hasattr(os, "O_DIRECTORY")
            or os.open not in os.supports_dir_fd or os.stat not in os.supports_dir_fd)):
        return None
    descriptors = []
    directories = []
    try:
        root = workspace_roots.project_root(repository)
        parent = (os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW) if files is os
                  else files.open_root(root))
        root_fd = parent
        descriptors.append(parent)
        parts = name.split("/")
        for part in parts[:-1]:
            child = files.open(part, os.O_RDONLY | files.O_DIRECTORY | files.O_NOFOLLOW, dir_fd=parent)
            directories.append((parent, part, child))
            parent = child
            descriptors.append(parent)
        fd = files.open(parts[-1], os.O_RDONLY | files.O_NOFOLLOW | files.O_NONBLOCK, dir_fd=parent)
        descriptors.append(fd)
        before = files.fstat(fd)
        if not stat.S_ISREG(before.st_mode) or before.st_nlink != 1 or before.st_size > SOURCE_HINT_FILE_BYTES:
            return None
        raw = bytearray()
        while len(raw) <= SOURCE_HINT_FILE_BYTES:
            chunk = os.read(fd, min(8192, SOURCE_HINT_FILE_BYTES + 1 - len(raw)))
            if not chunk:
                break
            raw.extend(chunk)
        after = files.fstat(fd)
        named = files.stat(parts[-1], dir_fd=parent, follow_symlinks=False)
        def identity(info):
            return (info.st_dev, info.st_ino, info.st_mode, info.st_nlink,
                    info.st_size, info.st_mtime_ns, info.st_ctime_ns)
        if (len(raw) > SOURCE_HINT_FILE_BYTES or identity(before) != identity(after)
                or identity(after) != identity(named) or hashlib.sha256(raw).hexdigest() != expected):
            return None
        def directory_identity(info):
            return (info.st_dev, info.st_ino, info.st_mode)
        if directory_identity(files.fstat(root_fd)) != directory_identity(files.stat(root, follow_symlinks=False)):
            return None
        for parent_fd, part, child_fd in directories:
            if directory_identity(files.fstat(child_fd)) != directory_identity(files.stat(part, dir_fd=parent_fd, follow_symlinks=False)):
                return None
        return bytes(raw).decode("utf-8").replace("\r\n", "\n").replace("\r", "\n")
    except (OSError, UnicodeError, ValueError):
        return None
    finally:
        for fd in reversed(descriptors):
            os.close(fd)


def _source_hint_schema_supported(repository: Path) -> bool:
    try:
        schema_file = brain_root(repository) / "schemas" / "retrieval-manifest.schema.json"
        if not schema_file.is_file():
            schema_file = brain_root(workspace_roots.tooling_root(repository)) / "schemas" / "retrieval-manifest.schema.json"
        schema = json.loads(schema_file.read_text(encoding="utf-8"))
        properties = schema["properties"]
        if ("source-hint" not in properties["selected"]["items"]["properties"]["selection"]["enum"]
                or properties["source_hints"]["properties"]["profile"]["const"] != SOURCE_HINT_PROFILE):
            return False
        # Partial upgrades can accept the marker while rejecting its counters
        # or selected-item fields. Abstain before native receipt planning.
        for state in ("ready", "invalid", "unsupported"):
            for count in (0, SOURCE_HINT_LIMIT):
                brain_runtime.validate_schema_value(
                    {"profile": SOURCE_HINT_PROFILE, "state": state, "supplied": count,
                     "admitted": count, "delivered": count}, properties["source_hints"])
        brain_runtime.validate_schema_value(
            {"path": "specs/source.md", "category": "evidence", "estimated_tokens": 1,
             "source_hash": "a" * 64, "selection": "source-hint", "score": 0.0, "rank": None},
            properties["selected"]["items"])
        return True
    except (OSError, ValueError, TypeError, KeyError, BrainError):
        return False


def _source_hint_candidates(connection, repository, config, references, seen, *, trace=None):
    candidates = []
    policy = _current_source_policy(connection, repository, trace)
    admitted = []
    for reference in references:
        name = reference["path"]
        if name in seen:
            continue
        # Metadata first, before hydrating a potentially large indexed body.
        rows = connection.execute("""
            SELECT d.path, d.layer, d.kind, d.title,
                   m.category, m.privacy, m.owner, m.authority, m.lifecycle,
                   m.source_hash, m.record_id, m.conflicts, m.source_fingerprints,
                   m.updated_at, m.confidence, m.attestation
            FROM documents d JOIN document_metadata m ON m.path=d.path
            WHERE d.path=? LIMIT 2""", (name,)).fetchall()
        if len(rows) != 1:
            continue
        item = dict(rows[0])
        # Initial additive profile cannot take the ordinary history/policy slots.
        if item["layer"] != "semantic" or item["category"] == "policy":
            continue
        item.update(selection="source-hint", match=None, adjusted_score=0.0, score=0.0, rank=None,
                    _hint_raw_sha256=reference["source_sha256"])
        item["conflicts"] = json.loads(item["conflicts"])
        item["source_fingerprints"] = json.loads(item["source_fingerprints"])
        if item["conflicts"] or _has_incoming_conflict(connection, item.get("record_id")):
            continue
        # Reject unadmitted/unsafe hints before policy lookup. Their symlink
        # or absent metadata must not turn optional hints into a global error.
        if _source_hint_read(repository, name, item["_hint_raw_sha256"]) is None:
            continue
        admitted.append(item)
    policy.prime(admitted)
    eligible, _ = _runtime_filter(repository, admitted, config, trace=trace,
                                    connection=connection, source_policy=policy)
    for item in eligible:
        name = item["path"]
        item["snippet"] = _body_snippet(item['_indexed_policy_image'][2])
        item["estimated_tokens"] = _estimate_tokens(item["title"] + item["snippet"])
        seen.add(name)
        candidates.append(item)
    return candidates


def _cited_directory(repository, name) -> bool:
    """A citation naming a directory inside the project, such as `src/Billing`.

    Memory Bank chunks cite whole modules this way, and the bank validator and
    `bank-reverify` accept it; the delivery-time existence check must too, or
    such a chunk silently leaves every capsule.
    """
    try:
        relative = name.split("#", 1)[0]
        path = workspace_roots.resolve(repository, relative)
        if path.is_symlink():
            return False
        resolved = path.resolve()
        return workspace_roots.contains(repository, resolved) and resolved.is_dir()
    except (OSError, ValueError, RuntimeError):
        return False


class _CurrentSourcePolicy:
    """One operation's bounded metadata batches, never a persistent permit.

    Ignore rules decide what is indexed and quoted: a document whose own path
    the project now ignores is withheld, cached or not. A cited file's ignore
    status withholds nothing. The record citing it is already shared through
    Project Brain or the Memory Bank, and citing `vendor/...`, `.env` or the
    accelerator's own ignored specs is how framework work gets described, so
    holding such knowledge back protected nothing and cost the answer.
    Citation edges still drive the existence check (missing_source).
    """
    def __init__(self, connection, repository):
        self.connection = connection
        self.repository = Path(repository).resolve()
        self.rows = {}
        self.observed = {}
        self.local_sources = {}
        self._source_exists = {}
        self.memory_boundaries = {}
        self.indexed_images = {}
        self.policy_checks = {}

    def prime(self, candidates, *, indexed_images=None):
        candidates = list(candidates)
        hinted = {item['path'] for item in candidates
                  if 'selection' in item.keys() and item['selection'] == 'source-hint'}
        for item in candidates:
            if '_indexed_policy_image' in item.keys():
                self.indexed_images.setdefault(item['path'], item['_indexed_policy_image'])
        for name, image in (indexed_images or {}).items():
            self.indexed_images.setdefault(name, image)
        names = sorted({item["path"] for item in candidates} - self.rows.keys())
        pending = {}
        for offset in range(0, len(names), 400):
            batch = names[offset:offset + 400]
            markers = ",".join("?" for _ in batch)
            refs = {name: set() for name in batch}
            local_sources = {name: set() for name in batch}
            missing_images = [name for name in batch if name not in self.indexed_images]
            # Optional hints retain their byte cap before any cached body is
            # hydrated, including stale rows whose current source is small.
            for bounded in (False, True):
                image_names = [name for name in missing_images if (name in hinted) == bounded]
                if not image_names:
                    continue
                image_markers = ','.join('?' for _ in image_names)
                bound = ' AND length(CAST(content AS BLOB))<=?' if bounded else ''
                parameters = image_names + ([SOURCE_HINT_FILE_BYTES] if bounded else [])
                for row in self.connection.execute(
                        f'SELECT path,title,summary,content FROM documents WHERE path IN ({image_markers}){bound}', parameters):
                    self.indexed_images[row['path']] = tuple(str(row[key] or '') for key in ('title','summary','content'))
            for row in self.connection.execute(
                    f"SELECT path,eligible_until FROM document_source_state WHERE path IN ({markers})", batch):
                self.memory_boundaries[row["path"]] = row["eligible_until"]
            for row in self.connection.execute(
                    f"SELECT path,ref_path FROM document_links WHERE ref_kind='source' AND path IN ({markers})", batch):
                if not row["ref_path"].startswith(("http://", "https://")):
                    local_sources[row["path"]].add(row["ref_path"])
            for row in self.connection.execute(
                    f"SELECT path,source_fingerprints FROM document_metadata WHERE path IN ({markers})", batch):
                for entry in json.loads(row["source_fingerprints"]):
                    if not entry["path"].startswith(("http://", "https://")):
                        local_sources[row["path"]].add(entry["path"])
            for name in batch:
                own = None
                if not (workspace_roots.accelerator_owned_source(name) or name.startswith("project-brain/")):
                    own = workspace_roots.project_source_key(self.repository, name)
                pending[name] = (own, refs[name])
                self.local_sources[name] = local_sources[name]
        keys = {key for own, refs in pending.values() for key in ([own] if own is not None else []) + list(refs)}
        delta = sorted(keys - self.observed.keys())
        ignored = workspace_roots._source_git_ignored(workspace_roots.project_root(self.repository), delta, 2.0)
        self.observed.update((key, os.fsencode(key) in ignored) for key in delta)
        self.rows.update(pending)

    def missing_source(self, candidate):
        # The full edge set, not just optional legacy digests. One final
        # existence observation per local citation; remote URLs stay unknown.
        for name in self.local_sources[candidate["path"]]:
            if name not in self._source_exists:
                try:
                    brain_runtime._cited_file(self.repository, name)
                    self._source_exists[name] = True
                except (BrainError, OSError, ValueError, RuntimeError):
                    self._source_exists[name] = _cited_directory(self.repository, name)
            if not self._source_exists[name]:
                return True
        return False

    def image_reason(self, image):
        if not isinstance(image, tuple) or len(image) != 3:
            return 'stale'
        # Cache an immutable image under the current grammar, never an
        # operation-long permit when patterns change during a callback.
        key = (secret_policy_fingerprint(), image)
        if key not in self.policy_checks:
            self.policy_checks[key] = indexed_secret_policy_reason(*image)
        return self.policy_checks[key]

    def reason(self, candidate, *, temporal=True):
        own, _ = self.rows[candidate["path"]]
        if own is not None and self.observed[own]:
            return "ignored"
        image = (candidate['_indexed_policy_image'] if '_indexed_policy_image' in candidate.keys()
                 else self.indexed_images.get(candidate['path']))
        if 'selection' in candidate.keys() and candidate['selection'] == 'source-hint':
            # Also bound images already held through another candidate lane;
            # the SQL cap above governs newly loaded images only.
            if not isinstance(image, tuple) or len(image) != 3 or not isinstance(image[2], str):
                return 'stale'
            if len(image[2]) > SOURCE_HINT_FILE_BYTES or len(image[2].encode('utf-8')) > SOURCE_HINT_FILE_BYTES:
                return 'stale'
        problem = self.image_reason(image)
        if problem is not None:
            return problem
        problem = indexed_secret_policy_reason(str(candidate['path']),
                                               str(candidate['title'] or '') if 'title' in candidate.keys() else '',
                                               str(candidate['snippet'] or '') if 'snippet' in candidate.keys() else '')
        if problem is not None:
            return problem
        # Cache the boundary, never its admission decision: a single retrieval
        # may cross midnight. Raw search has no held source body; governed
        # retrieval below rechecks its exact freshly read frontmatter instead.
        boundary = self.memory_boundaries.get(candidate["path"])
        if temporal and candidate["kind"] == "memory" and boundary is not None:
            return memory_temporal_reason({"review_after": boundary})
        return None


def _current_source_policy(connection, repository, trace=None):
    policy = getattr(trace, "_current_source_policy", None) if trace is not None else None
    if policy is None:
        if connection is None:
            raise workspace_roots.SourceGitError("Git source context unavailable")
        policy = _CurrentSourcePolicy(connection, repository)
    if ((connection is not None and policy.connection is not connection)
            or policy.repository != Path(repository).resolve()):
        raise workspace_roots.SourceGitError("Git source context unavailable")
    return policy


def _runtime_filter(
    repository: Path,
    candidates: Iterable[dict[str, Any]],
    config: dict[str, Any],
    *, trace: Optional[RetrievalTrace] = None, connection=None, source_policy=None,
    custody_only: bool = False,
) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    candidates = list(candidates)
    policy = source_policy or _current_source_policy(connection, repository, trace)
    policy.prime(candidates)
    # Cache exact freshly read bytes only; path and live-read checks still run.
    # Each invocation, including the final recheck, owns a new bounded cache.
    normalization_cache = _CitationDigestCache()
    selected = []
    excluded = []
    owners = set(config.get("owners", ["*"]))
    for candidate in candidates:
        # The first pre-pack image survives copies and final policy refresh.
        # A safe replacement cache row must not certify older packed content.
        candidate.setdefault('_indexed_policy_image', policy.indexed_images.get(candidate['path']))
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
            reason = policy.reason(candidate, temporal=False)
        content: Optional[str] = None
        if reason is None:
            if candidate.get("selection") == "source-hint":
                content = _source_hint_read(repository, candidate["path"], candidate["_hint_raw_sha256"])
            else:
                path = workspace_roots.resolve(repository, candidate["path"])
                try:
                    content = path.read_text(encoding="utf-8") if path.is_file() else None
                except (OSError, UnicodeError):
                    content = None
            # `source_hash` is the file's digest even when the indexed text is
            # masked, so this compares the file with the file it was indexed from.
            if content is None or _content_hash(content) != candidate["source_hash"]:
                reason = "stale"
        if (reason is None and content is not None
                and (candidate['kind'] == 'memory' or candidate['kind'].startswith('brain-'))):
            # Protected authority remains a whole-record profile: a masked
            # cached projection cannot make unsafe structured metadata valid.
            reason = indexed_secret_policy_reason(content)
        if reason is None and candidate["kind"] == "memory" and content is not None:
            # Reapply known temporal bounds to the image already hash-checked,
            # including the final custody pass after pack. No second file read,
            # reranking, refill, or reliance on a warm index refresh is needed.
            reason = memory_temporal_reason(_chunk_frontmatter(content))
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
                repository, candidate["source_fingerprints"],
                _normalization_cache=normalization_cache
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
                if not custody_only:
                    candidate["adjusted_score"] = (
                        float(candidate.get("adjusted_score") or 0.0)
                        * SOURCE_CHANGED_WEIGHT
                    )
        if reason:
            excluded.append({"path": candidate["path"], "reason": reason})
        else:
            selected.append(candidate)
        if trace is not None:
            trace.observe(candidate, "runtime", reason or ("source-changed-admitted" if candidate.get("source_changed") else "admitted"),
                          score=float(candidate.get("adjusted_score") or 0.0))
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
    *, trace: Optional[RetrievalTrace] = None,
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
            connection, repository, config, reference, trace=trace
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
    for item in candidates:
        item["snippet"] = _body_snippet(item['_indexed_policy_image'][2])
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
    ref_kinds: tuple[str, ...] = ("source",),
    trace: Optional[RetrievalTrace] = None,
) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    """Every eligible document that declares `reference` among its sources.

    ``ref_kinds`` defaults to citations. `file` rows mean an active task
    touched the path, not that it cites it; only `links` and the automatic
    seed channel read them.

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
    if not ref_kinds or not set(ref_kinds) <= set(LINK_KINDS):
        raise RetrievalError(f"Link kinds must be among: {', '.join(LINK_KINDS)}")
    predicate = f"({predicate}) AND l.ref_kind IN ({', '.join('?' for _ in ref_kinds)})"
    parameters = (*parameters, *ref_kinds)
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
        if trace is not None:
            trace.observe(item, "source-link", "hydrated")
    return _runtime_filter(repository, candidates, config, trace=trace, connection=connection,
                           source_policy=_CurrentSourcePolicy(connection, repository))


_PROMPT_PATH = re.compile(
    r"(?<![\w@:/\\.-])(?:\./)?((?:[\w.-]+/)+[\w-]+(?:\.[\w-]+)*\.[A-Za-z0-9]{1,8})(?![\w/-])"
)
_PROMPT_ABSOLUTE = re.compile(r"(?<![\w.-])(/(?:[\w.-]+/)+[\w.-]+\.[A-Za-z0-9]{1,8})")
_PROMPT_BACKSLASH = re.compile(r"(?<![\w\\])((?:[\w.-]+\\)+[\w.-]+\.[A-Za-z0-9]{1,8})")
_PROMPT_FQCN = re.compile(r"\\?((?:[A-Z][A-Za-z0-9_]*\\+)+[A-Z][A-Za-z0-9_]*)")
_PROMPT_BARE_FILE = re.compile(r"(?<![\w/\\.-])([\w-]+\.[A-Za-z0-9]{1,8})(?![\w/-])")
_PROMPT_SYMBOL = re.compile(r"(?<![\w\\$])([A-Z][a-z0-9]+(?:[A-Z][a-z0-9]*)+)(?![\w\\(])")
COMPOSER_READ_LIMIT = 256 * 1024
LINK_PATH_SCAN_LIMIT = 20000


def _composer_psr4(project: Path) -> list[tuple[str, list[str]]]:
    """composer.json's PSR-4 prefixes (autoload and autoload-dev), longest
    first, each with its directories; nothing when it cannot be read."""
    path = project / "composer.json"
    try:
        if not path.is_file() or path.is_symlink() or path.stat().st_size > COMPOSER_READ_LIMIT:
            return []
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError, UnicodeDecodeError):
        return []
    prefixes: dict[str, list[str]] = {}
    for section in ("autoload", "autoload-dev"):
        mapping = document.get(section, {}).get("psr-4") if isinstance(document.get(section), dict) else None
        if not isinstance(mapping, dict):
            continue
        for prefix, directories in mapping.items():
            if not isinstance(prefix, str):
                continue
            key = prefix.rstrip("\\") + "\\" if prefix else ""
            for directory in directories if isinstance(directories, list) else [directories]:
                if not isinstance(directory, str):
                    continue
                directory = directory[2:] if directory.startswith("./") else directory
                directory = directory.rstrip("/")
                if directory and source_path_problem(directory + "/x.php"):
                    continue
                prefixes.setdefault(key, []).append(directory)
    return sorted(prefixes.items(), key=lambda entry: -len(entry[0]))


def prompt_path_seeds(connection: sqlite3.Connection, repository: Path, text: str) -> list[str]:
    """Paths the request names - written out, as a PHP class (through
    composer.json's PSR-4 map), or as a bare file or class name - that some
    document is linked to. Read in memory only: nothing is written anywhere.
    """
    text = text[:PROMPT_SEED_SCAN_CHARS]
    project = workspace_roots.project_root(repository)
    found: list[str] = []
    found.extend(_PROMPT_PATH.findall(text))
    root = project.as_posix().rstrip("/") + "/"
    for absolute in _PROMPT_ABSOLUTE.findall(text):
        if absolute.startswith(root):
            found.append(absolute[len(root):])
    found.extend(path.replace("\\", "/") for path in _PROMPT_BACKSLASH.findall(text))
    names = [re.sub(r"\\+", "\\\\", name) for name in _PROMPT_FQCN.findall(text)]
    bare = [name for name in _PROMPT_BARE_FILE.findall(text) if "/" not in name]
    symbols = _PROMPT_SYMBOL.findall(text)
    if names:
        for name in names:
            for prefix, directories in _composer_psr4(project):
                if not name.startswith(prefix):
                    continue
                rest = name[len(prefix):].replace("\\", "/")
                found.extend(
                    (f"{directory}/{rest}.php" if directory else f"{rest}.php") for directory in directories[:2]
                )
                break
    linked_paths: Optional[list[str]] = None

    def link_paths() -> list[str]:
        nonlocal linked_paths
        if linked_paths is None:
            linked_paths = [
                row[0] for row in connection.execute(
                    "SELECT DISTINCT ref_path FROM document_links WHERE ref_kind IN ('source', 'file') LIMIT ?",
                    (LINK_PATH_SCAN_LIMIT,),
                )
            ]
        return linked_paths

    for name in names:
        segments = name.split("\\")
        if len(segments) > 1:
            suffix = "/".join(segments[1:]) + ".php"
            found.extend([path for path in link_paths() if path.endswith("/" + suffix)][:2])
    for name in bare:
        found.extend([path for path in link_paths() if path.rsplit("/", 1)[-1] == name][:2])
    for symbol in symbols:
        found.extend([
            path for path in link_paths()
            if posixpath.splitext(path.rsplit("/", 1)[-1])[0] == symbol
        ][:2])
    seeds: list[str] = []
    for path in dict.fromkeys(item[2:] if item.startswith("./") else item for item in found):
        if len(seeds) >= PROMPT_SEED_LIMIT:
            break
        if _file_link_problem(path) is not None:
            continue
        if connection.execute(
            "SELECT 1 FROM document_links WHERE ref_path = ? LIMIT 1", (path,)
        ).fetchone():
            seeds.append(path)
    return seeds


def _automatic_link_candidates(
    connection: sqlite3.Connection,
    repository: Path,
    config: dict[str, Any],
    seeds: list[tuple[str, str]],
    filtered: list[dict[str, Any]],
    *,
    own_paths: set[str],
    coverage: dict[str, int],
    trace: Optional[RetrievalTrace] = None,
) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    """At most FILE_LINK_LIMIT documents linked to a seed path, shaped like
    ranked candidates and marked `prompt-link` or `touch-link`.

    A seed is a path the request named (`prompt`) or the current task touched
    (`touch`). It reaches documents that cite it (`source` rows) and other
    active tasks that touched it (`file` rows); a path linked from more than
    FILE_LINK_HUB_LIMIT documents is a hub and reaches nothing. A document
    reached only through touched files must also share an informative query
    term: file sharing alone is dense on real projects, and "continue" must
    not hand over a sibling task.
    """
    blocked = {item["path"] for item in filtered if item.get("match") != "distinctive"} | own_paths
    found: dict[str, dict[str, Any]] = {}
    for index, (seed, origin) in enumerate(seeds):
        for kind_rank, kind in enumerate(LINK_KINDS):
            rows = [
                row[0] for row in connection.execute(
                    "SELECT path FROM document_links INDEXED BY document_links_source_path "
                    "WHERE ref_path = ? AND ref_kind = ? ORDER BY path LIMIT ?",
                    (seed, kind, FILE_LINK_HUB_LIMIT + 1 + len(own_paths)),
                )
            ]
            others = [path for path in rows if path not in own_paths]
            if len(others) > FILE_LINK_HUB_LIMIT:
                if trace is not None:
                    for path in others:
                        trace.known_event(path, "file-link", "hub-limit")
                continue
            for path in others:
                if path in blocked:
                    if trace is not None:
                        trace.known_event(path, "file-link", "already-selected-or-own")
                    continue
                rank = (0 if origin == "prompt" else 1, kind_rank, index)
                entry = found.setdefault(path, {"rank": rank, "seeds": set(), "via": seed, "prompt": False})
                if rank < entry["rank"]:
                    entry["rank"], entry["via"] = rank, seed
                entry["seeds"].add(seed)
                entry["prompt"] = entry["prompt"] or origin == "prompt"
    if not found:
        return [], []
    pool = sorted(found, key=lambda path: (*found[path]["rank"], -len(found[path]["seeds"]), path))
    if trace is not None:
        for path in pool[FILE_LINK_POOL:]:
            trace.known_event(path, "file-link", "pool-limit")
    pool = pool[:FILE_LINK_POOL]
    marks = ",".join("?" for _ in pool)
    rows = connection.execute(
        f"""
        SELECT
            d.path, d.layer, d.kind, d.title, d.summary, d.content,
            m.category, m.privacy, m.owner, m.authority, m.lifecycle,
            m.source_hash, m.record_id, m.conflicts, m.source_fingerprints,
            m.updated_at, m.confidence, m.attestation
        FROM documents AS d
        JOIN document_metadata AS m ON m.path = d.path
        WHERE d.path IN ({marks})
        """,
        pool,
    ).fetchall()
    candidates = []
    for row in rows:
        item = dict(row)
        content = str(item.pop("content") or "")
        item['_indexed_policy_image'] = (str(item['title'] or ''), str(item.pop('summary') or ''), content)
        if trace is not None:
            trace.observe(item, "file-link", "hydrated")
        if item["layer"] != "semantic" or item["category"] not in ("durable", "dynamic"):
            if trace is not None:
                trace.observe(item, "file-link", "category")
            continue
        item["conflicts"] = json.loads(item["conflicts"])
        item["source_fingerprints"] = json.loads(item["source_fingerprints"])
        if item["conflicts"] or _has_incoming_conflict(connection, item.get("record_id")):
            if trace is not None:
                trace.observe(item, "file-link", "conflict")
            continue
        entry = found[item["path"]]
        if not entry["prompt"] and coverage.get(item["path"], 0) < 1:
            if trace is not None:
                trace.observe(item, "file-link", "touch-without-query-term")
            continue
        item["_body"] = content
        candidates.append(item)
    eligible, _ = _runtime_filter(repository, candidates, config, trace=trace, connection=connection)
    eligible.sort(key=lambda item: (
        found[item["path"]]["rank"], bool(item.get("source_changed")),
        "".join(chr(0x10FFFF - ord(char)) for char in str(item.get("updated_at") or "")), item["path"],
    ))
    for item in eligible:
        entry = found[item["path"]]
        item["selection"] = "prompt-link" if entry["prompt"] else "touch-link"
        item["via"] = entry["via"]
        item["match"] = None
        item["rank"] = None
        item["adjusted_score"] = 0.0
        item["snippet"] = _body_snippet(item.pop("_body"))
        item["estimated_tokens"] = _estimate_tokens(item["title"] + item["snippet"])
    for item in candidates:
        item.pop("_body", None)
    return eligible[:FILE_LINK_LIMIT], [
        {"path": item["path"], "reason": "file-link-limit"} for item in eligible[FILE_LINK_LIMIT:]
    ]


def _insert_automatic_links(
    filtered: list[dict[str, Any]], linked: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    """`filtered` with the linked documents placed after its last strong
    semantic match (or explicit path link) and before weak matches; a weak
    match that a link confirms moves there. Nothing else changes order."""
    if not linked:
        return filtered
    moved = {item["path"] for item in linked}
    rest = [item for item in filtered if item["path"] not in moved]
    anchors = [
        index for index, item in enumerate(rest)
        if item.get("selection") == "path-link"
        or (item["category"] != "policy" and item["layer"] != "episodic" and item.get("match") == "covered")
    ]
    position = anchors[-1] + 1 if anchors else 0
    return rest[:position] + linked + rest[position:]


def _shared_source_documents(
    connection: sqlite3.Connection,
    repository: Path,
    config: dict[str, Any],
    anchor: dict[str, Any],
    max_rows: int,
    current_digests: dict[str, Optional[str]],
    *, trace: Optional[RetrievalTrace] = None,
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
        if trace is not None:
            trace.observe(item, "shared-source", "hydrated")
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
        else:
            if trace is not None:
                trace.observe(item, "shared-source", "no-current-shared-revision")
    eligible, _ = _runtime_filter(repository, candidates, config, trace=trace, connection=connection)
    return eligible, len(paths)


def source_link_candidates(
    connection: sqlite3.Connection,
    repository: Path,
    anchors: list[dict[str, Any]],
    seen: set[str],
    *, _image_sink=None,
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
    if _image_sink is not None:
        for name, image in _selected_index_images(candidates).items():
            _image_sink.setdefault(name, image)
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
    *, trace: Optional[RetrievalTrace] = None,
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
            if trace is not None:
                trace.observe(anchor, "source-link", "anchor-conflict")
            continue
        # Recheck at the traversal boundary, including edits since seed search.
        eligible, _ = _runtime_filter(repository, [dict(anchor)], config, trace=trace, connection=connection)
        if not eligible:
            continue
        remaining = GRAPH_ROW_LIMIT - stats["examined"]
        if remaining <= 0:
            stats["truncated"] = True
            break
        # Citations only: an active task that merely touched the anchor is
        # not a neighbour of what the anchor says.
        linked, withheld = linked_documents(
            connection, repository, config, anchor["path"], max_rows=remaining,
            ref_kinds=("source",),
            trace=trace,
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
                trace=trace,
            )
            linked.extend(shared)
            stats["examined"] += shared_rows
            stats["truncated"] |= shared_rows == remaining
        for item in linked:
            reason = (
                "already-seen" if item["path"] in seen
                else "category" if item["layer"] != "semantic" or item["category"] not in ("durable", "dynamic")
                else "conflict" if item["conflicts"] or _has_incoming_conflict(connection, item.get("record_id"))
                else None
            )
            if reason is not None:
                # A graph-only conflict component may need more semantic
                # slots than remain. Until admission is component-aware, do
                # not introduce either side through automatic expansion.
                if trace is not None:
                    trace.observe(item, "source-link", reason)
                continue
            seen.add(item["path"])
            stats["eligible"] += 1
            if len(candidates) >= GRAPH_CANDIDATE_LIMIT:
                stats["truncated"] = True
                if trace is not None:
                    trace.observe(item, "source-link", "candidate-limit")
                continue
            item["snippet"] = _body_snippet(item['_indexed_policy_image'][2])
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


TRACE_RECORD_LIMIT = 128
TRACE_EVENT_LIMIT = 16
TRACE_WATCH_LIMIT = 16
TRACE_FILE_LIMIT = 512 * 1024
TRACE_ID = re.compile(r"^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$")
TRACE_SAFE_DIRECTORY_IO = os.open in os.supports_dir_fd and hasattr(os, "O_NOFOLLOW") and hasattr(os, "O_DIRECTORY")


def _trace_path_allowed(path: object) -> bool:
    if not isinstance(path, str) or not path or len(path) > 512:
        return False
    if any(ord(char) < 32 or ord(char) == 127 for char in path):
        return False
    if posixpath.normpath(path) != path or path in (".", "") or "#" in path:
        return False
    problem = source_path_problem(path)
    # Memory is derived evidence, but is precisely what this observer follows.
    return problem is None or (
        problem == "Cite canonical project sources, not private runtime state or derived memory"
        and path.startswith(("memory-bank/chunks/", "project-brain/dynamic/"))
        and not source_path_problem("trace/" + path)
    )


def validate_trace_paths(paths: Optional[list[str]]) -> list[str]:
    if len(paths or []) > TRACE_WATCH_LIMIT:
        raise RetrievalError("--trace-path accepts at most 16 paths")
    if any(not _trace_path_allowed(path) for path in paths or []):
        raise RetrievalError("--trace-path requires a safe project-relative document path (max 512 characters)")
    return list(dict.fromkeys(paths or []))


# The aggregate funnel contains no identifiers, query material or global cap signals.
# Counters describe authorized observation occurrences, not unique candidates.
FUNNEL_COUNT_LIMIT = 65535
FUNNEL_EVENTS = {
    "search": ("fetched",),
    "admission": ("term-coverage", "admitted"),
    "ranking": ("admitted", "relative-score-floor", "authorized-window-rank"),
    "history": ("admitted",),
    "runtime": ("admitted", "source-changed-admitted", "authority", "lifecycle",
                "stale", "map-unverifiable", "map-drift", "source-missing",
                "promoted-to-chunk", "working-task", "host-loaded", "path-link-limit",
                "file-link-limit"),
    "source-link": ("hydrated", "anchor-conflict", "already-seen", "category",
                    "conflict", "candidate-limit"),
    "file-link": ("hydrated", "category", "conflict", "touch-without-query-term",
                  "hub-limit", "already-selected-or-own", "pool-limit"),
    "shared-source": ("hydrated", "no-current-shared-revision"),
    "queue": ("lexical", "path-link", "source-link", "prompt-link", "touch-link", "source-hint"),
    "budget": ("admitted", "budget", "hard-budget"),
    "layer": ("episodic-layer", "skill-subfile", "host-listed", "layer-limit",
              "delivered-this-session"),
    "gate": ("withheld",),
    "capsule": ("capsule-limit",),
    "delivery": ("selected",),
}
FUNNEL_BOUNDARIES = ("queue_after_runtime", "budget_admitted", "layer_pre_novelty",
                     "repeat_withheld", "gate_input", "gate_withheld", "pack_input",
                     "pack_omitted", "selected_post_pack")
FUNNEL_RENDER_REASONS = ("excerpt", "render-limit", "sanitized-empty", "no-excerpt")


def _funnel_counter(count: Optional[int] = None) -> dict[str, Any]:
    return {"count": count, "state": "unknown" if count is None else "exact"}


def _funnel_template(state: str = "active") -> dict[str, Any]:
    return {
        "schema_version": 1, "observer_state": state,
        "scope": "authorized-metadata-observation-occurrences",
        "absence": "unknown",
        "coverage": {"bounded_channels": "authorized-observations-only",
                     "history_pre_admission": "not-instrumented",
                     "episodes": "not-instrumented", "working_sources": "not-instrumented",
                     "path_only_events": "authorized-retained-records-only"},
        "events": {stage: {reason: _funnel_counter(0 if state == "active" else None)
                           for reason in reasons} for stage, reasons in FUNNEL_EVENTS.items()},
        "boundaries": {name: _funnel_counter() for name in FUNNEL_BOUNDARIES},
        "render": {"projection_state": "unknown", "scope": "capsule_projection",
                   "authorization_scope": "authorized-retained-records",
                   "emission": "unknown", "consumer": "unknown",
                   "headers": _funnel_counter(),
                   "reasons": {reason: _funnel_counter() for reason in FUNNEL_RENDER_REASONS}},
    }


class _PassiveFunnel:
    """Fixed, content-free diagnostics. Never used by retrieval decisions."""
    def __init__(self) -> None:
        self.data = _funnel_template()

    @staticmethod
    def _increment(counter: dict[str, Any]) -> None:
        if counter["count"] < FUNNEL_COUNT_LIMIT:
            counter["count"] += 1
        else:
            counter["state"] = "lower-bound"

    def event(self, stage: str, reason: str) -> None:
        if (not isinstance(stage, str) or not isinstance(reason, str)
                or reason not in FUNNEL_EVENTS.get(stage, ())):
            return
        self._increment(self.data["events"][stage][reason])

    def boundary(self, name: str, items: list[dict[str, Any]], eligible: Callable) -> None:
        if name not in FUNNEL_BOUNDARIES or not isinstance(items, (list, tuple)):
            return
        counter = _funnel_counter(0)
        for item in items:
            if eligible(item):
                self._increment(counter)
        self.data["boundaries"][name] = counter

    def snapshot(self) -> dict[str, Any]:
        return copy.deepcopy(self.data)

    @staticmethod
    def valid(value: Any) -> bool:
        template = _funnel_template()
        if (not isinstance(value, dict) or set(value) != set(template)
                or type(value["schema_version"]) is not int or value["schema_version"] != 1
                or value["observer_state"] not in ("active", "unavailable")
                or any(value[key] != template[key] for key in ("scope", "absence", "coverage"))):
            return False
        def counter(item: Any) -> bool:
            return (isinstance(item, dict) and set(item) == {"count", "state"}
                    and ((item["state"] == "unknown" and item["count"] is None)
                         or item["state"] in ("exact", "lower-bound")
                         and type(item["count"]) is int and 0 <= item["count"] <= FUNNEL_COUNT_LIMIT
                         and (item["state"] != "lower-bound" or item["count"] == FUNNEL_COUNT_LIMIT)))
        if (not isinstance(value["events"], dict) or set(value["events"]) != set(FUNNEL_EVENTS)
                or not isinstance(value["boundaries"], dict)
                or set(value["boundaries"]) != set(FUNNEL_BOUNDARIES)):
            return False
        for stage, reasons in FUNNEL_EVENTS.items():
            group = value["events"][stage]
            if not isinstance(group, dict) or set(group) != set(reasons) or not all(counter(x) for x in group.values()):
                return False
        render = value["render"]
        return (all(counter(x) for x in value["boundaries"].values())
                and isinstance(render, dict) and set(render) == set(template["render"])
                and render["projection_state"] in ("unknown", "observed")
                and all(render[key] == template["render"][key] for key in
                        ("scope", "authorization_scope", "emission", "consumer"))
                and counter(render["headers"]) and isinstance(render["reasons"], dict)
                and set(render["reasons"]) == set(FUNNEL_RENDER_REASONS)
                and all(counter(x) for x in render["reasons"].values()))

    @staticmethod
    def with_render(value: Any, records: list[dict[str, Any]]) -> dict[str, Any]:
        if not _PassiveFunnel.valid(value):
            return _funnel_template("unavailable")
        result = copy.deepcopy(value)
        # An invalid re-finalization must not retain an earlier projection claim.
        result["render"] = _funnel_template()["render"]
        # Existing finalization already restricts these to retained authorized records.
        # This deliberately does not claim coverage of records lost to trace caps.
        counts = {reason: _funnel_counter(0) for reason in FUNNEL_RENDER_REASONS}
        headers = _funnel_counter(0)
        for record in records:
            reason, length = record.get("body_reason"), record.get("body_bytes")
            if (reason not in FUNNEL_RENDER_REASONS or record.get("header_present") is not True
                    or type(length) is not int or length < 0
                    or type(record.get("pointer_only")) is not bool
                    or record["pointer_only"] != (length == 0)
                    or (reason == "excerpt") != (length > 0)):
                return result
            _PassiveFunnel._increment(headers)
            _PassiveFunnel._increment(counts[reason])
        result["render"].update(projection_state="observed", headers=headers, reasons=counts)
        return result


def unknown_ordinary_receipt() -> dict[str, Any]:
    """Unavailable metadata says nothing about an attempt or its effects."""
    return {"schema_version": 1, "observation_state": "unavailable",
            "plans_present": None, "returned_status": "unknown"}


def normalize_ordinary_receipt(value: Any) -> Optional[dict[str, Any]]:
    """Rebuild the fixed scalar sibling without touching arbitrary value hooks."""
    keys = ("schema_version", "observation_state", "plans_present", "returned_status")
    if (type(value) is not dict or any(type(key) is not str for key in value)
            or set(value) != set(keys) or type(value["schema_version"]) is not int
            or value["schema_version"] != 1 or type(value["observation_state"]) is not str
            or type(value["returned_status"]) is not str):
        return None
    state, plans, status = value["observation_state"], value["plans_present"], value["returned_status"]
    if state == "unavailable":
        if plans is not None or status != "unknown":
            return None
    elif state == "observed":
        if (type(plans) is not bool or status not in ("skipped", "deferred", "committed")
                or (plans is False and status != "skipped")):
            return None
    else:
        return None
    return {"schema_version": 1, "observation_state": state,
            "plans_present": plans, "returned_status": status}


def _observe_ordinary_receipt(value: Any, plans_present: bool) -> dict[str, Any]:
    if (type(value) is tuple and len(value) == 2 and type(value[0]) is str
            and type(plans_present) is bool):
        child = normalize_ordinary_receipt({"schema_version": 1, "observation_state": "observed",
                                            "plans_present": plans_present, "returned_status": value[0]})
        if child is not None:
            return child
    return unknown_ordinary_receipt()


class RetrievalTrace:
    """Bounded observer of existing decisions; never part of a capsule.

    Absence from the observed search window is unknown, not a rank rejection.
    Protected and missing watched documents share the same unavailable state.
    Only metadata already authorized by privacy/owner policy is collected.
    """
    def __init__(self, config: dict[str, Any], paths: Optional[list[str]] = None):
        self.config = config
        self.records = {path: {"path": path, "state": "unavailable", "events": []}
                        for path in validate_trace_paths(paths)}
        self.truncated = False
        self.search_limit: Optional[int] = None
        self.evidence_terms = 0
        self.required_terms: Optional[int] = None
        self.ordinary_receipt = None
        try:
            self.funnel = _PassiveFunnel()
        except Exception:
            self.funnel = None

    def _eligible(self, item: dict[str, Any]) -> bool:
        owners = self.config.get("owners", ["*"])
        return (
            _trace_path_allowed(item.get("path"))
            and item.get("privacy") in self.config["allowed_privacy"]
            and (item.get("privacy") != "restricted" or "*" in owners or item.get("owner") in owners)
        )

    def _record(self, item: dict[str, Any]) -> Optional[dict[str, Any]]:
        if not self._eligible(item):
            return None
        path = item["path"]
        if path not in self.records:
            if len(self.records) >= TRACE_RECORD_LIMIT:
                self.truncated = True
                return None
            self.records[path] = {"path": path, "state": "not_observed_in_bounded_search", "events": []}
        record = self.records[path]
        if record["state"] == "unavailable":
            record["state"] = "not_observed_in_bounded_search"
        digest = item.get("source_hash")
        if isinstance(digest, str) and re.fullmatch(r"[0-9a-f]{64}", digest):
            record["source_sha256"] = digest
        return record

    def watch(self, connection: sqlite3.Connection) -> None:
        for path in list(self.records):
            row = connection.execute("SELECT * FROM document_metadata WHERE path = ?", (path,)).fetchone()
            if row is not None:
                self._record(dict(row))

    def observe(self, item: dict[str, Any], stage: str, reason: str, **numbers: Any) -> None:
        self._funnel_event(item, stage, reason)
        record = self._record(item)
        if record is None:
            return
        record["state"] = "observed"
        event = {"stage": stage, "reason": reason}
        for key in ("rank", "matched_terms", "required_terms", "score", "threshold", "estimated_tokens"):
            value = numbers.get(key)
            if isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value):
                event[key] = round(value, 6)
        if len(record["events"]) < TRACE_EVENT_LIMIT:
            record["events"].append(event)
        else:
            record["events_truncated"] = True
            if stage == "delivery":
                record["events"][-1] = event

    def exclusions(self, entries: Iterable[dict[str, Any]], stage: str) -> None:
        for item in entries:
            record = self.records.get(item.get("path"))
            if record is not None and record["state"] != "unavailable":
                # Already checked metadata; a path-only exclusion cannot admit
                # a previously unseen protected record.
                previous = record["events"][-1] if record["events"] else None
                event = {"stage": stage, "reason": item["reason"]}
                if previous is not None and all(previous.get(key) == value for key, value in event.items()):
                    continue
                try:
                    if self.funnel is not None:
                        self.funnel.event(stage, item["reason"])
                except Exception:
                    self.funnel = None
                if len(record["events"]) < TRACE_EVENT_LIMIT:
                    record["events"].append(event)
                else:
                    record["events_truncated"] = True

    def known_event(self, path: str, stage: str, reason: str) -> None:
        record = self.records.get(path)
        if record is not None and record["state"] != "unavailable":
            record["state"] = "observed"
            self.exclusions([{"path": path, "reason": reason}], stage)

    def _funnel_event(self, item: dict[str, Any], stage: str, reason: str) -> None:
        try:
            if self.funnel is not None and self._eligible(item):
                self.funnel.event(stage, reason)
        except Exception:
            self.funnel = None

    def funnel_boundary(self, name: str, items: list[dict[str, Any]]) -> None:
        try:
            if self.funnel is not None:
                self.funnel.boundary(name, items, self._eligible)
        except Exception:
            self.funnel = None

    def _funnel_snapshot(self) -> Optional[dict[str, Any]]:
        try:
            if self.funnel is not None:
                result = self.funnel.snapshot()
                if _PassiveFunnel.valid(result):
                    return result
        except Exception:
            pass
        try:
            return _funnel_template("unavailable")
        except Exception:
            # Missing additive diagnostics mean unknown; preserve the legacy trace.
            return None

    def snapshot(self) -> dict[str, Any]:
        value = self._funnel_snapshot()
        try:
            receipt = normalize_ordinary_receipt(self.ordinary_receipt)
        except Exception:
            receipt = None
        return {"schema_version": 1, **({"ordinary_receipt": receipt} if receipt is not None else {}), **({"funnel": value} if value is not None else {}),
                "records": list(self.records.values()), "truncated": self.truncated,
                "search_limit": self.search_limit,
                "evidence_terms": self.evidence_terms, "required_terms": self.required_terms,
                "rank_scope": "authorized records in observed window; protected positions are omitted",
                "bounds": {"records": TRACE_RECORD_LIMIT, "events_per_record": TRACE_EVENT_LIMIT,
                           "watched_paths": TRACE_WATCH_LIMIT},
                "coverage": "existing bounded candidate windows only; unobserved causes are unknown"}


def _close_trace_descriptors(*descriptors: Optional[int]) -> bool:
    """Retired ownership only: one attempt per fd, drain before cancellation."""
    primary = sys.exc_info()[0] is not None
    cancellation = None
    complete = True
    seen = set()
    for descriptor in descriptors:
        if descriptor is None or descriptor in seen:
            continue
        seen.add(descriptor)
        try:
            os.close(descriptor)
        except (OSError, ValueError):
            complete = False  # Closure is unknown; never retry that fd number.
        except BaseException as error:
            complete = False
            if cancellation is None:
                cancellation = error
    if cancellation is not None and not primary:
        raise cancellation
    return complete


def _trace_directory(repository: Path, *, create: bool = False) -> Optional[int]:
    """Anchor every operation to directory handles; never follow replaced parents.

    Hosts lacking safe descriptor-relative opens skip optional diagnostics.
    """
    if not TRACE_SAFE_DIRECTORY_IO:
        return None
    descriptor = None
    try:
        descriptor = os.open(repository, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        for part in ("memory-bank", "local", "retrieval-traces"):
            if create:
                try:
                    os.mkdir(part, mode=0o700, dir_fd=descriptor)
                except FileExistsError:
                    pass
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=descriptor)
            parent = descriptor
            descriptor = child
            if not _close_trace_descriptors(parent):
                return None
        result, descriptor = descriptor, None
        return result
    except (OSError, ValueError):
        return None
    finally:
        owned, descriptor = descriptor, None
        _close_trace_descriptors(owned)


def _close_trace_file(handle: Any) -> bool:
    """Retired file-object ownership, one close attempt, primary preserved."""
    if handle is None:
        return True
    primary = sys.exc_info()[0] is not None
    try:
        handle.close()
        return True
    except (OSError, ValueError):
        return False
    except BaseException:
        if not primary:
            raise
        return False


def _trace_payload_json(payload: dict[str, Any]) -> str:
    candidate = payload
    if "ordinary_receipt" in payload:
        try:
            receipt = normalize_ordinary_receipt(payload["ordinary_receipt"])
        except Exception:
            receipt = None
        candidate = {key: value for key, value in payload.items() if key != "ordinary_receipt"}
        if receipt is not None:
            candidate["ordinary_receipt"] = receipt
    try:
        encoded = json.dumps(candidate, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    except Exception:
        if "ordinary_receipt" not in candidate:
            raise
        candidate = {key: value for key, value in candidate.items() if key != "ordinary_receipt"}
        encoded = json.dumps(candidate, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    if "ordinary_receipt" in candidate and len(encoded.encode("utf-8")) > TRACE_FILE_LIMIT:
        candidate = {key: value for key, value in candidate.items() if key != "ordinary_receipt"}
        encoded = json.dumps(candidate, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    if "funnel" in candidate and len(encoded.encode("utf-8")) > TRACE_FILE_LIMIT:
        # Additive diagnostics cannot displace a legacy trace or final render.
        legacy = {key: value for key, value in candidate.items() if key != "funnel"}
        encoded = json.dumps(legacy, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    return encoded


def _write_trace(directory: int, identifier: str, payload: dict[str, Any]) -> None:
    encoded = _trace_payload_json(payload).encode("utf-8")
    if len(encoded) > TRACE_FILE_LIMIT:
        return
    temporary = "." + identifier + ".tmp"
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=directory)
    try:
        handle = os.fdopen(descriptor, "wb")
        descriptor = None
        try:
            handle.write(encoded)
            handle.flush()
            os.fsync(handle.fileno())
        finally:
            owned, handle = handle, None
            confirmed = _close_trace_file(owned)
        if not confirmed:
            return
        os.replace(temporary, identifier + ".json", src_dir_fd=directory, dst_dir_fd=directory)
    finally:
        primary = sys.exc_info()[0] is not None
        owned, descriptor = descriptor, None
        cancellation = None
        try:
            _close_trace_descriptors(owned)
        except BaseException as error:
            cancellation = error
        try:
            os.unlink(temporary, dir_fd=directory)
        except OSError:
            pass
        except BaseException as error:
            if cancellation is None:
                cancellation = error
        if cancellation is not None and not primary:
            raise cancellation


def _save_trace(repository: Path, identifier: str, trace: RetrievalTrace, config: dict[str, Any]) -> None:
    # A diagnostics failure cannot withhold the capsule or change delivery state.
    directory = None
    try:
        if not TRACE_ID.fullmatch(identifier):
            return
        directory = _trace_directory(repository, create=True)
        if directory is None:
            return
        payload = {**trace.snapshot(), "id": identifier, "created_at": utc_now()}
        _write_trace(directory, identifier, payload)
        files = []
        for name in os.listdir(directory):
            if name.endswith(".json") and TRACE_ID.fullmatch(name[:-5]):
                try:
                    info = os.stat(name, dir_fd=directory, follow_symlinks=False)
                    if stat.S_ISREG(info.st_mode):
                        files.append((info.st_mtime_ns, name))
                except OSError:
                    continue
        current_name = identifier + ".json"
        if not any(name == current_name for _, name in files):
            # Some filesystems enumerate the directory handle's opening
            # snapshot. The new file is still reachable through anchored stat.
            info = os.stat(current_name, dir_fd=directory, follow_symlinks=False)
            if stat.S_ISREG(info.st_mode):
                files.append((info.st_mtime_ns, current_name))
        files.sort()
        victims = [entry for entry in files if entry[1] != identifier + ".json"]
        for _, name in victims[:max(0, len(files) - local_manifest_retention(config))]:
            try:
                os.unlink(name, dir_fd=directory)
            except OSError:
                pass
    except (OSError, ValueError, TypeError):
        pass
    finally:
        owned, directory = directory, None
        _close_trace_descriptors(owned)


def record_render_trace(repository: Path, capsule: dict[str, Any], render: dict[str, Any]) -> None:
    """Finalize an existing sidecar from the actual emitter, never a preview.

    Open the existing inode: a concurrent retention prune must not resurrect it.
    Preserve creation mtime so rendering does not extend the retention window.
    """
    descriptor = None
    directory = None
    try:
        identifier = Path(str(capsule.get("manifest", ""))).stem
        if not TRACE_ID.fullmatch(identifier):
            return
        directory = _trace_directory(repository)
        if directory is None:
            return
        descriptor = os.open(identifier + ".json", os.O_RDWR | os.O_NOFOLLOW, dir_fd=directory)
        info = os.fstat(descriptor)
        if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or info.st_size > TRACE_FILE_LIMIT:
            return
        handle = os.fdopen(descriptor, "r+", encoding="utf-8", newline="")
        descriptor = None
        try:
            data = json.load(handle)
            allowed = {item["path"] for item in data["records"] if item["state"] != "unavailable"}
            data["render"] = {"sha256": render["sha256"], "bytes": render["bytes"], "form": render.get("form", "capsule_text"),
                              "records": [item for item in render["records"] if item["path"] in allowed]}
            if "funnel" in data:
                try:
                    data["funnel"] = _PassiveFunnel.with_render(data["funnel"], data["render"]["records"])
                except Exception:
                    try:
                        data["funnel"] = _funnel_template("unavailable")
                    except Exception:
                        data.pop("funnel", None)
            encoded = _trace_payload_json(data)
            if len(encoded.encode("utf-8")) > TRACE_FILE_LIMIT:
                return
            handle.seek(0)
            handle.write(encoded)
            handle.truncate()
            handle.flush()
            os.utime(handle.fileno(), ns=(info.st_atime_ns, info.st_mtime_ns))
        finally:
            owned, handle = handle, None
            _close_trace_file(owned)
    except (OSError, ValueError, TypeError, KeyError, RecursionError):
        pass
    finally:
        owned, descriptor, directory = (descriptor, directory), None, None
        _close_trace_descriptors(*owned)


def _window_json_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("Ambiguous observation object")
        result[key] = value
    return result


# Diagnosis consumes only this invocation's approved, bounded metadata sidecar.
# It never certifies source absence or requests a retrieval/scoring experiment.
FUNNEL_DIAGNOSIS_LOSSES = {
    "term_coverage": ("admission", "term-coverage"),
    "relative_score_floor": ("ranking", "relative-score-floor"),
    "budget": ("budget", "budget"), "hard_budget": ("budget", "hard-budget"),
    "layer": ("layer", "layer-limit"), "repeat": ("layer", "delivered-this-session"),
    "gate": ("gate", "withheld"), "pack": ("capsule", "capsule-limit"),
    "authority": ("runtime", "authority"), "lifecycle": ("runtime", "lifecycle"),
    "stale": ("runtime", "stale"), "source_missing": ("runtime", "source-missing"),
    "map_unverifiable": ("runtime", "map-unverifiable"), "map_drift": ("runtime", "map-drift"),
}


def unknown_funnel_diagnosis() -> dict[str, Any]:
    return {
        "schema_version": 1, "observation_state": "unavailable",
        "source_coverage": "unknown", "quality": "unmeasured",
        "count_scope": "authorized-observation-occurrences",
        "coverage": {"history_pre_admission": "not-instrumented", "episodes": "not-instrumented",
                     "working_sources": "not-instrumented", "path_only_events": "authorized-retained-records-only"},
        "loss_observations": {key: _funnel_counter() for key in FUNNEL_DIAGNOSIS_LOSSES},
        "boundaries": {key: _funnel_counter() for key in FUNNEL_BOUNDARIES},
        "render_projection": {"scope": "authorized-retained-records",
                              "headers": _funnel_counter(),
                              "reasons": {key: _funnel_counter() for key in FUNNEL_RENDER_REASONS}},
        "emission": "unknown", "consumer": "unknown", "next_action": "unknown",
    }


def valid_funnel_diagnosis(value: Any) -> bool:
    template = unknown_funnel_diagnosis()
    if (not isinstance(value, dict) or set(value) != set(template)
            or type(value["schema_version"]) is not int or value["schema_version"] != 1
            or value["observation_state"] not in ("observed", "unavailable")
            or value["next_action"] not in ("unknown", "inspect-observed-loss")
            or any(value[key] != template[key] for key in
                   ("source_coverage", "quality", "count_scope", "coverage", "emission", "consumer"))):
        return False
    def counter(item: Any) -> bool:
        return (isinstance(item, dict) and set(item) == {"count", "state"}
                and (item == {"count": None, "state": "unknown"}
                     or item["state"] in ("exact", "lower-bound") and type(item["count"]) is int
                     and 0 <= item["count"] <= FUNNEL_COUNT_LIMIT
                     and (item["state"] != "lower-bound" or item["count"] == FUNNEL_COUNT_LIMIT)))
    for key in ("loss_observations", "boundaries"):
        if (not isinstance(value[key], dict) or set(value[key]) != set(template[key])
                or not all(counter(item) for item in value[key].values())):
            return False
    render = value["render_projection"]
    if (not isinstance(render, dict) or set(render) != set(template["render_projection"])
            or render["scope"] != template["render_projection"]["scope"] or not counter(render["headers"])
            or not isinstance(render["reasons"], dict) or set(render["reasons"]) != set(FUNNEL_RENDER_REASONS)
            or not all(counter(item) for item in render["reasons"].values())):
        return False
    if value["observation_state"] == "unavailable":
        return value == template
    action = ("inspect-observed-loss" if any(type(item["count"]) is int and item["count"] > 0
              for item in value["loss_observations"].values()) else "unknown")
    return value["next_action"] == action


def diagnose_funnel(repository: Path, capsule: Any, expected_render: Any, _ordinary_receipt_sink: Any = None) -> dict[str, Any]:
    """One anchored metadata read; every unavailable/rejected state is identical."""
    directory = descriptor = None
    receipt = None
    sink = _ordinary_receipt_sink if type(_ordinary_receipt_sink) is dict and len(_ordinary_receipt_sink) == 0 else None
    def finish(result):
        if sink is not None:
            try:
                child = normalize_ordinary_receipt(receipt) if receipt is not None else unknown_ordinary_receipt()
                if child is not None:
                    sink["ordinary_receipt"] = child
            except Exception:
                pass
        return result
    try:
        if not isinstance(capsule, dict) or not isinstance(expected_render, dict):
            return finish(unknown_funnel_diagnosis())
        reference = capsule.get("manifest")
        if (not isinstance(reference, str) or re.fullmatch(
                r"(?:project-brain/control|memory-bank/local)/retrieval-manifests/[0-9a-f-]{36}\.json",
                reference) is None):
            return finish(unknown_funnel_diagnosis())
        identifier = reference.rsplit("/", 1)[1][:-5]
        form, length, digest = (expected_render.get(key) for key in ("form", "bytes", "sha256"))
        if (TRACE_ID.fullmatch(identifier) is None or form not in ("capsule_text", "printed_capsule")
                or type(length) is not int or not 0 <= length <= TRACE_FILE_LIMIT or not _window_digest(digest)):
            return finish(unknown_funnel_diagnosis())
        if not callable(getattr(os, "getuid", None)):
            return finish(unknown_funnel_diagnosis())
        directory = _trace_directory(repository)
        if directory is None:
            return finish(unknown_funnel_diagnosis())
        filename = identifier + ".json"
        descriptor = os.open(filename, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
        before = os.fstat(descriptor)
        if (not stat.S_ISREG(before.st_mode) or before.st_nlink != 1 or before.st_size > TRACE_FILE_LIMIT
                or before.st_uid != os.getuid()):
            return finish(unknown_funnel_diagnosis())
        encoded = os.read(descriptor, TRACE_FILE_LIMIT + 1)
        after = os.fstat(descriptor)
        current = os.stat(filename, dir_fd=directory, follow_symlinks=False)
        attributes = ("st_dev", "st_ino", "st_size", "st_mtime_ns", "st_ctime_ns", "st_nlink", "st_uid")
        if (len(encoded) > TRACE_FILE_LIMIT or len(encoded) != before.st_size or after.st_nlink != 1
                or any(getattr(before, key) != getattr(after, key) or getattr(after, key) != getattr(current, key)
                       for key in attributes)):
            return finish(unknown_funnel_diagnosis())
        data = json.loads(encoded, object_pairs_hook=_window_json_object)
        if (not isinstance(data, dict) or type(data.get("schema_version")) is not int
                or data["schema_version"] != 1 or data.get("id") != identifier):
            return finish(unknown_funnel_diagnosis())
        render, funnel = data.get("render"), data.get("funnel")
        if (not isinstance(render, dict) or render.get("form") != form or type(render.get("bytes")) is not int
                or render["bytes"] != length or render.get("sha256") != digest):
            return finish(unknown_funnel_diagnosis())
        try:
            receipt = normalize_ordinary_receipt(data.get("ordinary_receipt"))
        except Exception:
            receipt = None
        if (not _PassiveFunnel.valid(funnel) or funnel["observer_state"] != "active"
                or funnel["render"]["projection_state"] != "observed"):
            return finish(unknown_funnel_diagnosis())
        result = unknown_funnel_diagnosis()
        result["observation_state"] = "observed"
        result["loss_observations"] = {key: copy.deepcopy(funnel["events"][stage][reason])
                                      for key, (stage, reason) in FUNNEL_DIAGNOSIS_LOSSES.items()}
        result["boundaries"] = copy.deepcopy(funnel["boundaries"])
        result["render_projection"].update(headers=copy.deepcopy(funnel["render"]["headers"]),
                                          reasons=copy.deepcopy(funnel["render"]["reasons"]))
        if any(type(item["count"]) is int and item["count"] > 0 for item in result["loss_observations"].values()):
            result["next_action"] = "inspect-observed-loss"
        return finish(result if valid_funnel_diagnosis(result) else unknown_funnel_diagnosis())
    except Exception:
        return finish(unknown_funnel_diagnosis())
    finally:
        for handle in (descriptor, directory):
            if handle is not None:
                try:
                    os.close(handle)
                except Exception:
                    pass


def _read_window_render(repository: Path, reference: dict[str, Any], doc_ref: str) -> Optional[dict[str, Any]]:
    """Read bounded final-render metadata through existing secured handles."""
    descriptor = directory = None
    try:
        identifier = reference.get("manifest_id")
        if (not isinstance(identifier, str) or TRACE_ID.fullmatch(identifier) is None
                or reference.get("doc_ref") != _window_key(doc_ref)):
            return None
        directory = _trace_directory(repository)
        if directory is None:
            return None
        descriptor = os.open(identifier + ".json", os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
        info = os.fstat(descriptor)
        if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or info.st_size > TRACE_FILE_LIMIT:
            return None
        encoded = os.read(descriptor, TRACE_FILE_LIMIT + 1)
        if len(encoded) > TRACE_FILE_LIMIT:
            return None
        data = json.loads(encoded, object_pairs_hook=_window_json_object)
        if not isinstance(data, dict) or data.get("id") != identifier:
            return None
        records = data.get("records")
        render = data.get("render")
        if (not isinstance(records, list) or len(records) > TRACE_RECORD_LIMIT
                or not isinstance(render, dict) or render.get("form") not in ("capsule_text", "printed_capsule")):
            return None
        sources = [item for item in records if isinstance(item, dict) and item.get("path") == doc_ref]
        if len(sources) != 1 or sources[0].get("state") != "observed":
            return None
        events = sources[0].get("events")
        if (not isinstance(events, list) or len(events) > TRACE_EVENT_LIMIT
                or not any(isinstance(event, dict) and event.get("stage") == "delivery"
                           and event.get("reason") == "selected" for event in events)):
            return None
        bodies = render.get("records")
        if not isinstance(bodies, list) or len(bodies) > TRACE_RECORD_LIMIT:
            return None
        bodies = [item for item in bodies if isinstance(item, dict) and item.get("path") == doc_ref]
        if len(bodies) != 1:
            return None
        body = bodies[0]
        length, digest, reason = body.get("body_bytes"), body.get("body_sha256"), body.get("body_reason")
        if (body.get("header_present") is not True or not _window_digest(digest)
                or type(length) is not int or not 0 <= length <= WINDOW_BODY_BYTES_LIMIT
                or type(body.get("pointer_only")) is not bool or body["pointer_only"] != (length == 0)
                or type(body.get("shrunk")) is not bool
                or reason not in ("excerpt", "render-limit", "sanitized-empty", "no-excerpt")
                or (length > 0) != (reason == "excerpt")
                or length == 0 and digest != hashlib.sha256(b"").hexdigest()):
            return None
        result = {key: body[key] for key in ("body_bytes", "body_sha256", "body_reason", "pointer_only", "shrunk")}
        initial = body.get("initial_excerpt_bytes")
        if type(initial) is int and length <= initial <= WINDOW_BODY_BYTES_LIMIT:
            result["initial_excerpt_bytes"] = initial
        source_hash = sources[0].get("source_sha256")
        if _window_digest(source_hash):
            result["source_sha256"] = source_hash
        return result
    except (OSError, ValueError, TypeError, KeyError, RecursionError):
        return None
    finally:
        if descriptor is not None:
            os.close(descriptor)
        if directory is not None:
            os.close(directory)


def _observe_repeat_window(
    trace: RetrievalTrace, repository: Path, item: dict[str, Any],
    key: str, revision: str, seen: list[Any], turn: int,
    previous: dict[str, Any], proposals: dict[str, Any],
) -> None:
    """Add exposure metadata only; existing handed decisions remain authority."""
    record = trace._record(item)
    if record is None:
        return
    observation: dict[str, Any] = {"classification": "unknown-proposal", "scope": "document-pre-novelty-pre-pack"}
    record["window_observation"] = observation
    proposal = proposals.get(item["path"])
    if not isinstance(proposal, dict) or set(proposal) != {"sha256", "bytes", "state"}:
        return
    digest, length, state = proposal["sha256"], proposal["bytes"], proposal["state"]
    if (not _window_digest(digest) or type(length) is not int or not 0 <= length <= WINDOW_BODY_BYTES_LIMIT
            or state not in ("excerpt", "sanitized-empty", "no-excerpt")
            or (length > 0) != (state == "excerpt")
            or length == 0 and digest != hashlib.sha256(b"").hexdigest()):
        return
    observation.update(proposal_sha256=digest, proposal_bytes=length, proposal_state=state,
                       receipt_age=turn - seen[1], classification="empty-proposal" if not length else "unknown-prior-render")
    if not length:
        return
    refs = _window_references(previous, previous.get("items", {}))
    reference = refs.get(_window_key(key))
    if (reference is None or reference["revision"] != revision or reference["handed_turn"] != seen[1]):
        return
    prior = _read_window_render(repository, reference, item["path"])
    if prior is None:
        return
    source = prior.pop("source_sha256", None)
    observation["prior"] = prior
    current = item.get("source_hash")
    observation["source_relation"] = (
        "unknown-source" if not _window_digest(source) or not _window_digest(current)
        else "same-source" if source == current else "changed-source")
    observation["classification"] = (
        "same-window" if digest == prior["body_sha256"] and length == prior["body_bytes"]
        else "different-exposure-window")


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
class _IndexImageCapsule(dict):
    """Operation-local renderer input, deliberately outside public JSON keys."""
    __slots__ = ('_admitted_images',)

    def __init__(self, values, images):
        super().__init__(values)
        object.__setattr__(self, '_admitted_images', tuple(images))

    def __setattr__(self, name, value):
        raise AttributeError('Admitted images are immutable')

    def __deepcopy__(self, memo):
        result = _IndexImageCapsule(copy.deepcopy(dict(self), memo), self._admitted_images)
        memo[id(self)] = result
        return result


def _selected_index_images(items):
    return {item['path']: item.get('_indexed_policy_image') for item in items}


def _capsule_index_images(capsule):
    return dict(capsule._admitted_images) if type(capsule) is _IndexImageCapsule else None


def _with_index_images(capsule, images):
    paths = {item['path'] for key in ('procedural', 'semantic', 'episodic', 'selected')
             for item in capsule.get(key, []) if isinstance(item, dict) and isinstance(item.get('path'), str)}
    admitted = [(name, image) for name, image in images.items() if name in paths
                and isinstance(image, tuple) and len(image) == 3
                and all(isinstance(value, str) for value in image)]
    return _IndexImageCapsule(capsule, admitted)


def _carry_index_images(source, capsule):
    images = _capsule_index_images(source)
    return _with_index_images(capsule, images) if images is not None else capsule


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
    if item.get("via"):
        # The path an automatic link came through; capsule JSON only.
        public["via"] = item["via"]
    return public


def _capsule_projection(selected: list[dict[str, Any]], local_episode_selected: list[dict[str, Any]]) -> dict[str, Any]:
    """Preserve the exact category/history/related ordering of capsule layers."""
    groups = {category: [] for category in BUDGETS}
    # Conflict partners are both delivered by contract; the capsule names the
    # other side so a reader does not have to infer it from a bare mark.
    paths_by_record = {
        item.get("record_id"): str(item.get("path"))
        for item in selected
        if isinstance(item.get("record_id"), str) and item.get("path")
    }
    for item in selected:
        public = _public_item(item)
        conflicts = item.get("conflicts")
        if isinstance(conflicts, list):
            partners = [
                paths_by_record[record_id]
                for record_id in conflicts
                if isinstance(record_id, str)
                and record_id in paths_by_record
                and record_id != item.get("record_id")
            ]
            if partners:
                public["conflict"] = partners
        groups[item["category"]].append(public)
    procedural = groups["policy"][:CAPSULE_PROCEDURAL_LIMIT]
    history_items = [
        item for group in groups.values() for item in group if item["layer"] == "episodic"
    ]
    # Recorded history (an event, or a local episode) first, then the
    # changelog: the text ceiling shrinks from the tail, and the changelog's
    # long sections are what gives way.
    recorded = [item for item in history_items if item["kind"] == HISTORY_KIND][:CAPSULE_EVENT_LIMIT]
    recorded.extend(local_episode_selected[: CAPSULE_EVENT_LIMIT - len(recorded)])
    episodic = [
        *recorded,
        *[item for item in history_items if item["kind"] != HISTORY_KIND][:CAPSULE_EPISODIC_LIMIT],
    ]
    # Episodic items are history, not knowledge slots: a delivered event used
    # to be counted here too (category `dynamic`), then removed as a
    # duplicate, and the third semantic item was lost with it.
    semantic = [
        item
        for category in ("handoff", "durable", "dynamic", "evidence")
        for item in groups[category]
        if item["layer"] != "episodic"
    ][:CAPSULE_SEMANTIC_LIMIT]
    # Category grouping must not move related knowledge ahead of direct
    # hits when the renderer spends its character allowance.
    semantic.sort(key=lambda item: (item.get("selection") == "source-hint",
                                   item.get("selection") in ("source-link", *AUTOMATIC_LINK_SELECTIONS)))
    return {"categories": groups, "procedural": procedural, "semantic": semantic, "episodic": episodic}


SOURCE_PROVENANCE_PROFILE = 'local-source-provenance-v1'
SOURCE_PROVENANCE_BYTES = 65536


def local_source_provenance(capsule: dict[str, Any], manifest: dict[str, Any]) -> Optional[dict[str, Any]]:
    """Same-invocation transport consistency, never a disk rehash/authenticator."""
    visible, _ = _shown_items(capsule)
    rows = [{'path': row['path'], 'source_hash': row['source_hash']}
            for row in manifest['selected'] if row['path'] in visible]
    paths = [row['path'] for row in rows]
    if len(rows) > 32 or len(set(paths)) != len(paths) or set(paths) != visible:
        return None
    try:
        frame = json.dumps(capsule, ensure_ascii=False, separators=(',', ':'),
                           sort_keys=True, allow_nan=False).encode('utf-8')
        result = {'schema_version': 1, 'profile': SOURCE_PROVENANCE_PROFILE,
                  'scope': 'local', 'publication': 'unconfirmed',
                  'task_uuid': capsule.get('task_uuid'), 'task_revision': capsule.get('task_revision'),
                  'capsule_sha256': hashlib.sha256(frame).hexdigest(), 'selected': rows}
        encoded = json.dumps(result, ensure_ascii=False, separators=(',', ':'), allow_nan=False).encode('utf-8')
        return result if len(encoded) <= SOURCE_PROVENANCE_BYTES else None
    except (ValueError, TypeError, UnicodeError, RecursionError):
        return None


def _read_final_working_record(source_file):
    """Read a regular descriptor; a POSIX FIFO replacement cannot block open.

    This is a single descriptor observation, not whole-filesystem stability or
    a deadline for regular/network file reads. Unsupported types are withheld.
    """
    descriptor = os.open(source_file, os.O_RDONLY | getattr(os, "O_NONBLOCK", 0)
                         | getattr(os, "O_NOFOLLOW", 0))
    try:
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):
            raise OSError("Working record is not a regular file")
        stream = os.fdopen(descriptor, "r", encoding="utf-8")
        descriptor = None
        try:
            content = stream.read()
        except BaseException:
            # A cleanup OSError must not turn cancellation into an ordinary
            # working-unavailable result. A new cleanup cancellation escapes.
            try:
                stream.close()
            except Exception:
                pass
            raise
        stream.close()
        return content
    finally:
        if descriptor is not None:
            active_error = sys.exc_info()[1]
            try:
                os.close(descriptor)
            except Exception:
                if active_error is None:
                    raise


def _working_record_image(record, body):
    return hashlib.sha256(json.dumps([record, body], ensure_ascii=False,
        sort_keys=True, separators=(",", ":"), allow_nan=False).encode("utf-8")).hexdigest()


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
    query_fallback: Optional[str] = None,
    phase_seconds: Optional[dict[str, Any]] = None,
    gate_mode: str = RETRIEVAL_GATE_DEFAULT,
    paths: Optional[list[str]] = None,
    source_hints: Any = None,
    host: str = "cli",
    entry_point: str = "retrieve",
    history_text: Optional[str] = None,
    automatic_seeds: Optional[dict[str, list[str]]] = None,
    session_id: Optional[str] = None,
    transcript: Optional[str] = None,
    pack: Optional[Callable[[dict[str, Any]], dict[str, Any]]] = None,
    trace_paths: Optional[list[str]] = None,
    prepare_excerpts: Optional[Callable[[sqlite3.Connection, list[dict[str, Any]], str], None]] = None,
    describe_windows: Optional[Callable[[sqlite3.Connection, dict[str, Any]], dict[str, Any]]] = None,
    delivery_profile: str = 'ordinary',
    source_provenance_sink: Optional[dict[str, Any]] = None,
    admit_delivery: Optional[Callable[[], None]] = None,
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

    ``history_text`` is the text recorded history is searched with - the whole
    request, where ``query`` is its distilled form; ``query`` when omitted.

    ``automatic_seeds`` - only automatic entry points pass it - names paths
    the request mentions (`prompt`) and the current task touched (`touched`);
    one document linked to them may join the semantic layer after the strong
    matches (`_automatic_link_candidates`).

    ``pack`` turns the result into the capsule as the caller delivers it -
    the layer and character limits of its JSON, the ceiling of its rendered
    text - and returns it. Only what it still shows counts as delivered: the
    conversation's repeat record, the manifest's selection and the token
    estimates are written after it, and what it left out is excluded as
    ``capsule-limit``. Without it the whole selection counts as delivered.
    """
    retrieval_started = time.monotonic()
    if source_provenance_sink is not None:
        if type(source_provenance_sink) is not dict:
            raise RetrievalError('Invalid source provenance transport')
        source_provenance_sink.clear()
    if delivery_profile not in ('ordinary','cursor-rule') or (delivery_profile == 'cursor-rule' and
            (host != 'cursor' or entry_point not in ('refresh','hook-context') or session_id is not None or transcript is not None)):
        raise RetrievalError('Unsupported persistent delivery profile')
    if limit < 1:
        raise RetrievalError("--limit must be a positive integer")
    if query_source not in QUERY_SOURCES:
        raise RetrievalError(
            f"Query source must be one of {', '.join(sorted(QUERY_SOURCES))}"
        )
    if query_fallback:
        # A short imperative continuation ("fix it", "do it") carries no
        # subject. The task it continues does. Keep the prompt's own terms
        # first and add the task's terms behind them; the manifest records the
        # combined query, so the turn can still be audited.
        try:
            combined = list(dict.fromkeys([
                *query_tokens(query), *query_tokens(query_fallback)
            ]))
        except RetrievalError:
            combined = query_tokens(query_fallback) if query_fallback else []
        if combined:
            query = " ".join(combined)
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
        task_path, task, task_body = None, None, None
        manifest_scope = "local"
    else:
        task_path, task, task_body = find_task(repository, task_identifier)
        validate_record(task)
    initial_working_image = _working_record_image(task, task_body) if task is not None else None
    config = load_config(repository)
    trace = RetrievalTrace(config, trace_paths)
    ensure_metadata_tables(connection)
    trace._current_source_policy = _CurrentSourcePolicy(connection, repository)
    try:
        trace.watch(connection)
    except sqlite3.Error:
        # A failed observational lookup must not prevent the existing search.
        pass
    trace.search_limit = max(20, limit * 10)
    candidates, diagnostics = _candidates(connection, query, max(20, limit * 10), trace=trace)
    # Recorded history has its own search over the whole request: events and
    # local episodes share one slot beside the changelog's.
    request_text = query if history_text is None else history_text
    known_tickets = _known_ticket_prefixes(connection, task)
    request_tickets = _request_tickets(request_text, task, known_tickets)
    history = history_query(connection, request_text)
    if history is not None:
        history["tickets"] = request_tickets
        history["known_tickets"] = known_tickets
    event_candidates, episode_candidates = (
        _history_candidates(connection, history, trace=trace) if history else ([], [])
    )
    # Taken before any filter runs, so "nothing matched" cannot be confused
    # with "everything that matched was withheld".
    matched_layers = {item["layer"] for item in candidates}
    if event_candidates or episode_candidates:
        matched_layers.add("episodic")
    no_match = [
        layer
        for layer in ("procedural", "semantic", "episodic")
        if layer not in matched_layers
    ]
    trace._current_source_policy.prime([*candidates, *event_candidates])
    filtered, filter_excluded = _runtime_filter(repository, candidates, config, trace=trace, connection=connection)
    # A candidate whose cited file changed kept a reduced score; ranking it
    # again keeps fresh knowledge of equal fit ahead of it. Stable, so
    # nothing else moves.
    filtered.sort(key=lambda item: -float(item.get("adjusted_score") or 0.0))
    exact_events = [item for item in event_candidates if item.get("_ticket_match")]
    chosen_event, history_excluded = _best_history_event(
        repository, exact_events if exact_events else event_candidates, config,
        trace=trace,
        connection=connection,
        # A same-ticket series is only a series when events carry the ticket
        # the request names; a ticket no event carries - or "PHP 8.3" - left
        # two neighbours in the slot, and neither was the answer.
        limit=CAPSULE_EVENT_LIMIT if exact_events else 1,
        known=known_tickets,
    )
    filter_excluded.extend(history_excluded)
    history_episode: list[dict[str, Any]] = []
    if episode_candidates and (
        not chosen_event or episode_candidates[0][0] > chosen_event[0]["adjusted_score"]
    ):
        # Ties go to the event: it is the team's record, the episode a
        # machine-local note of the same kind.
        history_episode, chosen_event = [episode_candidates[0][2]], []
    history_paths = {item["path"] for item in chosen_event}
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
            {item["path"] for item in filtered} | history_paths,
            trace=trace,
        )
        path_matched_count = len(linked) + len(link_excluded)
        filter_excluded.extend(link_excluded)
        # Ahead of the query's own matches, because `_apply_budgets` and the
        # per-layer ladder both honour input order and the caller named these.
        filtered = [*linked, *filtered]
    if automatic_seeds is not None:
        own_paths: set[str] = set()
        if task_path is not None and task is not None:
            own_paths = {
                task_path.relative_to(repository).as_posix(),
                handoff_path(repository, task["id"]).relative_to(repository).as_posix(),
            }
        task_files = list(reversed(task["files"])) if task is not None else []
        seeds = [(path, "prompt") for path in automatic_seeds.get("prompt", [])[:PROMPT_SEED_LIMIT]]
        touched = [
            path for path in [*automatic_seeds.get("touched", []), *task_files]
            if _file_link_problem(path) is None
        ]
        seeds += [(path, "touch") for path in list(dict.fromkeys(touched))[:TOUCH_SEED_LIMIT]]
        unique: dict[str, tuple[str, str]] = {}
        for path, origin in seeds:
            unique.setdefault(path, (path, origin))
        automatic_linked, automatic_excluded = _automatic_link_candidates(
            connection, repository, config, list(unique.values()), filtered,
            own_paths=own_paths | history_paths, coverage=diagnostics.get("coverage") or {},
            trace=trace,
        )
        filter_excluded.extend(automatic_excluded)
        filtered = _insert_automatic_links(filtered, automatic_linked)
    graph_candidates, graph_stats = _source_link_candidates(
        connection, repository, config, graph_anchors,
        {item["path"] for item in filtered} | history_paths,
        trace=trace,
    )
    # Related knowledge fills remaining capacity after direct matches and
    # explicit paths; provenance alone never gives it priority over them.
    filtered.extend(graph_candidates)
    filtered, ticket_excluded = _ticket_filter(
        filtered, request_tickets, connection=connection, repository=repository,
        request_text=request_text, known=known_tickets,
    )
    filter_excluded.extend(ticket_excluded)
    # The history search's event goes through the same conflict expansion,
    # exclusions and budget as everything else, ahead in the budget queue.
    filtered = [*chosen_event, *filtered]
    hint_candidates = []
    hint_stats = None
    if source_hints is not None:
        references, hint_state = normalize_source_hints(source_hints)
        if not _source_hint_schema_supported(repository):
            references, hint_state = [], "unsupported"
        if hint_state == "ready":
            hint_candidates = _source_hint_candidates(
                connection, repository, config, references,
                {item["path"] for item in filtered} | history_paths, trace=trace)
        # Only admitted allowed rows are counted; refused identities stay private.
        if _source_hint_schema_supported(repository):
            hint_stats = {"profile": SOURCE_HINT_PROFILE, "state": hint_state,
                          "supplied": len(references), "admitted": len(hint_candidates), "delivered": 0}
        filtered.extend(hint_candidates)
    # A promoted chunk and the record it was promoted from say the same thing,
    # and both used to take a slot. The chunk is the durable form, so the
    # record yields to it.
    promoted_from = {
        str(source.get("path"))
        for item in filtered
        if item.get("kind") == "memory" and item.get("selection") not in ("source-link", "source-hint")
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
            repository, conflict_candidates, config, trace=trace
        , connection=connection)
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
    filtered.sort(key=lambda item: item.get("selection") == "source-hint")
    for position, item in enumerate(filtered, start=1):
        trace.observe(item, "queue", item.get("selection") or "lexical",
                      rank=position, estimated_tokens=item.get("estimated_tokens"),
                      score=float(item.get("adjusted_score") or 0.0))
    trace.exclusions(filter_excluded, "runtime")
    trace.funnel_boundary("queue_after_runtime", filtered)
    selected, budget_excluded, usage, escalation_reason = _apply_budgets(filtered)
    trace.funnel_boundary("budget_admitted", selected)
    trace.exclusions(budget_excluded, "budget")
    for item in selected:
        trace.observe(item, "budget", "admitted", estimated_tokens=item.get("estimated_tokens"))
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
    # Nor does the pick go into the capsule. Every host lists its skills and
    # loads its instruction files itself, and in real sessions agents took
    # none of the 71 skills a capsule named - no Skill call, no SKILL.md read
    # on those turns - while accelerator skills were used on 1.1% of Claude
    # turns at all. The line cost room the answer-bearing excerpts need; the
    # pick stays in the manifest as `host-listed` and `search` finds skills.
    listed_paths = {item["path"] for item in procedural_ranked}
    procedural_ranked = []
    # The semantic layer is built from categories and the episodic layer from
    # the layer column, and the two taxonomies overlap: `category_for` has no
    # `changelog` branch, so CHANGELOG.md is category 'evidence' AND layer
    # 'episodic'. Without this filter it takes one of the three semantic slots
    # and an episodic slot at once, and the degradation ladder then
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
    episodic_ranked = [
        item for item in selected
        if item["layer"] == "episodic" and item["kind"] != HISTORY_KIND
    ]
    # The history search's event first; an event a path or a conflict brought
    # in only after it.
    event_ranked = [
        item for item in selected
        if item["layer"] == "episodic" and item["kind"] == HISTORY_KIND
    ]
    capsule_selected = [
        *procedural_ranked[:CAPSULE_PROCEDURAL_LIMIT],
        *semantic_ranked[:CAPSULE_SEMANTIC_LIMIT],
        *event_ranked[:CAPSULE_EVENT_LIMIT],
        *episodic_ranked[:CAPSULE_EPISODIC_LIMIT],
    ]
    local_episode_selected = history_episode[
        : max(0, CAPSULE_EVENT_LIMIT - len(event_ranked[:CAPSULE_EVENT_LIMIT]))
    ]
    local_episode_tokens = sum(
        _estimate_tokens(_serialized_episode(episode))
        for episode in local_episode_selected
    )
    selected_tokens = sum(item["estimated_tokens"] for item in capsule_selected)
    # The local episode was selected independently of the optional queue. Give
    # its existing budget back by dropping hints, never by discarding history.
    if local_episode_selected and selected_tokens + local_episode_tokens > TARGET_BUDGET:
        for item in reversed(capsule_selected[:]):
            if item.get("selection") == "source-hint":
                capsule_selected.remove(item)
                selected_tokens -= item["estimated_tokens"]
                budget_excluded.append({"path": item["path"], "reason": "budget"})
                if selected_tokens + local_episode_tokens <= TARGET_BUDGET:
                    break
    if local_episode_selected and selected_tokens + local_episode_tokens > TARGET_BUDGET:
        local_episode_selected = []
        budget_excluded.append({"path": "local-episode", "reason": "budget"})
    trace.funnel_boundary("layer_pre_novelty", capsule_selected)
    # Optional maintainer experiment: eligibility and episode admission are
    # already fixed; prepare the quote before session novelty is decided.
    if prepare_excerpts is not None:
        prepare_excerpts(connection, capsule_selected, query)
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
    session_expected = _receipt_expectation(session_deliveries, session) if session else None
    window_proposals: dict[str, Any] = {}
    if session and describe_windows is not None:
        try:
            proposed = describe_windows(connection, copy.deepcopy(_with_index_images({
                "query": query, **_capsule_projection(capsule_selected, local_episode_selected)},
                _selected_index_images(capsule_selected))))
            if isinstance(proposed, dict):
                window_proposals = proposed
        except Exception:
            pass
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
                if describe_windows is not None:
                    try:
                        _observe_repeat_window(trace, repository, item, key, revision, recent[key],
                                               session_turn, previous_entry, window_proposals)
                    except Exception:
                        pass
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
    if session:
        trace.funnel_boundary("repeat_withheld", repeated)
    capsule_paths = {item["path"] for item in capsule_selected}
    layer_excluded = [
        {
            "path": item["path"],
            # A document held back because its own layer will carry it is not
            # a document that ran out of room.
            "reason": (
                "episodic-layer" if item["layer"] == "episodic"
                else "skill-subfile" if item["path"] in vacated_paths
                else "host-listed" if item["path"] in listed_paths
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
    # Another session's turn checkpoint can rewrite the task record while this
    # retrieval runs - with several sessions on one checkout, every few turns.
    # The working state is taken again here, just before it is signed and
    # packed, so the custody check below withdraws it only for a rewrite in
    # the last moments. A query derived from the task keeps the state it was
    # derived from.
    if task is not None and query_source not in ("task", "continuation"):
        try:
            again_text = _read_final_working_record(task_path)
            again_task, again_body = parse_markdown_record(task_path, text=again_text)
            if (again_task.get("id") == task["id"]
                    and _working_record_image(again_task, again_body) != initial_working_image):
                validate_record(again_task)
                if record_is_eligible(repository, again_task, load_config(repository))[0]:
                    task, task_body = again_task, again_body
                    initial_working_image = _working_record_image(task, task_body)
        except (BrainError, OSError, UnicodeError, ValueError, TypeError):
            pass
    # Decided here rather than earlier because `selection_identical_to_
    # previous_turn` is a claim about what the capsule delivers, and after the
    # episodic-layer filter and the 2+3 truncation above that is only now
    # known. Deciding earlier would describe a set the capsule never carried.
    signature = _retrieval_signature(
        query,
        [
            *(item.get("_excerpt_identity", (item["path"], item["source_hash"])) for item in selected),
            *(_episode_signature(episode) for episode in local_episode_selected),
        ],
        task["revision"] if task is not None else 0,
    )
    baseline_key = _retrieval_baseline_key(
        own_id if own_id is not None else "no-task", host, entry_point, delivery_profile
    )
    baseline_snapshot = _load_last_retrievals(connection)
    baseline_expected = _receipt_expectation(baseline_snapshot, baseline_key)
    previous = baseline_snapshot.get(baseline_key)
    gate = gate_decision(
        gate_mode,
        signature=signature,
        previous=previous if isinstance(previous, dict) else None,
        diagnostics=diagnostics,
        no_match=no_match,
        matched_count=(
            len(candidates) + len(event_candidates) + len(episode_candidates) + path_matched_count
        ),
        selected_count=len(selected) + len(local_episode_selected),
        # The selection above already left out what this conversation holds.
        conversation=session is not None,
        delivery_profile=delivery_profile,
    )
    trace.funnel_boundary("gate_input", selected)
    withheld = gate["mode"] == "enforce" and gate["decision"] == "skip"
    trace.funnel_boundary("gate_withheld", selected if withheld else [])
    trace.exclusions(layer_excluded, "layer")
    if withheld:
        for item in selected:
            trace.observe(item, "gate", "withheld")
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
    projection = _capsule_projection(selected, local_episode_selected)
    groups, procedural, semantic, episodic = (projection[key] for key in ("categories", "procedural", "semantic", "episodic"))
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
            "progress": (task['progress'] if delivery_profile == 'cursor-rule' else
                         render_current_state(task["progress"], task.get("auto_checkpoint"))),
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
    result = _with_index_images(result, _selected_index_images(selected))
    if pack is not None:
        # The capsule as the host gets it: the limits of its JSON and of its
        # rendered text leave out what does not fit, and what they leave out
        # was not handed. Recorded as handed, it was suppressed on the next
        # turns as an item that "still applies" although the conversation
        # never saw it; so the conversation's record, the manifest and its
        # token estimates are written from what the capsule shows.
        trace.funnel_boundary("pack_input", selected)
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
        trace.funnel_boundary("pack_omitted", left_out)
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
    # Last point-in-time source custody before receipts. Packing callbacks can
    # race edits: remove all views and recompute hint-bearing gate provenance.
    final_config = load_config(repository)
    final_policy = _CurrentSourcePolicy(connection, repository)
    final_policy.prime(selected)
    working_current = task is None
    working_changed = False
    if task is not None:
        try:
            current_text = _read_final_working_record(task_path)
            current_task, current_body = parse_markdown_record(task_path, text=current_text)
            validate_record(current_task)
            eligible, _ = record_is_eligible(repository, current_task, final_config)
            working_current = (eligible and current_task["id"] == task["id"]
                and _working_record_image(current_task, current_body) == initial_working_image)
            # Still this task and still deliverable, only rewritten - by
            # another session's turn checkpoint, say - while this capsule
            # was assembled.
            working_changed = not working_current and eligible and current_task["id"] == task["id"]
        except (BrainError, OSError, UnicodeError, ValueError, TypeError):
            working_current = False
    working_withdrawn = task is not None and not working_current
    derived_withdrawn = working_withdrawn and query_source in ("task", "continuation")
    if working_withdrawn:
        result["working"] = None
        if working_changed:
            result["working_changed"] = True
    if derived_withdrawn:
        query, query_source = task_identifier, "task-id"
        result["query"] = query
        local_episode_selected = []
        # This also applies to an episode-only capsule: packed views are
        # delivery, even when there are no selected document rows to prune.
        result.update(procedural=[], semantic=[], episodic=[], selected=[],
                      categories={key: [] for key in result.get("categories", {})})
    result["query_source"] = query_source
    if selected:
        # Recheck only delivered rows, on copies and without reranking. A
        # second ranking pass must not halve already-admitted citation drift.
        expected_drift = {item["path"]: set(item.get("source_changed", [])) for item in selected}
        final_rows = [dict(item) for item in selected]
        for item in final_rows:
            item.pop("source_changed", None)
        current, custody_excluded = _runtime_filter(repository, final_rows, final_config, trace=trace,
            connection=connection, source_policy=final_policy, custody_only=True)
        current_paths = set()
        for item in current:
            if final_policy.missing_source(item):
                custody_excluded.append({"path": item["path"], "reason": "source-missing"})
            elif set(item.get("source_changed", [])) != expected_drift[item["path"]]:
                # Do not expand already-packed warnings or adopt new content.
                custody_excluded.append({"path": item["path"], "reason": "source-changed-during-pack"})
            else:
                current_paths.add(item["path"])
        dropped = {item["path"] for item in selected if item["path"] not in current_paths}
        if derived_withdrawn:
            dropped.update(item["path"] for item in selected)
        if dropped:
            filter_excluded.extend(custody_excluded)
            selected = [item for item in selected if item["path"] not in dropped]
            signature = _retrieval_signature(query,
                [*(item.get("_excerpt_identity", (item["path"], item["source_hash"])) for item in selected),
                 *(_episode_signature(episode) for episode in local_episode_selected)],
                task["revision"] if task is not None else 0)
            gate = gate_decision(gate_mode, signature=signature,
                previous=previous if isinstance(previous, dict) else None,
                diagnostics=diagnostics, no_match=no_match,
                matched_count=len(candidates) + len(event_candidates) + len(episode_candidates) + path_matched_count,
                selected_count=len(selected) + len(local_episode_selected), conversation=session is not None,
                delivery_profile=delivery_profile)
            withheld = gate["mode"] == "enforce" and gate["decision"] == "skip"
            if withheld:
                selected, local_episode_selected = [], []
            # Prune already packed views, never rerun callbacks or expand clipped
            # snippets. The native renderer runs from these views after return.
            allowed = {item["path"] for item in selected}
            for key in ("procedural", "semantic", "selected"):
                result[key] = [item for item in result.get(key, []) if item.get("path") in allowed]
            result["episodic"] = [item for item in result.get("episodic", [])
                                  if not withheld and (item.get("path") in allowed or item.get("id") in
                                      {episode.get("id") for episode in local_episode_selected})]
            result["categories"] = {key: [item for item in items if item.get("path") in allowed]
                                    for key, items in result.get("categories", {}).items()}
            result["gate"] = gate
    if (pack is not None or hint_candidates or derived_withdrawn) and not withheld:
        # A packed-out hint was never delivered either. Its proposed identity
        # must not poison the next turn's repeat gate or baseline receipt.
        signature = _retrieval_signature(query,
            [*(item.get("_excerpt_identity", (item["path"], item["source_hash"])) for item in selected),
             *(_episode_signature(episode) for episode in local_episode_selected)],
            task["revision"] if task is not None else 0)
        gate = gate_decision(gate_mode, signature=signature,
            previous=previous if isinstance(previous, dict) else None,
            diagnostics=diagnostics, no_match=no_match,
            matched_count=len(candidates) + len(event_candidates) + len(episode_candidates) + path_matched_count,
            selected_count=len(selected) + len(local_episode_selected), conversation=session is not None,
            delivery_profile=delivery_profile)
        withheld = gate["mode"] == "enforce" and gate["decision"] == "skip"
        result["gate"] = gate
        if withheld:
            selected, local_episode_selected = [], []
            result.update(procedural=[], semantic=[], episodic=[], selected=[],
                          categories={key: [] for key in result.get("categories", {})})
    if hint_stats is not None:
        hint_stats["delivered"] = sum(item.get("selection") == "source-hint" for item in selected)
    usage = _token_usage(selected, local_episode_selected)
    trace.funnel_boundary("selected_post_pack", selected)
    trace.exclusions((item for item in layer_excluded if item["reason"] == "capsule-limit"), "capsule")
    for item in selected:
        trace.observe(item, "delivery", "selected")
    result["token_estimates"] = usage
    # Rebind only from original final admitted rows, never callback attributes.
    # This also precedes the local-publication-failure early return.
    result = _with_index_images(result, _selected_index_images(selected))
    baseline_plan = None if withheld else {
        "key": baseline_key, "signature": signature, "expected": baseline_expected}
    session_plan = None
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
        window_receipts = {}
        for item in selected:
            if item["path"] not in delivery_keys or not trace._eligible(item):
                continue
            key, revision = delivery_keys[item["path"]]
            window_receipts[_window_key(key)] = {
                "revision": revision, "handed_turn": session_turn,
                "manifest_id": manifest_id, "doc_ref": _window_key(item["path"])}
        session_plan = _ordinary_session_plan(
            session,
            session_turn,
            {**still_handed, **handed_now},
            session_deliveries,
            transcript_position,
            reset=compacted,
            window_receipts=window_receipts,
            exact_next_turn=True,
        )
        # The expectation predates callbacks/packing, not the later projection.
        session_plan["expected"] = session_expected
    manifest = {
        "schema_version": MANIFEST_SCHEMA_VERSION,
        "id": manifest_id,
        "created_at": utc_now(),
        "query": query,
        "task_id": own_id,
        "task_revision": task["revision"] if task is not None else None,
        "local_episode_count": len(local_episode_selected),
        "filters": {
            "privacy": final_config["allowed_privacy"],
            "owners": final_config["owners"],
            "authority": final_config["allowed_authority"],
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
    if hint_stats is not None:
        manifest["source_hints"] = hint_stats
    validate_schema_file(
        repository, "retrieval-manifest.schema.json", manifest
    )
    # Revalidate caller-owned inferred identity after packing and schema work,
    # before either manifest path or delivery receipts can claim a publication.
    if admit_delivery is not None:
        admit_delivery()
    # A delivery receipt must not refer to a publication that then fails.
    # Governed provenance is mandatory; local diagnostics are best effort.
    if manifest_scope == 'governed':
        with mutation_lock(repository):
            atomic_json(manifest_path, manifest)
    else:
        try:
            atomic_json(manifest_path, manifest)
        except OSError:
            # The write might have taken effect before an error returned.
            # Never adopt/delete that possible file or claim absence/no-op.
            result['manifest'] = None
            result['manifest_publication'] = 'unconfirmed'
            trace.ordinary_receipt = unknown_ordinary_receipt()
            if source_provenance_sink is not None:
                proof = local_source_provenance(result, manifest)
                if proof is not None:
                    source_provenance_sink.update(proof)
            return result
        try:
            _prune_local_manifests(manifest_directory, local_manifest_retention(config))
        except OSError:
            # Publication was confirmed. A retention error cannot invalidate it.
            pass
    receipt_return = _commit_ordinary_receipts(connection, baseline=baseline_plan, session=session_plan)
    try:
        try:
            trace.ordinary_receipt = _observe_ordinary_receipt(
                receipt_return, baseline_plan is not None or session_plan is not None)
        except Exception:
            pass
    finally:
        # Drop the owner's sensitive latest-session projection immediately.
        del receipt_return
    _save_trace(repository, manifest_id, trace, config)
    return result
