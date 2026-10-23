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
cwd = payload.get("cwd", "")
if not isinstance(cwd, str) or len(cwd) > 4096 or "\n" in cwd or "\0" in cwd or not os.path.isabs(cwd):
    cwd = ""
sys.stdout.write(("1" if active else "0") + "\n" + transcript + "\n" + cwd + "\n.")
' --host cursor 2>/dev/null)
MEMORY_RECOVERY=${HOOK_FIELDS%%$'\n'*}
TRANSCRIPT=
SESSION_DIR=
case "$HOOK_FIELDS" in
  *$'\n'*$'\n'*$'\n'*)
    TRANSCRIPT=${HOOK_FIELDS#*$'\n'}
    SESSION_DIR=${TRANSCRIPT#*$'\n'}; SESSION_DIR=${SESSION_DIR%%$'\n'*}
    TRANSCRIPT=${TRANSCRIPT%%$'\n'*} ;;
esac

# A Claude Code desktop session in its own worktree - and any session after
# EnterWorktree - runs the hooks of the checkout they are installed in and
# names its own checkout in the input's "cwd". This checkout's task is its
# own branch's: a turn recorded from another checkout of the same repository
# would land in a task that session never worked on, so none is recorded. A
# subdirectory of this checkout, or a directory of another repository,
# changes nothing.
checkout_identity() {
  local directory=$1 output top common
  output=$(git -C "$directory" rev-parse --show-toplevel --git-common-dir 2>/dev/null) || return 1
  case "$output" in *$'\n'*) ;; *) return 1 ;; esac
  top=${output%%$'\n'*}
  common=${output#*$'\n'}
  case "$common" in /*|[A-Za-z]:[\\/]*) ;; *) common="$directory/$common" ;; esac
  top=$(cd -- "$top" 2>/dev/null && pwd -P) || return 1
  common=$(cd -- "$common" 2>/dev/null && pwd -P) || return 1
  printf '%s\n%s' "$top" "$common"
}
in_other_worktree() {
  local session=$1 mine theirs
  [ -n "$session" ] && [ "$session" != "$PROJECT_DIR" ] && [ -d "$session" ] || return 1
  theirs=$(checkout_identity "$session") || return 1
  mine=$(checkout_identity "$PROJECT_DIR") || return 1
  [ "${theirs%%$'\n'*}" != "${mine%%$'\n'*}" ] && [ "${theirs#*$'\n'}" = "${mine#*$'\n'}" ]
}
in_other_worktree "$SESSION_DIR" && exit 0

TURN_ARGUMENTS=(turn --task-id "$TASK_ID" --flush-after "$FLUSH_AFTER")
[ -n "$TRANSCRIPT" ] && [ -f "$TRANSCRIPT" ] && TURN_ARGUMENTS+=(--transcript "$TRANSCRIPT")

if [ "$MEMORY_RECOVERY" != "1" ]; then
if command -v timeout > /dev/null 2>&1; then
  timeout "$BUDGET_SECONDS" "${PYTHON[@]}" "$CONTEXT_CLI" "${TURN_ARGUMENTS[@]}" > /dev/null 2>&1
else
  "${PYTHON[@]}" "$CONTEXT_CLI" "${TURN_ARGUMENTS[@]}" > /dev/null 2>&1
fi
fi

# One line per Stop in the native result journal, beside refresh-health.ndjson:
# the save request's exit status. The shell writes it because the case worth
# counting is the one the runtime never finishes - killed at its budget (124)
# or failed before its own line. A status and nothing else: no payload, no
# paths, no task or session. As the prompt hook's health line, never through
# a link in an attached state, and never through a linked journal file.
NATIVE_HOST=(--host cursor)
native_journal() {
  local directory="$STATE_DIR/memory-bank/local" entry entries
  entries=("$directory/native-results.ndjson")
  [ "$STATE_DIR" = "$ROOT_DIR" ] || entries+=("${STATE_DIR%/}" "$STATE_DIR/memory-bank" "$directory")
  for entry in "${entries[@]}"; do
    [ -L "$entry" ] && return 0
  done
  { [ -d "$directory" ] || mkdir -p "$directory" 2>/dev/null; } || return 0
  # One physical line: the Cursor render guards admit a clock only on the
  # line that writes the journal.
  # 2>/dev/null first: a journal it cannot open stays silent (see the health line).
  printf '{"at":"%s","host":"%s","stage":"stop-hook",%s}\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${NATIVE_HOST[1]}" "$1" 2>/dev/null >> "$directory/native-results.ndjson" || true
}

# The payload travels on stdin; only a metadata challenge is kept. A supported
# Stop requests one structured save from the same agent, never a new model.
if [ "${CONTEXT_CAPSULE_DELIVERED:-}" != "1" ] && [ -f "$ROOT_DIR/memory-bank/scripts/native_results.py" ]; then
  if command -v timeout > /dev/null 2>&1; then
    NATIVE_RESPONSE=$(printf '%s' "$HOOK_STDIN" | timeout "$BUDGET_SECONDS" "${PYTHON[@]}" "$CONTEXT_CLI" native-stop \
      --host cursor --task-id "$TASK_ID" 2>/dev/null)
  else
    NATIVE_RESPONSE=$(printf '%s' "$HOOK_STDIN" | "${PYTHON[@]}" "$CONTEXT_CLI" native-stop \
      --host cursor --task-id "$TASK_ID" 2>/dev/null)
  fi
  NATIVE_STATUS=$?
  [ "$NATIVE_STATUS" -eq 0 ] && [ -n "$NATIVE_RESPONSE" ] && printf '%s\n' "$NATIVE_RESPONSE"
  native_journal '"event":"exited","status":'"$NATIVE_STATUS"
elif [ -f "$ROOT_DIR/memory-bank/scripts/native_results.py" ]; then
  # A host that delivers the capsule itself (the Harness) is asked for no
  # native save; the line keeps those turns from reading as no Stop at all.
  native_journal '"event":"skipped","code":"capsule-delivered"'
fi

# Cursor keeps working-memory.mdc, a generated alwaysApply rule, for direct-host delivery.
# Attached rules belong to the shared clone and are never maintained here.
[ "$STATE_DIR" = "$ROOT_DIR" ] || exit 0
cursor_rule_run() {
  local CURSOR_MAINTENANCE_STATUS=0 CURSOR_MAINTENANCE_REPORT CURSOR_PROJECTION_STATUS=0
  if command -v timeout >/dev/null 2>&1; then
    CURSOR_MAINTENANCE_REPORT=$(timeout 3 "${PYTHON[@]}" "$CONTEXT_CLI" "$@" --diagnostic 2>/dev/null) || CURSOR_MAINTENANCE_STATUS=$?
  else
    CURSOR_MAINTENANCE_REPORT=$("${PYTHON[@]}" "$CONTEXT_CLI" "$@" --diagnostic 2>/dev/null) || CURSOR_MAINTENANCE_STATUS=$?
  fi
  if [ "$CURSOR_MAINTENANCE_STATUS" -ne 0 ]; then
    printf '%s\n' 'Cursor memory rule was not updated: maintenance_unavailable.' >&2
    return "$CURSOR_MAINTENANCE_STATUS"
  fi
  # 4096 limits projection admission, not the shell capture of trusted CLI
  # stdout. Both subprocess stderr streams are discarded before fixed reports.
  printf '%s' "$CURSOR_MAINTENANCE_REPORT" | "${PYTHON[@]}" -I -S -c '
import json,sys
try:
    raw=sys.stdin.buffer.read(4097)
    if len(raw)>4096: raise ValueError("bounded outcome required")
    value=json.loads(raw)
    state=value.get("state") if isinstance(value,dict) else None
    if state not in ("written","unchanged","kept","removed","absent","attached","preserved","incomplete"):
        raise ValueError("known state required")
    print(json.dumps({"state":state}))
    sys.exit(5 if state=="preserved" and value.get("reason")=="linked_directory"
             else 3 if state=="preserved" else 4 if state=="incomplete" else 0)
except (ValueError,TypeError,RecursionError):
    sys.exit(1)
' 2>/dev/null || CURSOR_PROJECTION_STATUS=$?
  case "$CURSOR_PROJECTION_STATUS" in
    0) return 0 ;;
    3) printf '%s\n' 'Cursor memory rule was not updated: custom_or_unavailable.' >&2 ;;
    5) printf '%s\n' 'Cursor memory rule was not updated: linked_directory.' >&2 ;;
    4) printf '%s\n' 'Cursor memory rule was not updated: maintenance_incomplete.' >&2 ;;
    *) printf '%s\n' 'Cursor memory rule was not updated: maintenance_unavailable.' >&2; return 1 ;;
  esac
  return 0
}
if [ "${CONTEXT_CAPSULE_DELIVERED:-}" = "1" ]; then
  cursor_rule_run cursor-rule-retire >/dev/null || true
  exit 0
fi
# Preserve this task's proven prompt rule after checkpointing. Task alias is
# not chat ownership or current canonical-source authority. This only asks:
# "preserved" means another rule is there, which the write below replaces, so
# it reports nothing - the write, keep or retire that follows says what
# happened to the rule.
HELD_RULE=$(cursor_rule_run cursor-rule --action prompt-held --task-id "$TASK_ID" 2>/dev/null || true)
case "$HELD_RULE" in *'"state": "kept"'*) exit 0 ;; esac
CAPSULE_IDENTITY_ARGUMENTS=()
if [ -z "${CONTEXT_TASK_ID:-}" ] && [ -n "$TASK_ID" ]; then
  CAPSULE_IDENTITY_ARGUMENTS=(--expected-branch "$TASK_ID")
fi
CAPSULE_STATUS=1
if command -v timeout > /dev/null 2>&1; then
  CAPSULE=$(timeout "$BUDGET_SECONDS" "${PYTHON[@]}" "$CONTEXT_CLI" hook-context --host cursor --delivery-profile cursor-rule --task-id "$TASK_ID" "${CAPSULE_IDENTITY_ARGUMENTS[@]}" 2>/dev/null)
  CAPSULE_STATUS=$?
else
  CAPSULE=$("${PYTHON[@]}" "$CONTEXT_CLI" hook-context --host cursor --delivery-profile cursor-rule --task-id "$TASK_ID" "${CAPSULE_IDENTITY_ARGUMENTS[@]}" 2>/dev/null)
  CAPSULE_STATUS=$?
fi
if [ "$CAPSULE_STATUS" -eq 3 ]; then
  cursor_rule_run cursor-rule-retire >/dev/null || true
elif [ "$CAPSULE_STATUS" -eq 0 ]; then
  printf '%s' "$CAPSULE" | cursor_rule_run cursor-rule --action write --phase turn --task-id "$TASK_ID" >/dev/null || true
else
  cursor_rule_run cursor-rule --action keep --task-id "$TASK_ID" >/dev/null || true
fi

exit 0
