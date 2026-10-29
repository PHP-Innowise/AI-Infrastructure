#!/bin/bash
# The command repetition guard of a generated bash-validator.sh. Not a hook:
# hook-forge step 2 copies every byte from the opening `bash-validator
# repetition guard` marker below through the closing one into the
# bash-validator.sh of each edition it generates, between the `fi` of the
# bv_validate call and the final `exit 0`, and bootstrap-verifier holds every
# copy to these bytes. This header stays here: each marker occurs once.
#
# The block reads $cmd and $input, which the payload decoding copied from the
# generator's own validator assigns, and $BV_HOST (claude, cursor or codex),
# which the generated hook assigns below the generic section. It counts in the
# directory the generated loop-detection.sh keeps its edit-* counters in
# (hook-forge step 5). It is the four editions' guard: carry a change made
# there into this file.

# >>> bash-validator repetition guard >>>
# Repetition guard. The file-edit counterpart lives in loop-detection.sh; a
# command loop is invisible to it, because rerunning one failing command
# forever touches no file. This hook sees the call before it runs and cannot
# see its result, so identical invocations are the only signal available -
# and they are counted per exact command string, so any real change of
# approach starts its own count. A file edit that loop-detection.sh recorded
# since the command last ran starts its count over too: edit-and-rerun is
# progress, and only a rerun with nothing changed is the loop this guards.
# Only a command that passed the rules above is counted: a refused one never
# ran.
#
# Polling is not a loop. A read-only status query standing alone - CI checks
# and runs (gh pr checks/status/view, gh run list/view/watch), git status, a
# container's or a cluster's state and logs (docker [compose] ps/logs,
# kubectl get/describe/logs), a log tail - changes its answer without any
# edit, so asking again is waiting and is not counted. Optionally after a
# `sleep N &&` and piped into head/tail/wc/cut/grep/jq; any other chain counts
# like any command.
BV_POLL_QUERY='^(sleep [0-9.]+[smhd]? *(&&|;) *)?(git status|gh pr (checks|status|view)|gh run (list|view|watch)|docker (compose )?(ps|logs)|kubectl (get|describe|logs)|tail)( [^;&`$()<>|]*)?(\| *(head|tail|wc|cut|grep|jq)( [^;&`$()<>|]*)?)*$'
if [[ "$cmd" =~ $BV_POLL_QUERY ]] && [[ "$cmd" != *'||'* ]] && [[ "$cmd" != *$'\n'* ]]; then
  exit 0
fi

# The window is a session. Counts are keyed by the host's session id (Claude
# Code and Codex send session_id, Cursor conversation_id), so two sessions in
# one checkout never add to each other's counts, and the session start hook
# clears only its own. A payload without one shares the key "shared".
SESSION_KEY=$(printf '%s' "$input" \
  | sed -n -E 's/.*"(session_id|conversation_id)"[[:space:]]*:[[:space:]]*"([^"]*)".*/\2/p' | head -1)
SESSION_KEY=${SESSION_KEY//[^A-Za-z0-9]/}
SESSION_KEY=${SESSION_KEY:0:64}
[ -n "$SESSION_KEY" ] || SESSION_KEY=shared

# The counter directory is the per-user one loop-detection.sh uses, named for
# the host this copy serves (BV_HOST). Its name is predictable, so a directory
# that is already there is used only when it is a real directory this user
# owns - one another user planted could otherwise pre-seed a count that blocks
# a first run, or hold a counter that is a link to a file this hook would then
# overwrite - and a counter that is a symbolic link is never read or written.
# Anything else turns the guard off rather than trusting it.
REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
REPO_KEY=$(printf '%s' "$REPO_ROOT" | cksum | cut -d' ' -f1)
TRACK_BASE=${TMPDIR:-/tmp}
TRACK_DIR="${TRACK_BASE%/}/$BV_HOST-loop-detection-${EUID:-0}-$REPO_KEY"
mkdir -m 700 "$TRACK_DIR" 2>/dev/null
if [ -L "$TRACK_DIR" ] || [ ! -d "$TRACK_DIR" ] || [ ! -O "$TRACK_DIR" ]; then
  exit 0
fi

if command -v md5sum > /dev/null 2>&1; then
  COMMAND_KEY=$(printf '%s' "$cmd" | md5sum | cut -d' ' -f1)
elif command -v md5 > /dev/null 2>&1; then
  COMMAND_KEY=$(printf '%s' "$cmd" | md5 -q)
else
  COMMAND_KEY=$(printf '%s' "$cmd" | cksum | tr -d ' ')
fi
TRACK_FILE="$TRACK_DIR/cmd-$SESSION_KEY-$COMMAND_KEY"
[ -L "$TRACK_FILE" ] && exit 0

# Only digits are a count, read in base 10: "08" would otherwise abort the
# arithmetic and switch the guard off for that command.
COUNT=0
[ -f "$TRACK_FILE" ] && COUNT=$(cat "$TRACK_FILE" 2>/dev/null)
case "$COUNT" in *[!0-9]*|'') COUNT=0 ;; esac
[ "${#COUNT}" -gt 9 ] && COUNT=0
COUNT=$((10#$COUNT))
if [ "$COUNT" -gt 0 ] && [ -n "$(find "$TRACK_DIR" -maxdepth 1 -type f -name 'edit-*' -newer "$TRACK_FILE" -print -quit 2>/dev/null)" ]; then
  COUNT=0
fi
COUNT=$((COUNT + 1))
echo "$COUNT" > "$TRACK_FILE" 2>/dev/null

# The messages name no skill: which skills a generated project has depends on
# what its scan found.
if [ "$COUNT" -ge 12 ]; then
  {
    printf '[bash-validator] BLOCKED: this exact command has run %s times this session.\n' "$COUNT"
    printf '   Repeating it again is not a new attempt. Either change the\n'
    printf '   command (narrow it, add the failing case, read the output\n'
    printf '   differently) or find the root cause before running it again.\n'
  } >&2
  exit 2
elif [ "$COUNT" -ge 6 ]; then
  BV_WARNING="[bash-validator] WARNING: this exact command has run $COUNT times this session. If it keeps failing, look for the root cause instead of another rerun."
  if [ "$BV_HOST" = cursor ]; then
    # Cursor documents no channel that shows the agent a message about a
    # shell command it lets run (beforeShellExecution's agent_message
    # accompanies a denial), so here the warning reaches only the user, in
    # the hooks output, and the agent meets the guard first at the block.
    printf '%s\n' "$BV_WARNING" >&2
    exit 1
  fi
  # Claude Code and Codex add a PreToolUse hook's additionalContext to the
  # model's context next to the tool result, without blocking the call or
  # touching its permission decision. A warning on stderr with a non-blocking
  # exit code reaches only the user's transcript: the agent would meet the
  # guard first at the block.
  printf '{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"%s"}}\n' "$BV_WARNING"
fi
# <<< bash-validator repetition guard <<<
