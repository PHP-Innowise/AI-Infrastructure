#!/bin/bash
# Context Continuity Hook
# Keeps this chat's visible text for an explicit chat merge, and hands a new
# task the merge prepared for it.
# Hook type: session start, prompt submission and end of turn - one script;
# the event name in the client's payload selects the action
# Exit codes: always 0 (continuity must never block or fail a turn)
#
# On a prompt or a final answer the documented visible text is appended to a
# per-chat snapshot under the ignored .context-handoff/ directory. At session
# start a merge prepared with `context-load merge` is delivered to the new
# task through the client's native context channel. Nothing else is restored:
# carrying a branch's work into the next session is the Task Capsule's job
# (working-memory-read.sh). Nothing here writes Project Brain, Memory Bank or
# the local index. CONTEXT_CONTINUITY_DISABLED=1 turns both halves off.

set -u

# An internal draft-only follow-up is not part of the conversation.
[ "${CONTEXT_MEMORY_RECOVERY:-}" = "1" ] && exit 0

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd) || exit 0
# Installed, the accelerator, the project and its state are all ROOT_DIR.
# Attached - a launcher lends this clone's edition to a project and names it
# in ACCELERATOR_HOME - the snapshots live in the launcher's state directory,
# so nothing is written into the clone or the project.
STATE_DIR=$ROOT_DIR
accelerator_absolute() { case "$1" in /*|[A-Za-z]:[\\/]*) return 0 ;; esac; return 1; }
if accelerator_absolute "${ACCELERATOR_STATE_DIR:-}" && accelerator_absolute "${ACCELERATOR_PROJECT_DIR:-}" \
  && [ "$(cd "${ACCELERATOR_HOME:-/nonexistent}" 2>/dev/null && pwd -P)" = "$(cd "$ROOT_DIR" && pwd -P)" ]; then
  STATE_DIR=$ACCELERATOR_STATE_DIR
fi
RUNTIME="$ROOT_DIR/memory-bank/scripts/context_continuity.py"
BUDGET_SECONDS="${CONTEXT_HOOK_BUDGET:-5}"

command -v python3 > /dev/null 2>&1 || exit 0
[ -f "$RUNTIME" ] || exit 0

HOOK_STDIN=$(cat 2>/dev/null)

ARGUMENTS=(--root "$STATE_DIR" --host codex --event hook)
if command -v timeout > /dev/null 2>&1; then
  printf '%s' "$HOOK_STDIN" | timeout "$BUDGET_SECONDS" python3 "$RUNTIME" "${ARGUMENTS[@]}" 2>/dev/null
else
  printf '%s' "$HOOK_STDIN" | python3 "$RUNTIME" "${ARGUMENTS[@]}" 2>/dev/null
fi

exit 0
