#!/bin/bash
# Working-Memory Read Hook
# Refreshes procedural, semantic, and episodic memory and emits a bounded Task
# Capsule for this request.
# Cursor hook event: beforeSubmitPrompt
# Exit codes: always 0 (context tooling must never block a prompt)
#
# This is the read half of automatic memory. It writes no task state: at prompt
# time nothing has happened yet, so there is no delta to record. The write half
# runs on Stop; see working-memory-write.sh.
#
# The layer report is printed even when it fails. A request that silently reads
# a stale index is worse than one told which layer went stale.

set -u

# Output: Cursor reads this hook's stdout as JSON and holds the prompt until
# the hook answers, so every exit path answers "continue" - memory never
# blocks a prompt. The capsule itself goes into the alwaysApply rule below.
trap 'printf "{\"continue\": true}\n"' EXIT
# An internal draft-only follow-up must not create another memory turn.
[ "${CONTEXT_MEMORY_RECOVERY:-}" = "1" ] && exit 0

# A host that puts this turn's Task Capsule into the prompt itself sets
# CONTEXT_CAPSULE_DELIVERED=1; the Harness does, retrieved for the message
# alone. A second capsule here would be distilled from that whole prompt and
# spend the turn's memory budget twice. The write half still runs on Stop.
[ "${CONTEXT_CAPSULE_DELIVERED:-}" = "1" ] && exit 0

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
# Installed, the accelerator, the project and its state are all ROOT_DIR.
# Attached - a launcher lends this clone's edition to a project and names it
# in ACCELERATOR_HOME - the project and the state directory are the
# launcher's, so nothing is written into the clone or the project.
PROJECT_DIR=$ROOT_DIR
STATE_DIR=$ROOT_DIR
accelerator_absolute() { case "$1" in /*|[A-Za-z]:[\\/]*) return 0 ;; esac; return 1; }
if accelerator_absolute "${ACCELERATOR_STATE_DIR:-}" && accelerator_absolute "${ACCELERATOR_PROJECT_DIR:-}" \
  && [ "$(cd "${ACCELERATOR_HOME:-/nonexistent}" 2>/dev/null && pwd -P)" = "$(cd "$ROOT_DIR" && pwd -P)" ]; then
  PROJECT_DIR=$ACCELERATOR_PROJECT_DIR
  STATE_DIR=$ACCELERATOR_STATE_DIR
fi
CONTEXT_CLI="$ROOT_DIR/memory-bank/scripts/context.py"
BUDGET_SECONDS="${CONTEXT_HOOK_BUDGET:-5}"

command -v python3 > /dev/null 2>&1 || exit 0
[ -f "$CONTEXT_CLI" ] || exit 0

HOOK_STDIN=$(cat)

run() {
  if command -v timeout > /dev/null 2>&1; then
    timeout "$BUDGET_SECONDS" "$@"
  else
    "$@"
  fi
}

# The prompt arrives as JSON on stdin and is passed to the CLI as-is: query
# distillation (informative terms ranked by rarity in the index) lives in
# context.py, and --sanitize there cuts secrets, personal data and pasted
# transcript prefixes out of the prompt before any of it can reach a query or
# a manifest; a prompt with nothing left to search gets no capsule. The
# conversation's session id travels with it, so what this conversation was
# handed in its last few turns is not handed again, and so does the path of
# its transcript, where a compaction since the last turn shows.
# Keep the program in -c: a heredoc would occupy stdin and hide the prompt.
# Output: the session id, the transcript path, the prompt, one per line, and
# a final "\n." that keeps command substitution from eating the prompt's own
# trailing newlines.
HOOK_INPUT=$(printf '%s' "$HOOK_STDIN" | python3 -c '
import json
import os
import re
import sys

try:
    data = json.load(sys.stdin)
except (UnicodeDecodeError, ValueError):
    data = {}
if not isinstance(data, dict):
    data = {}
prompt = data.get("prompt", "")
if not isinstance(prompt, str):
    prompt = ""
session = data.get("session_id", "")
if not isinstance(session, str) or not re.fullmatch(r"[A-Za-z0-9._:-]{1,128}", session):
    session = ""
transcript = data.get("transcript_path", "")
if not isinstance(transcript, str) or len(transcript) > 4096 or "\n" in transcript or not os.path.isabs(transcript):
    transcript = ""
sys.stdout.write(session + "\n" + transcript + "\n" + prompt + "\n.")
' 2>/dev/null)
case "$HOOK_INPUT" in *$'\n'*$'\n'*) ;; *) HOOK_INPUT=$'\n\n\n.' ;; esac
SESSION_ID=${HOOK_INPUT%%$'\n'*}
HOOK_REST=${HOOK_INPUT#*$'\n'}
TRANSCRIPT=${HOOK_REST%%$'\n'*}
QUERY=${HOOK_REST#*$'\n'}
QUERY=${QUERY%$'\n.'}

TASK_ID="${CONTEXT_TASK_ID:-$(git -C "$PROJECT_DIR" branch --show-current 2>/dev/null)}"

# One process refreshes all three layers and assembles the capsule. Splitting
# them would index twice, because retrieval refreshes the index itself.
# --ephemeral keeps the per-request manifest, including the query text, in
# ignored local state instead of shared Git history.
ARGUMENTS=(refresh --host cursor)
if [ -n "$QUERY" ] && [ -n "$TASK_ID" ]; then
  # --sanitize: secrets, personal data and pasted transcript prefixes are cut
  # out of the prompt, instead of the whole turn going without memory.
  ARGUMENTS+=(--query "$QUERY" --task-id "$TASK_ID" --ephemeral --sanitize)
  # The rule is re-sent whole with every request, so nothing may be left out
  # of it as "handed earlier": no --session-id. JSON, for capsule_text.
  ARGUMENTS+=(--json)
fi

REPORT=$(run python3 "$CONTEXT_CLI" "${ARGUMENTS[@]}" 2>/dev/null)
HOOK_STATUS=$?

# Recorded before the early exit below, because the one case worth measuring
# is the one that produces no report: on a timeout `timeout` returns 124 and
# the Python process was killed before it could append anything itself, so
# the shell has to write this line or the turn leaves no trace at all. The
# record carries a status and nothing else — no query, no paths.
if [ -d "$STATE_DIR/memory-bank/local" ] || mkdir -p "$STATE_DIR/memory-bank/local" 2>/dev/null; then
  printf '{"at":"%s","hook_status":%s,"source":"hook"}\n' \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$HOOK_STATUS" \
    >> "$STATE_DIR/memory-bank/local/refresh-health.ndjson" 2>/dev/null || true
fi

# Capsule delivery: the rule is rendered from this prompt before the request
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

exit 0
