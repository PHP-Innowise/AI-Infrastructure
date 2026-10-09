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

# An internal draft-only follow-up must not create another memory turn.
[ "${CONTEXT_MEMORY_RECOVERY:-}" = "1" ] && exit 0

# A host that puts this turn's Task Capsule into the prompt itself sets
# CONTEXT_CAPSULE_DELIVERED=1; the Harness does, retrieved for the message
# alone. A second capsule here would be distilled from that whole prompt and
# spend the turn's memory budget twice. The write half still runs on Stop.
[ "${CONTEXT_CAPSULE_DELIVERED:-}" = "1" ] && exit 0

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

# Remove recognized personal-data spans in memory before constructing a query.
# The CLI still validates the safe result; secrets/raw conversations are refused.
# Keep the program in -c: a heredoc would occupy stdin and hide the prompt.
QUERY=$(python3 -c '
import json
import sys
sys.path.insert(0, sys.argv[1])
from automatic_query import sanitize_automatic_query

try:
    prompt = json.load(sys.stdin).get("prompt", "")
except (AttributeError, UnicodeDecodeError, ValueError):
    prompt = ""
if not isinstance(prompt, str):
    prompt = ""
print(sanitize_automatic_query(prompt))
' "$ROOT_DIR/memory-bank/scripts" 2>/dev/null)

# A refused/PII-only prompt must not fall back to the task goal or branch name.
[ -n "$QUERY" ] || exit 0

TASK_ID="${CONTEXT_TASK_ID:-$(git -C "$ROOT_DIR" branch --show-current 2>/dev/null)}"

# One process refreshes all three layers and assembles the capsule. Splitting
# them would index twice, because retrieval refreshes the index itself.
# --ephemeral keeps the per-request manifest, including the query text, in
# ignored local state instead of shared Git history.
ARGUMENTS=(refresh --host claude)
if [ -n "$QUERY" ] && [ -n "$TASK_ID" ]; then
  ARGUMENTS+=(--query "$QUERY" --task-id "$TASK_ID" --ephemeral)
fi

REPORT=$(run python3 "$CONTEXT_CLI" "${ARGUMENTS[@]}" 2>/dev/null)
HOOK_STATUS=$?

# Recorded before the early exit below, because the one case worth measuring
# is the one that produces no report: on a timeout `timeout` returns 124 and
# the Python process was killed before it could append anything itself, so
# the shell has to write this line or the turn leaves no trace at all. The
# record carries a status and nothing else — no query, no paths.
if [ -d "$ROOT_DIR/memory-bank/local" ] || mkdir -p "$ROOT_DIR/memory-bank/local" 2>/dev/null; then
  printf '{"at":"%s","hook_status":%s,"source":"hook"}\n' \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$HOOK_STATUS" \
    >> "$ROOT_DIR/memory-bank/local/refresh-health.ndjson" 2>/dev/null || true
fi

[ -n "$REPORT" ] || exit 0

echo "Memory refresh (retrieved context is not authoritative — verify the source)"
echo "=========================================================================="
echo "$REPORT"

exit 0
