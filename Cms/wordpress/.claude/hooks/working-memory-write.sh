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

# An internal draft-only follow-up must not create another memory turn.
[ "${CONTEXT_MEMORY_RECOVERY:-}" = "1" ] && exit 0

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
# context_cli.py runs context.py from its cached bytecode - run as a script,
# context.py is compiled again on every start. An older runtime has only
# context.py.
CONTEXT_CLI="$ROOT_DIR/memory-bank/scripts/context_cli.py"
[ -f "$CONTEXT_CLI" ] || CONTEXT_CLI="$ROOT_DIR/memory-bank/scripts/context.py"
BUDGET_SECONDS="${CONTEXT_HOOK_BUDGET:-5}"
FLUSH_AFTER="${CONTEXT_FLUSH_AFTER:-5}"

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
[ -f "$CONTEXT_CLI" ] || exit 0

HOOK_STDIN=$(cat 2>/dev/null)
# Cursor also runs the Claude Code hooks it finds. Where this project has its
# own Cursor hooks they serve the session; this copy stands down.
case "$HOOK_STDIN" in
  *'"cursor_version"'*) [ -f "$ROOT_DIR/.cursor/hooks.json" ] && exit 0 ;;
esac

rebased_branch() {
  local directory state name
  directory=$(git -C "$PROJECT_DIR" rev-parse --absolute-git-dir 2>/dev/null) || return 1
  for state in rebase-merge rebase-apply; do
    [ -f "$directory/$state/head-name" ] || continue
    name=
    IFS= read -r name < "$directory/$state/head-name" || [ -n "$name" ] || continue
    case "$name" in refs/heads/?*) printf '%s' "${name#refs/heads/}"; return 0 ;; esac
  done
  return 1
}
TASK_ID="${CONTEXT_TASK_ID:-$(git -C "$PROJECT_DIR" branch --show-current 2>/dev/null)}"
[ -n "$TASK_ID" ] || TASK_ID=$(rebased_branch)
[ -n "$TASK_ID" ] || exit 0

# A bookkeeping continuation must not checkpoint the same turn again. The
# transcript the host names gives the files this turn read or changed, for the
# next session's "recent files"; the runtime keeps their paths only.
HOOK_FIELDS=$(printf '%s' "$HOOK_STDIN" | "${PYTHON[@]}" -c '
import json, os, sys
try:
    payload = json.load(sys.stdin)
except (ValueError, UnicodeError):
    payload = {}
if not isinstance(payload, dict):
    payload = {}
active = (type(payload.get("loop_count")) is int and payload["loop_count"] > 0
          if sys.argv[-1] == "cursor" else payload.get("stop_hook_active") is True)
transcript = payload.get("transcript_path", "")
if not isinstance(transcript, str) or len(transcript) > 4096 or "\n" in transcript or not os.path.isabs(transcript):
    transcript = ""
sys.stdout.write(("1" if active else "0") + "\n" + transcript + "\n.")
' --host claude 2>/dev/null)
MEMORY_RECOVERY=${HOOK_FIELDS%%$'\n'*}
TRANSCRIPT=
case "$HOOK_FIELDS" in
  *$'\n'*$'\n'*) TRANSCRIPT=${HOOK_FIELDS#*$'\n'}; TRANSCRIPT=${TRANSCRIPT%%$'\n'*} ;;
esac
TURN_ARGUMENTS=(turn --task-id "$TASK_ID" --flush-after "$FLUSH_AFTER")
[ -n "$TRANSCRIPT" ] && [ -f "$TRANSCRIPT" ] && TURN_ARGUMENTS+=(--transcript "$TRANSCRIPT")

if [ "$MEMORY_RECOVERY" != "1" ]; then
if command -v timeout > /dev/null 2>&1; then
  timeout "$BUDGET_SECONDS" "${PYTHON[@]}" "$CONTEXT_CLI" "${TURN_ARGUMENTS[@]}" > /dev/null 2>&1
else
  "${PYTHON[@]}" "$CONTEXT_CLI" "${TURN_ARGUMENTS[@]}" > /dev/null 2>&1
fi
fi

# The payload travels on stdin; only a metadata challenge is kept. A supported
# Stop requests one structured save from the same agent, never a new model.
if [ "${CONTEXT_CAPSULE_DELIVERED:-}" != "1" ] && [ -f "$ROOT_DIR/memory-bank/scripts/native_results.py" ]; then
  if command -v timeout > /dev/null 2>&1; then
    NATIVE_RESPONSE=$(printf '%s' "$HOOK_STDIN" | timeout "$BUDGET_SECONDS" "${PYTHON[@]}" "$CONTEXT_CLI" native-stop \
      --host claude --task-id "$TASK_ID" 2>/dev/null)
  else
    NATIVE_RESPONSE=$(printf '%s' "$HOOK_STDIN" | "${PYTHON[@]}" "$CONTEXT_CLI" native-stop \
      --host claude --task-id "$TASK_ID" 2>/dev/null)
  fi
  NATIVE_STATUS=$?
  [ "$NATIVE_STATUS" -eq 0 ] && [ -n "$NATIVE_RESPONSE" ] && printf '%s\n' "$NATIVE_RESPONSE"
fi

# Capsule delivery: this client receives the Task Capsule at prompt time
# through working-memory-read.sh, so the turn checkpoint above is all that
# runs here.

exit 0
