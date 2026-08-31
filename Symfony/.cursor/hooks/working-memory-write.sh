#!/bin/bash
# Working-Memory Write Hook
# Buffers this turn's change set and flushes it to the authoritative task on a
# boundary.
# Hook type: Stop
# Exit codes: always 0 (a failed checkpoint must never surface as a turn error)
#
# This is the write half of automatic memory. It runs at the end of a turn,
# where a real delta exists, rather than at prompt time. Turns are buffered in
# ignored local state and flushed together so that continuity does not cost one
# governed revision and one rewritten handoff per turn.
#
# Only Git porcelain metadata is read. Working-tree contents never reach the
# buffer, and paths that look sensitive are excluded and reported by the CLI.

set -u

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
CONTEXT_CLI="$ROOT_DIR/memory-bank/scripts/context.py"
BUDGET_SECONDS="${CONTEXT_HOOK_BUDGET:-5}"
FLUSH_AFTER="${CONTEXT_FLUSH_AFTER:-5}"

command -v python3 > /dev/null 2>&1 || exit 0
[ -f "$CONTEXT_CLI" ] || exit 0

TASK_ID="${CONTEXT_TASK_ID:-$(git -C "$ROOT_DIR" branch --show-current 2>/dev/null)}"
[ -n "$TASK_ID" ] || exit 0

if command -v timeout > /dev/null 2>&1; then
  timeout "$BUDGET_SECONDS" python3 "$CONTEXT_CLI" turn \
    --task-id "$TASK_ID" --flush-after "$FLUSH_AFTER" > /dev/null 2>&1
else
  python3 "$CONTEXT_CLI" turn \
    --task-id "$TASK_ID" --flush-after "$FLUSH_AFTER" > /dev/null 2>&1
fi

# Capsule delivery: Cursor has no UserPromptSubmit-equivalent event, so
# working-memory-read.sh is not shipped in .cursor/hooks. The read path is
# served here instead: after the turn checkpoint, the freshest Task Capsule
# is rendered into an alwaysApply Cursor rule, which Cursor attaches to
# every prompt of the next turn. The file is ignored local state, one turn
# stale by design, and replaced only when a fresh render succeeds.
RULES_DIR="$ROOT_DIR/.cursor/rules"
RULE_FILE="$RULES_DIR/working-memory.mdc"
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

exit 0
