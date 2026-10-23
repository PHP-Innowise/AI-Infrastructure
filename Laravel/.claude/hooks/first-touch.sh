#!/bin/bash
# Knowledge-at-First-Touch Hook
# Hands the agent, as it first reads or edits a file in a session, the memory
# that cites that file: Memory Bank chunks, findings and decisions whose
# sources name it, and the specs that name it.
# Hook type: PreToolUse (Read|Edit|Write|MultiEdit|NotebookEdit|Bash)
# Exit codes: always 0 (context tooling must never block a tool call)
#
# The prompt hook predicts what a turn needs from the prompt; this one waits
# for the file. Off unless `first_touch_delivery` is true in
# project-brain/config/runtime.json or CONTEXT_FIRST_TOUCH_DELIVERY=1: it is an
# experiment, and the repository takes a new delivery heuristic only on a
# task-outcome measurement (docs/MEMORY-EVAL.md). Switched off, the hook reads
# that one setting and stops; it starts no interpreter.
#
# Once per file per session, at most three documents and 1,200 characters a
# call, filtered as the Task Capsule is (privacy, secrets, freshness); a file
# nothing cites delivers nothing. The session's first call that needs it
# builds a baseline - the files the index has knowledge for - and a call that
# names none of them ends here with one grep.

set -u

# Output: hookSpecificOutput.additionalContext JSON, which Claude Code and
# Codex place beside the tool result. Plain stdout from a PreToolUse hook
# reaches neither model.

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

# An internal draft-only follow-up must not create another memory turn.
# A hook that leaves before reading its input breaks the host's write of a
# large call - a Write of a big file - which Claude Code then reports as a
# hook error (EPIPE). Every way out before the input is read drains it:
# `exec cat` reads it to the end in this process, no interpreter started.
[ "${CONTEXT_MEMORY_RECOVERY:-}" = "1" ] && exec cat > /dev/null

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

# The switch: the variable first, then runtime.json. The runtime reads the
# setting again and decides; this only keeps a project that left it off from
# paying for an interpreter on every tool call.
case "${CONTEXT_FIRST_TOUCH_DELIVERY:-}" in
  1|[Tt][Rr][Uu][Ee]) ;;
  '')
    grep -Eq '"first_touch_delivery"[[:space:]]*:[[:space:]]*true' \
      "$STATE_DIR/project-brain/config/runtime.json" 2> /dev/null || exec cat > /dev/null
    ;;
  *) exec cat > /dev/null ;;
esac

# context_cli.py runs context.py from its cached bytecode - run as a script,
# context.py is compiled again on every start. An older runtime has only
# context.py.
CONTEXT_CLI="$ROOT_DIR/memory-bank/scripts/context_cli.py"
[ -f "$CONTEXT_CLI" ] || CONTEXT_CLI="$ROOT_DIR/memory-bank/scripts/context.py"
BUDGET_SECONDS="${CONTEXT_FIRST_TOUCH_BUDGET:-3}"

# cat, not `read -d ''`: bash reads a pipe a byte at a time, and a Write
# payload carries the whole file - 150 ms a megabyte against 9.
HOOK_STDIN=$(cat)
# Cursor also runs the Claude Code hooks it finds. Where this project has its
# own Cursor hooks they serve the session; this copy stands down.
case "$HOOK_STDIN" in
  *'"cursor_version"'*) [ -f "$ROOT_DIR/.cursor/hooks.json" ] && exit 0 ;;
esac

# Once per file per session needs the session: Claude Code and Codex send
# session_id, Cursor conversation_id. A call without one delivers nothing.
SESSION_KEY=$(printf '%s' "$HOOK_STDIN" \
  | sed -n -E 's/.*"(session_id|conversation_id)"[[:space:]]*:[[:space:]]*"([^"]*)".*/\2/p' | head -1)
SESSION_KEY=${SESSION_KEY//[^A-Za-z0-9]/}
SESSION_KEY=${SESSION_KEY:0:64}
[ -n "$SESSION_KEY" ] || exit 0

# The session's state is ignored local state, never reached through a link.
SESSION_DIR="$STATE_DIR/memory-bank/local/first-touch/$SESSION_KEY"
for STATE_ENTRY in "$STATE_DIR/memory-bank" "$STATE_DIR/memory-bank/local" \
    "$STATE_DIR/memory-bank/local/first-touch" "$SESSION_DIR"; do
  [ -L "$STATE_ENTRY" ] && exit 0
done
[ -e "$SESSION_DIR/off" ] && exit 0
BASELINE="$SESSION_DIR/cited"
if [ -f "$BASELINE" ] && [ ! -L "$BASELINE" ]; then
  # Nothing cited is left untouched, or this call names none of it. Paths are
  # matched as text anywhere in the call: a false match costs one runtime
  # start, which then decides exactly.
  [ -s "$BASELINE" ] || exit 0
  printf '%s' "$HOOK_STDIN" | grep -F -q -f "$BASELINE" 2> /dev/null || exit 0
fi

resolve_python || exit 0
[ -f "$CONTEXT_CLI" ] || exit 0

run() {
  if command -v timeout > /dev/null 2>&1; then
    timeout "$BUDGET_SECONDS" "$@"
  else
    "$@"
  fi
}

REPORT=$(printf '%s' "$HOOK_STDIN" | run "${PYTHON[@]}" "$CONTEXT_CLI" first-touch \
  --host claude --session "$SESSION_KEY" 2> /dev/null)
HOOK_STATUS=$?
if [ "$HOOK_STATUS" -eq 124 ]; then
  # A runtime past its budget would hold up every call that names a cited
  # file: the session stops delivering instead.
  mkdir -p "$SESSION_DIR" 2> /dev/null && [ ! -L "$SESSION_DIR" ] && [ ! -L "$SESSION_DIR/off" ] \
    && : > "$SESSION_DIR/off" 2> /dev/null
  exit 0
fi
[ "$HOOK_STATUS" -eq 0 ] && [ -n "$REPORT" ] && printf '%s\n' "$REPORT"
exit 0
