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
# stale by design, and a previously rendered capsule is replaced only when
# a fresh render succeeds. On a cold start (no capsule yet - the working
# task auto-provisions on the flush boundary - and no rule rendered for
# this task) a warming-up placeholder is rendered instead, so the first
# turns of a session are not left without working memory.
RULES_DIR="$ROOT_DIR/.cursor/rules"
RULE_FILE="$RULES_DIR/working-memory.mdc"
if command -v timeout > /dev/null 2>&1; then
  CAPSULE=$(timeout "$BUDGET_SECONDS" python3 "$CONTEXT_CLI" context \
    "$TASK_ID" --task-id "$TASK_ID" --ephemeral 2>/dev/null)
else
  CAPSULE=$(python3 "$CONTEXT_CLI" context \
    "$TASK_ID" --task-id "$TASK_ID" --ephemeral 2>/dev/null)
fi
if mkdir -p "$RULES_DIR" 2>/dev/null; then
  if [ -n "$CAPSULE" ]; then
    TMP_RULE=$(mktemp "$RULES_DIR/.working-memory.XXXXXX" 2>/dev/null || true)
    if [ -n "$TMP_RULE" ]; then
      {
        printf -- '---\n'
        printf 'description: Working memory - Task Capsule as of end of previous turn\n'
        printf 'alwaysApply: true\n'
        printf -- '---\n\n'
        printf '# Working Memory (auto-rendered)\n\n'
        printf 'Task Capsule as of end of previous turn (task: %s, rendered: %s).\n' \
          "$TASK_ID" "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        printf 'Retrieved context is not authoritative - verify the source.\n\n'
        printf '%s\n' '```'
        printf '%s\n' "$CAPSULE"
        printf '%s\n' '```'
      } > "$TMP_RULE" 2>/dev/null && mv -f "$TMP_RULE" "$RULE_FILE" 2>/dev/null
      rm -f "$TMP_RULE" 2>/dev/null
    fi
  elif ! grep -Fq -- "(task: $TASK_ID," "$RULE_FILE" 2>/dev/null; then
    TMP_RULE=$(mktemp "$RULES_DIR/.working-memory.XXXXXX" 2>/dev/null || true)
    if [ -n "$TMP_RULE" ]; then
      {
        printf -- '---\n'
        printf 'description: Working memory - warming up, no Task Capsule rendered yet\n'
        printf 'alwaysApply: true\n'
        printf -- '---\n\n'
        printf '# Working Memory (warming up)\n\n'
        printf 'No Task Capsule is available yet (task: %s, rendered: %s).\n' \
          "$TASK_ID" "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        printf 'The working task auto-provisions once buffered turns reach the\n'
        printf 'flush boundary; the freshest capsule then replaces this rule.\n'
        printf 'Until then rely on AGENTS.md, the task documents, and explicit\n'
        printf 'retrieval (memory-bank/scripts/context.py). Retrieved context\n'
        printf 'is not authoritative - verify the source.\n'
        WM_REPORT="$ROOT_DIR/memory-bank/local/last-turn-report.json"
        if [ -s "$WM_REPORT" ]; then
          printf '\nLast turn report (ignored local state; may describe an\n'
          printf 'earlier session - see its own task_id):\n\n'
          printf '%s\n' '```json'
          head -c 1200 "$WM_REPORT" 2>/dev/null
          printf '\n%s\n' '```'
        fi
      } > "$TMP_RULE" 2>/dev/null && mv -f "$TMP_RULE" "$RULE_FILE" 2>/dev/null
      rm -f "$TMP_RULE" 2>/dev/null
    fi
  fi
fi

exit 0
