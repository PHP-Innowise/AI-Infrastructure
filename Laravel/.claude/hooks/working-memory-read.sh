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
# Cursor also runs the Claude Code hooks it finds. Where this project has its
# own Cursor hooks they serve the session; this copy stands down.
case "$HOOK_STDIN" in
  *'"cursor_version"'*) [ -f "$ROOT_DIR/.cursor/hooks.json" ] && exit 0 ;;
esac

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
# Output: the session id, the transcript path, the prompt, one per line, and
# a final "\n." that keeps command substitution from eating the prompt's own
# trailing newlines.
HOOK_INPUT=$(printf '%s' "$HOOK_STDIN" | "${PYTHON[@]}" -c '
import json
import os
import re
import sys

try:
    data = json.load(sys.stdin)
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
sys.stdout.write(session + "\n" + transcript + "\n" + prompt + "\n.")
' 2>/dev/null)
case "$HOOK_INPUT" in *$'\n'*$'\n'*) ;; *) HOOK_INPUT=$'\n\n\n.' ;; esac
SESSION_ID=${HOOK_INPUT%%$'\n'*}
HOOK_REST=${HOOK_INPUT#*$'\n'}
TRANSCRIPT=${HOOK_REST%%$'\n'*}
QUERY=${HOOK_REST#*$'\n'}
QUERY=${QUERY%$'\n.'}

# A rebase detaches HEAD on the commits it replays, and the branch being
# rebased is still the work in hand - often the very conflict a turn asks
# about. Git keeps its name in rebase-merge/head-name (rebase-apply/ for the
# apply backend) of this worktree's Git directory; the runtime admits it as
# the inferred branch only while that rebase is in progress.
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

# Arm one semantic result capture for this real prompt. The CLI recognizes
# an internal Stop continuation and preserves its original challenge.
if [ -f "$ROOT_DIR/memory-bank/scripts/native_results.py" ] && [ -n "$TASK_ID" ]; then
  NATIVE_REPORT=$(printf '%s' "$HOOK_STDIN" | run "${PYTHON[@]}" "$CONTEXT_CLI" native-prompt \
    --host claude --task-id "$TASK_ID" 2>/dev/null)
  NATIVE_STATUS=$?
  if [ "$NATIVE_STATUS" -eq 0 ] && [ "$NATIVE_REPORT" = "Previous native memory result was not saved completely." ]; then
    printf '%s\n' "$NATIVE_REPORT"
  fi
fi

# One process refreshes all three layers and assembles the capsule. Splitting
# them would index twice, because retrieval refreshes the index itself.
# --ephemeral keeps the per-request manifest, including the query text, in
# ignored local state instead of shared Git history.
ARGUMENTS=(refresh --host claude)
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
  printf '{"at":"%s","hook_status":%s,"source":"hook"}\n' \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$HOOK_STATUS" \
    >> "$HEALTH_DIR/refresh-health.ndjson" 2>/dev/null || true
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
    if [ "$STATE_DIR" = "$ROOT_DIR" ]; then
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
