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

# Output: plain text on stdout, which Claude Code and Codex add to the
# prompt as context.

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
[ "${CONTEXT_MEMORY_RECOVERY:-}" = "1" ] && exit 0

# A host that puts this turn's Task Capsule into the prompt itself sets
# CONTEXT_CAPSULE_DELIVERED=1; the Harness does, retrieved for the message
# alone. A second capsule here would be distilled from that whole prompt and
# spend the turn's memory budget twice. The write half still runs on Stop.
[ "${CONTEXT_CAPSULE_DELIVERED:-}" = "1" ] && exit 0

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

resolve_python || exit 0
[ -f "$CONTEXT_CLI" ] || exit 0

HOOK_STDIN=$(cat)

run() {
  if command -v timeout > /dev/null 2>&1; then
    timeout "$BUDGET_SECONDS" "$@"
  else
    "$@"
  fi
}

# The prompt arrives as JSON on stdin and is passed to the CLI as-is: query
# distillation (informative terms ranked by rarity in the index) lives in
# context.py, and --sanitize there cuts secrets, personal data and pasted
# transcript prefixes out of the prompt before any of it can reach a query or
# a manifest; a prompt with nothing left to search gets no capsule. The
# conversation's session id travels with it, so what this conversation was
# handed in its last few turns is not handed again, and so does the path of
# its transcript, where a compaction since the last turn shows.
# Keep the program in -c: a heredoc would occupy stdin and hide the prompt.
# Output: the session id, the transcript path, the session's directory, the
# prompt, one per line, and a final "\n." that keeps command substitution
# from eating the prompt's own trailing newlines. Bytes both ways: on Windows
# a text stream writes \n as \r\n - every field cut here kept a \r, and the
# session's directory was no directory - and reads by the ANSI code page.
# Isolated (-I): run in the session's folder, Python imported a project's own
# json.py before the standard one, on every prompt.
HOOK_INPUT=$(printf '%s' "$HOOK_STDIN" | "${PYTHON[@]}" -I -c '
import json
import os
import re
import sys

try:
    data = json.loads(sys.stdin.buffer.read())
except (UnicodeDecodeError, ValueError):
    data = {}
if not isinstance(data, dict):
    data = {}
prompt = data.get("prompt", "")
if not isinstance(prompt, str):
    prompt = ""
# The prompt reaches the CLI as one argument: Linux refuses one over 128 KiB
# and Windows a command line over 32,767 characters, so a pasted log past that
# failed the refresh ("Argument list too long") and the turn lost its memory.
# The head and the tail are kept, where a question usually stands.
if len(prompt) > 24000:
    prompt = prompt[:16000] + "\n...\n" + prompt[-8000:]
session = data.get("session_id", "")
if not isinstance(session, str) or not re.fullmatch(r"[A-Za-z0-9._:-]{1,128}", session):
    session = ""
transcript = data.get("transcript_path", "")
if not isinstance(transcript, str) or len(transcript) > 4096 or "\n" in transcript or not os.path.isabs(transcript):
    transcript = ""
cwd = data.get("cwd", "")
if not isinstance(cwd, str) or len(cwd) > 4096 or "\n" in cwd or "\0" in cwd or not os.path.isabs(cwd):
    cwd = ""
sys.stdout.buffer.write((session + "\n" + transcript + "\n" + cwd + "\n" + prompt + "\n.").encode("utf-8", "replace"))
' 2>/dev/null)
case "$HOOK_INPUT" in *$'\n'*$'\n'*$'\n'*) ;; *) HOOK_INPUT=$'\n\n\n\n.' ;; esac
SESSION_ID=${HOOK_INPUT%%$'\n'*}
HOOK_REST=${HOOK_INPUT#*$'\n'}
TRANSCRIPT=${HOOK_REST%%$'\n'*}
HOOK_REST=${HOOK_REST#*$'\n'}
SESSION_DIR=${HOOK_REST%%$'\n'*}
QUERY=${HOOK_REST#*$'\n'}
QUERY=${QUERY%$'\n.'}

# A session that entered a worktree of this repository - Claude Code's
# EnterWorktree - still runs these hooks, and names its own checkout in the
# input's "cwd". It works there, on that worktree's branch. A worktree that has
# none of this installation is served as the dispatcher serves a session
# started in it (memory-bank/scripts/claude_dispatch.sh): this installation's
# state, the worktree's folder as the project. Any other checkout of the
# repository gets no capsule: handed to a session on another branch, this
# checkout's task would read as that session's own state. A subdirectory of
# this checkout, or a directory of another repository, changes nothing.
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
OTHER_CHECKOUT=
if [ -n "$SESSION_DIR" ] && [ "$SESSION_DIR" != "$PROJECT_DIR" ] && [ -d "$SESSION_DIR" ] \
  && THEIRS=$(checkout_identity "$SESSION_DIR") && MINE=$(checkout_identity "$PROJECT_DIR") \
  && [ "${THEIRS%%$'\n'*}" != "${MINE%%$'\n'*}" ] && [ "${THEIRS#*$'\n'}" = "${MINE#*$'\n'}" ]; then
  OTHER_CHECKOUT=${THEIRS%%$'\n'*}
  # Cursor delivers the capsule through a rule file in the project, which in
  # a worktree would be written into it: a Cursor session there is not served.
  # Served are the worktrees of an installation in the root and, attached,
  # those of the launcher's project, whose one state keeps them all (the
  # Harness's); a worktree a dispatcher serves keeps to itself.
  if { [ "$STATE_DIR" != "$ROOT_DIR" ] || [ "$PROJECT_DIR" = "$ROOT_DIR" ]; } \
    && case "$HOOK_STDIN" in *'"cursor_version"'*) false ;; *) true ;; esac \
    && PROJECT_REAL=$(cd "$PROJECT_DIR" && pwd -P) && SESSION_REAL=$(cd "$SESSION_DIR" && pwd -P); then
    MINE_TOP=${MINE%%$'\n'*}
    SERVED=$OTHER_CHECKOUT${PROJECT_REAL:${#MINE_TOP}}
    case "$SESSION_REAL/" in
      "$SERVED"/*)
        if [ -d "$SERVED" ] && [ ! -f "$SERVED/memory-bank/scripts/context.py" ]; then
          PROJECT_DIR=$SERVED
          # The runtime reads these as Windows writes them: Git Bash spells C:\ as /c/,
          # which Python on Windows takes for a folder on the current drive, and the
          # drive letter in upper case, as the save step names the root beside them.
          NATIVE_ROOT=$ROOT_DIR NATIVE_SERVED=$SERVED
          if [ "${OS:-}" = "Windows_NT" ]; then
            case "$NATIVE_ROOT" in /[A-Za-z]|/[A-Za-z]/*) letter=${NATIVE_ROOT:1:1}; NATIVE_ROOT="${letter^^}:${NATIVE_ROOT:2}" ;; esac
            case "$NATIVE_SERVED" in /[A-Za-z]|/[A-Za-z]/*) letter=${NATIVE_SERVED:1:1}; NATIVE_SERVED="${letter^^}:${NATIVE_SERVED:2}" ;; esac
          fi
          # Attached, the launcher's home and state stand; installed, they are this root.
          [ "$STATE_DIR" != "$ROOT_DIR" ] || export ACCELERATOR_HOME="$NATIVE_ROOT" ACCELERATOR_STATE_DIR="$NATIVE_ROOT"
          export ACCELERATOR_PROJECT_DIR="$NATIVE_SERVED"
          OTHER_CHECKOUT=
        fi ;;
    esac
  fi
fi
if [ -n "$OTHER_CHECKOUT" ]; then
  # Cursor reads only JSON from this hook; its answer is printed on exit.
  case "$HOOK_STDIN" in *'"cursor_version"'*) exit 0 ;; esac
  SESSION_BRANCH=$(git -C "$SESSION_DIR" branch --show-current 2>/dev/null)
  echo "Memory not consulted: this session works in another worktree of the repository${SESSION_BRANCH:+ (branch $SESSION_BRANCH)}, and this memory follows the checkout its hooks are installed in."
  echo "Working memory was NOT consulted this turn. Read the canonical sources directly."
  exit 0
fi

# A rebase detaches HEAD on the commits it replays, and the branch being
# rebased is still the work in hand - often the very conflict a turn asks
# about. Git keeps its name in rebase-merge/head-name (rebase-apply/ for the
# apply backend) of this worktree's Git directory; the runtime admits it as
# the inferred branch only while that rebase is in progress. So does a
# bisect, for the branch it started from.
rebased_branch() {
  local directory state name
  directory=$(git -C "$PROJECT_DIR" rev-parse --absolute-git-dir 2>/dev/null) || return 1
  for state in rebase-merge rebase-apply; do
    [ -f "$directory/$state/head-name" ] || continue
    name=
    IFS= read -r name < "$directory/$state/head-name" || [ -n "$name" ] || continue
    case "$name" in refs/heads/?*) printf '%s' "${name#refs/heads/}"; return 0 ;; esac
  done
  # A bisect tests commits on a detached HEAD; BISECT_START keeps the branch
  # it started from (a commit ID when it started detached) until the reset.
  [ -f "$directory/BISECT_START" ] || return 1
  name=
  IFS= read -r name < "$directory/BISECT_START" || [ -n "$name" ] || return 1
  [ -n "$name" ] && [[ ! $name =~ ^[0-9a-f]{40}([0-9a-f]{24})?$ ]] || return 1
  printf '%s' "$name"
}
# A worktree Codex makes for a thread - `codex --worktree`, a new worktree in
# the Codex app - starts on a detached HEAD and keeps it until someone makes a
# branch there. Codex writes the thread that owns it into codex-thread.json of
# the worktree's Git directory, {"version": 1, "ownerThreadId": "<UUID>"}.
# Until a branch names the work, that thread does - codex-thread/<UUID>, which
# the runtime admits only while HEAD stays detached and the record names it.
# Any other detached HEAD names no task.
codex_thread_task() {
  local directory record= version=$'"version"[ \t\n\v\f\r]*:[ \t\n\v\f\r]*1[ \t\n\v\f\r]*[,}]'
  local owner=$'"ownerThreadId"[ \t\n\v\f\r]*:[ \t\n\v\f\r]*"([0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12})"'
  directory=$(git -C "$PROJECT_DIR" rev-parse --absolute-git-dir 2>/dev/null) || return 1
  [ -f "$directory/codex-thread.json" ] && [ ! -L "$directory/codex-thread.json" ] || return 1
  IFS= read -r -d '' -n 4097 record < "$directory/codex-thread.json"
  [ "${#record}" -le 4096 ] && [[ $record =~ $version ]] && [[ $record =~ $owner ]] || return 1
  printf 'codex-thread/%s' "${BASH_REMATCH[1]}"
}
TASK_ID="${CONTEXT_TASK_ID:-$(git -C "$PROJECT_DIR" branch --show-current 2>/dev/null)}"
[ -n "$TASK_ID" ] || TASK_ID=$(rebased_branch)
[ -n "$TASK_ID" ] || TASK_ID=$(codex_thread_task)

# Arm one semantic result capture for this real prompt. The CLI recognizes
# an internal Stop continuation and preserves its original challenge. It runs
# beside the refresh below: neither reads what the other writes (the arm
# leaves the unsaved-result notice to this hook's own line, and only Cursor's
# arm writes it for the capsule), and the line is printed when the refresh
# returns, before anything else. One Python start after the other was a
# third of every prompt (209 -> 147 ms).
NATIVE_PID=
if [ -f "$ROOT_DIR/memory-bank/scripts/native_results.py" ] && [ -n "$TASK_ID" ]; then
  NATIVE_FILE=$(mktemp "${TMPDIR:-/tmp}/working-memory-native.XXXXXX" 2>/dev/null) || NATIVE_FILE=/dev/null
  printf '%s' "$HOOK_STDIN" | run "${PYTHON[@]}" "$CONTEXT_CLI" native-prompt \
    --host codex --task-id "$TASK_ID" > "$NATIVE_FILE" 2>/dev/null &
  NATIVE_PID=$!
fi

# The Stop hook's save continuation, which Codex and Cursor send back as a
# prompt, is this turn's bookkeeping, not a request: the turn's own prompt
# brought the capsule. Cursor reads its rule before this hook runs, so a
# capsule rendered here for the instruction's words would be what the
# person's next request carries instead of the last prompt's.
case "$QUERY" in
  "INTERNAL_NATIVE_MEMORY_RESULT "*)
    [ -z "${NATIVE_PID:-}" ] || wait "$NATIVE_PID"
    [ -z "${NATIVE_FILE:-}" ] || [ "$NATIVE_FILE" = /dev/null ] || rm -f "$NATIVE_FILE" 2>/dev/null
    exit 0 ;;
esac

# One process refreshes all three layers and assembles the capsule. Splitting
# them would index twice, because retrieval refreshes the index itself.
# --ephemeral keeps the per-request manifest, including the query text, in
# ignored local state instead of shared Git history.
ARGUMENTS=(refresh --host codex)
if [ -n "$QUERY" ] && [ -n "$TASK_ID" ]; then
  # --sanitize: secrets, personal data and pasted transcript prefixes are cut
  # out of the prompt, instead of the whole turn going without memory.
  ARGUMENTS+=(--query "$QUERY" --task-id "$TASK_ID" --ephemeral --sanitize)
  # Only a shell-inferred identity is tied to this branch observation.
  [ -n "${CONTEXT_TASK_ID:-}" ] || ARGUMENTS+=(--expected-branch "$TASK_ID")
  [ -n "$SESSION_ID" ] && ARGUMENTS+=(--session-id "$SESSION_ID")
  [ -n "$SESSION_ID" ] && [ -n "$TRANSCRIPT" ] && ARGUMENTS+=(--transcript "$TRANSCRIPT")
fi

# stderr goes to a private file rather than /dev/null: when the refresh
# fails, its last line is what names the failure below.
ERROR_FILE=$(mktemp "${TMPDIR:-/tmp}/working-memory-read.XXXXXX" 2>/dev/null) || ERROR_FILE=/dev/null
REPORT=$(run "${PYTHON[@]}" "$CONTEXT_CLI" "${ARGUMENTS[@]}" 2>"$ERROR_FILE")
HOOK_STATUS=$?
if [ -n "$NATIVE_PID" ]; then
  wait "$NATIVE_PID"
  NATIVE_STATUS=$?
  NATIVE_REPORT=
  [ "$NATIVE_FILE" = /dev/null ] || { NATIVE_REPORT=$(cat "$NATIVE_FILE" 2>/dev/null); rm -f "$NATIVE_FILE" 2>/dev/null; }
  if [ "$NATIVE_STATUS" -eq 0 ] && [ "$NATIVE_REPORT" = "Previous native memory result was not saved completely." ]; then
    printf '%s\n' "$NATIVE_REPORT"
  fi
fi
# A refresh stopped at its budget may already have printed part of a
# capsule, and a cut capsule reads like a whole one. It is dropped, and the
# turn is told below that memory was not consulted.
[ "$HOOK_STATUS" -eq 124 ] && REPORT=""
# One bounded line: enough to name the failure, never enough to paste a trace
# or anything the CLI refused to accept into the prompt.
DETAIL=$(grep -v '^[[:space:]]*$' "$ERROR_FILE" 2>/dev/null | tail -n 1 | tr '\t' ' ' | cut -c1-160)
[ "$ERROR_FILE" = /dev/null ] || rm -f "$ERROR_FILE" 2>/dev/null

# Recorded before the early exit below, because the one case worth measuring
# is the one that produces no report: on a timeout `timeout` returns 124 and
# the Python process was killed before it could append anything itself, so
# the shell has to write this line or the turn leaves no trace at all. The
# record carries a status and nothing else — no query, no paths.
# Attached, a state directory that is a symbolic link - or a link on the way
# to this file - is refused by the runtime (workspace_roots.py), and the line
# is not appended through it either: it would land wherever the link points.
HEALTH_DIR="$STATE_DIR/memory-bank/local"
if [ "$STATE_DIR" != "$ROOT_DIR" ]; then
  for STATE_ENTRY in "${STATE_DIR%/}" "$STATE_DIR/memory-bank" "$HEALTH_DIR" "$HEALTH_DIR/refresh-health.ndjson"; do
    [ -L "$STATE_ENTRY" ] && HEALTH_DIR=""
  done
fi
if [ -n "$HEALTH_DIR" ] && { [ -d "$HEALTH_DIR" ] || mkdir -p "$HEALTH_DIR" 2>/dev/null; }; then
  # 2>/dev/null before >>: bash opens redirections left to right, and a file
  # it cannot open (a read-only state directory) printed its error first.
  printf '{"at":"%s","hook_status":%s,"source":"hook"}\n' \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$HOOK_STATUS" \
    2>/dev/null >> "$HEALTH_DIR/refresh-health.ndjson" || true
fi

if [ -z "$REPORT" ]; then
  # A refresh that succeeded with nothing to say - a prompt the sanitizer
  # left nothing of - stays silent. One that failed says so, and says what
  # follows from it, instead of leaving the turn to assume memory was read.
  [ "$HOOK_STATUS" -eq 0 ] && exit 0
  if [ "$HOOK_STATUS" -eq 124 ]; then
    echo "Memory refresh unavailable: it exceeded its ${BUDGET_SECONDS}s budget."
    # The first index of a large project can take longer than a prompt's
    # budget, and an index stopped there starts over on the next prompt.
    # Installed, in the project root: run elsewhere - attached, or for a
    # worktree its main checkout serves - the command needs the roots' variables.
    if [ "$PROJECT_DIR" = "$ROOT_DIR" ]; then
      echo "If this repeats on every prompt, build the index once outside the hook: ${PYTHON[*]} memory-bank/scripts/context.py index"
    fi
  else
    echo "Memory refresh unavailable: it exited $HOOK_STATUS${DETAIL:+ — $DETAIL}."
  fi
  echo "Working memory was NOT consulted this turn. Read the canonical sources directly."
  exit 0
fi

# The capsule names itself: its memory section says the text is reference
# data to check against the cited file, so no banner precedes it.
echo "$REPORT"

exit 0
