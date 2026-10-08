#!/bin/bash
# Working-Memory Read Hook
# Refreshes procedural, semantic, and episodic memory and emits a bounded Task
# Capsule for this request.
# Hook type: UserPromptSubmit
# Exit codes: always 0 (context tooling must never block a prompt)
#
# This is the read half of automatic memory. It writes no task state: at prompt
# time nothing has happened yet, so there is no delta to record. The write half
# runs on Stop; see working-memory-write.sh.
#
# The layer report is printed even when it fails. A request that silently reads
# a stale index is worse than one told which layer went stale.

set -u

# Output: plain text on stdout, which Claude Code and Codex add to the
# prompt as context.

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
# context.py, which also rejects anything that looks like a secret or
# personal data before the text can reach a query or a manifest. The
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
ARGUMENTS=(refresh --host codex)
if [ -n "$QUERY" ] && [ -n "$TASK_ID" ]; then
  # --sanitize: secrets, personal data and pasted transcript prefixes are cut
  # out of the prompt, instead of the whole turn going without memory.
  ARGUMENTS+=(--query "$QUERY" --task-id "$TASK_ID" --ephemeral --sanitize)
  [ -n "$SESSION_ID" ] && ARGUMENTS+=(--session-id "$SESSION_ID")
  [ -n "$SESSION_ID" ] && [ -n "$TRANSCRIPT" ] && ARGUMENTS+=(--transcript "$TRANSCRIPT")
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

[ -n "$REPORT" ] || exit 0

# The capsule names itself: its memory section says the text is reference
# data to check against the cited file, so no banner precedes it.
echo "$REPORT"

exit 0
