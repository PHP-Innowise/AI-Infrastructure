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
# a stale index is worse than one told which layer went stale. The same applies
# one level up: a refresh that never ran must say so. An empty hook and a
# crashed one look identical from inside the turn, and the difference decides
# whether working memory may be treated as consulted at all.

set -u

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
CONTEXT_CLI="$ROOT_DIR/memory-bank/scripts/context.py"
BUDGET_SECONDS="${CONTEXT_HOOK_BUDGET:-5}"

command -v python3 > /dev/null 2>&1 || exit 0
[ -f "$CONTEXT_CLI" ] || exit 0

run() {
  if command -v timeout > /dev/null 2>&1; then
    timeout "$BUDGET_SECONDS" "$@"
  else
    "$@"
  fi
}

# The prompt arrives as JSON on stdin. Reduce it to a bounded, quoteless query;
# the CLI still rejects anything that looks like a secret or personal data.
# Keep the program in -c: a heredoc would occupy stdin and hide the prompt.
QUERY=$(python3 -c '
import json
import re
import sys

try:
    prompt = json.load(sys.stdin).get("prompt", "")
except (AttributeError, UnicodeDecodeError, ValueError):
    prompt = ""
if not isinstance(prompt, str):
    prompt = ""
print(" ".join(re.findall(r"\w+", prompt, flags=re.UNICODE)[:24]))
' 2>/dev/null)

TASK_ID="${CONTEXT_TASK_ID:-$(git -C "$ROOT_DIR" branch --show-current 2>/dev/null)}"

# One process refreshes all three layers and assembles the capsule. Splitting
# them would index twice, because retrieval refreshes the index itself.
# --ephemeral keeps the per-request manifest, including the query text, in
# ignored local state instead of shared Git history.
ARGUMENTS=(refresh)
if [ -n "$QUERY" ] && [ -n "$TASK_ID" ]; then
  ARGUMENTS+=(--query "$QUERY" --task-id "$TASK_ID" --ephemeral)
fi

ERROR_FILE="${TMPDIR:-/tmp}/working-memory-read-$$.err"
REPORT=$(run python3 "$CONTEXT_CLI" "${ARGUMENTS[@]}" 2>"$ERROR_FILE")
STATUS=$?
# One bounded line: enough to name the failure, never enough to paste a trace
# or anything the CLI refused to accept into the prompt.
DETAIL=$(tr '\n\t' '  ' < "$ERROR_FILE" 2>/dev/null | cut -c1-160)
rm -f "$ERROR_FILE" 2>/dev/null

echo "Memory refresh (retrieved context is not authoritative — verify the source)"
echo "=========================================================================="

if [ -n "$REPORT" ]; then
  echo "$REPORT"
  exit 0
fi

# Nothing to report means the refresh did not complete. Say which, and say
# what follows from it, instead of leaving the turn to assume memory was read.
if [ "$STATUS" -eq 124 ]; then
  echo "unavailable: refresh exceeded its ${BUDGET_SECONDS}s budget"
else
  echo "unavailable: refresh exited $STATUS${DETAIL:+ — $DETAIL}"
fi
echo "Working memory was NOT consulted this turn. Read the canonical sources directly."

exit 0
