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

# Python 3.9+ by whichever name this shell knows it. Native Windows (Git Bash)
# often has no working python3: the python.org installer provides python and
# py, and the python3 found there may be the Microsoft Store stub, which only
# offers to install Python. Elsewhere python3 is taken without a probe.
resolve_python() {
  PYTHON=()
  if [ "${OS:-}" != "Windows_NT" ] && command -v python3 > /dev/null 2>&1; then
    PYTHON=(python3)
    return 0
  fi
  # On Windows the name found is remembered per user, and taken again while
  # the shell finds it at the same path (the hash table, no process): every
  # hook otherwise started an interpreter just to ask its version.
  local name path remembered="${TMPDIR:-/tmp}/ai-accelerator-python-${UID:-0}"
  if [ "${OS:-}" = "Windows_NT" ] && [ -f "$remembered" ] && [ ! -L "$remembered" ] \
      && IFS=$'\t' read -r name path < "$remembered" && [ -n "$path" ]; then
    case "$name" in
      python|py|python3)
        if hash "$name" 2> /dev/null && [ "${BASH_CMDS[$name]:-}" = "$path" ]; then
          PYTHON=("$name")
          [ "$name" = py ] && PYTHON+=(-3)
          return 0
        fi
        ;;
    esac
  fi
  for name in python py python3; do
    PYTHON=("$name")
    [ "$name" = py ] && PYTHON+=(-3)
    if "${PYTHON[@]}" -I -S -c 'import sys; sys.exit(sys.version_info < (3, 9))' > /dev/null 2>&1; then
      if [ "${OS:-}" = "Windows_NT" ] && [ ! -L "$remembered" ] && hash "$name" 2> /dev/null; then
        printf '%s\t%s\n' "$name" "${BASH_CMDS[$name]:-}" > "$remembered" 2> /dev/null
      fi
      return 0
    fi
  done
  PYTHON=()
  return 1
}
resolve_python || exit 0
[ -f "$RUNTIME" ] || exit 0

HOOK_STDIN=$(cat 2>/dev/null)
# Cursor also runs the Claude Code hooks it finds. Where this project has its
# own Cursor hooks they serve the session; this copy stands down.
case "$HOOK_STDIN" in
  *'"cursor_version"'*) [ -f "$ROOT_DIR/.cursor/hooks.json" ] && exit 0 ;;
esac

ARGUMENTS=(--root "$STATE_DIR" --host claude --event hook)
if command -v timeout > /dev/null 2>&1; then
  printf '%s' "$HOOK_STDIN" | timeout "$BUDGET_SECONDS" "${PYTHON[@]}" "$RUNTIME" "${ARGUMENTS[@]}" 2>/dev/null
else
  printf '%s' "$HOOK_STDIN" | "${PYTHON[@]}" "$RUNTIME" "${ARGUMENTS[@]}" 2>/dev/null
fi

exit 0
