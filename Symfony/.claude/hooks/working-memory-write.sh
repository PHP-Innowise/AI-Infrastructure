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
FLUSH_AFTER="${CONTEXT_FLUSH_AFTER:-5}"

command -v python3 > /dev/null 2>&1 || exit 0
[ -f "$CONTEXT_CLI" ] || exit 0

HOOK_STDIN=$(cat 2>/dev/null)
# Cursor also runs the Claude Code hooks it finds. Where this project has its
# own Cursor hooks they serve the session; this copy stands down.
case "$HOOK_STDIN" in
  *'"cursor_version"'*) [ -f "$ROOT_DIR/.cursor/hooks.json" ] && exit 0 ;;
esac

TASK_ID="${CONTEXT_TASK_ID:-$(git -C "$PROJECT_DIR" branch --show-current 2>/dev/null)}"
[ -n "$TASK_ID" ] || exit 0

if command -v timeout > /dev/null 2>&1; then
  timeout "$BUDGET_SECONDS" python3 "$CONTEXT_CLI" turn \
    --task-id "$TASK_ID" --flush-after "$FLUSH_AFTER" > /dev/null 2>&1
else
  python3 "$CONTEXT_CLI" turn \
    --task-id "$TASK_ID" --flush-after "$FLUSH_AFTER" > /dev/null 2>&1
fi

# Capsule delivery: this client receives the Task Capsule at prompt time
# through working-memory-read.sh, so the turn checkpoint above is all that
# runs here.

exit 0
