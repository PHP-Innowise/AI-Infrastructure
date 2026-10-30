#!/bin/bash
# Loop Detection Hook
# Watches one prompt-turn for a fix-and-retry loop: the same command failure
# coming back after edits. Edits alone never warn - a turn that edits one
# file many times is ordinary work - and a failure that changes is progress.
# Hook type: PostToolUse (Edit|Write|MultiEdit|NotebookEdit) records an edit;
# PostToolUseFailure (Bash) weighs a failed command.
# Exit code: always 0 (a warning travels as JSON additionalContext).
#
# Currently: the same failure back after an edit for the second time warns
# the model; for the fourth time the user is told as well.

# Consume the complete hook payload without relying on external utilities.
# `read -d ''` returns nonzero at EOF, which is the expected delimiter here.
INPUT=
IFS= read -r -d '' INPUT || :

# A top-level string field of the payload. A key quoted inside another string
# arrives escaped (\"key\"), so only the field itself can match.
json_field() {
  printf '%s\n' "$INPUT" \
    | sed -n -E 's/.*"'"$1"'"[[:space:]]*:[[:space:]]*"(([^"\\]|\\.)*)".*/\1/p' | head -1
}

# A JSON string body as lines of text: line breaks restored, carriage returns
# and tabs made spaces, quotes unescaped. Other escapes stay as they are,
# which is all a comparison needs. sed, not bash substitution: that takes a
# tenth of a second on a long test log.
json_text() {
  printf '%s\n' "$1" | sed -e 's/\\n/\
/g' -e 's/\\[rt]/ /g' -e 's/\\"/"/g'
}

# What makes two failures the same: the command's output with every number,
# hex value and absolute path taken out, so the same failure run again hashes
# the same and a changed one does not. The host's own lines around it (Claude
# Code's "Exit code 1", Codex's exit code, wall time and "Output:") go too.
# The first and last 200 lines decide; a failure without output (grep or test
# answering 1) has nothing to compare.
failure_signature() {
  local text
  text=$(json_text "$1")
  { printf '%s\n' "$text" | sed -n '1,200p'; printf '%s\n' "$text" | tail -n 200; } \
    | sed -E -e '/^[[:space:]]*(Exit code|Process exited with code|Wall time|Total output lines)[[:space:]:]/d' \
        -e '/^[[:space:]]*Output:[[:space:]]*$/d' \
        -e 's/0x[0-9A-Fa-f]+//g' -e 's/[0-9]+//g' \
        -e "s#(^|[[:space:]\"'(=])/[^[:space:]\"')]*#\\1#g" \
        -e 's/[[:space:]]+$//' -e '/^$/d' \
    | { IFS= read -r FIRST || exit 0; { printf '%s\n' "$FIRST"; cat; } | cksum | cut -d' ' -f1; }
}

# A stored count, accepted only as digits read in base 10: anything else
# starts over instead of reaching shell arithmetic (a planted `08` would stop
# it, a planted `a[$(cmd)]` would run cmd). count_of NAME VALUE sets NAME.
count_of() {
  case "$2" in
    *[!0-9]*|'') printf -v "$1" 0 ;;
    *) if [ "${#2}" -gt 9 ]; then printf -v "$1" 0; else printf -v "$1" %d "$((10#$2))"; fi ;;
  esac
}

# What the call was. Claude Code reports a failed call only as
# PostToolUseFailure; Codex reports every call as PostToolUse, a shell command
# with the result it returned; Cursor's afterFileEdit names no tool and is an
# edit. Only edit tools advance the edit count - Codex registers this hook
# without a matcher, so a read-only payload carrying a file_path (Read) must
# not count - and only shell commands are weighed as failures: a refused or
# stale edit is a hiccup of the tool, not the loop this hook is for.
EVENT=$(json_field hook_event_name)
TOOL_NAME=$(json_field tool_name)
SHELL_TOOLS='^(bash|shell|local_shell|exec_command)$'
EDIT_TOOLS='(^|[.:/])(write|edit|multiedit|multi_edit|notebookedit|notebook_edit|apply_patch)$'
OUTPUT=
if [ "$EVENT" = PostToolUseFailure ]; then
  printf '%s\n' "$TOOL_NAME" | grep -Eqi "$SHELL_TOOLS" || exit 0
  # An interrupted call is the user stopping it, not a failure.
  case "$INPUT" in *'"is_interrupt":true'*|*'"is_interrupt": true'*) exit 0 ;; esac
  KIND=failure
  OUTPUT=$(json_field error)
elif [ -z "$TOOL_NAME" ] || printf '%s\n' "$TOOL_NAME" | grep -Eqi "$EDIT_TOOLS"; then
  KIND=edit
elif printf '%s\n' "$TOOL_NAME" | grep -Eqi "$SHELL_TOOLS"; then
  # A shell result names its exit status as a field (exec_command's
  # exit_code) or as its first line ("Exit code: 1", "Process exited with
  # code 1"). A result that names none is not weighed, whatever its text says.
  OUTPUT=$(json_field tool_response)
  [ -n "$OUTPUT" ] || OUTPUT=$(json_field output)
  STATUS=$(printf '%s\n' "$INPUT" \
    | sed -n -E 's/.*"(exit_code|exitCode)"[[:space:]]*:[[:space:]]*(-?[0-9]+).*/\2/p' | head -1)
  if [ -z "$STATUS" ]; then
    STATUS=$(json_text "$OUTPUT" | sed -n '1,5p' \
      | sed -n -E 's/^[[:space:]]*(Exit code|Process exited with code)[[:space:]:]+(-?[0-9]+).*/\2/p' | head -1)
  fi
  case "$STATUS" in ''|0|-0) exit 0 ;; esac
  KIND=failure
else
  exit 0
fi

if [ "$KIND" = edit ]; then
  # Extract file path from JSON input (POSIX-compatible, no grep -P)
  FILE_PATH=$(echo "$INPUT" | sed -n 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)

  # Claude Code's NotebookEdit names its file notebook_path.
  if [ -z "$FILE_PATH" ]; then
    FILE_PATH=$(echo "$INPUT" | sed -n 's/.*"notebook_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)
  fi

  # Codex edits through apply_patch, whose payload names no file_path: the
  # files sit inside the patch text, one per "*** Add File: X",
  # "*** Update File: X" or "*** Move to: X" line - the markers
  # file-naming-validator.sh reads too. On the wire the patch is one JSON
  # string, so its line breaks arrive as \n escapes.
  if [ -z "$FILE_PATH" ]; then
    PATCH_TEXT=${INPUT//\\n/$'\n'}
    FILE_PATH=$(printf '%s\n' "$PATCH_TEXT" \
      | sed -n -E 's/^\*\*\* (Add File|Update File|Move to): ([^"]*).*$/\2/p' \
      | sed -e 's/\\r$//' -e 's/[[:space:]]*$//' | grep -v '^$')
  fi

  [ -n "$FILE_PATH" ] || exit 0
else
  SIGNATURE=$(failure_signature "$OUTPUT")
  [ -n "$SIGNATURE" ] || exit 0
fi

# Counters live in a per-user, per-repository directory under TMPDIR (or
# /tmp), namespaced by a stable hash of the repo root so parallel projects do
# not share them. The name is predictable, so a directory that is already
# there is used only when it is a real directory this user owns - one another
# user planted could otherwise feed this hook its counts or point a counter at
# a file to overwrite - and a counter that is a symbolic link is never read
# or written. Anything else turns the count off rather than trusting it.
REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
REPO_KEY=$(printf '%s' "$REPO_ROOT" | cksum | cut -d' ' -f1)
TRACK_BASE=${TMPDIR:-/tmp}
TRACK_DIR="${TRACK_BASE%/}/claude-loop-detection-${EUID:-0}-$REPO_KEY"
mkdir -m 700 "$TRACK_DIR" 2>/dev/null
if [ -L "$TRACK_DIR" ] || [ ! -d "$TRACK_DIR" ] || [ ! -O "$TRACK_DIR" ]; then
  exit 0
fi

# Counts belong to one session: the host's session id (Claude Code and Codex
# send session_id, Cursor conversation_id) keys every counter file, so two
# sessions in one checkout never add to each other's counts and the session
# start hook clears only its own. A payload without one shares the key
# "shared".
SESSION_KEY=$(printf '%s' "$INPUT" \
  | sed -n -E 's/.*"(session_id|conversation_id)"[[:space:]]*:[[:space:]]*"([^"]*)".*/\2/p' | head -1)
SESSION_KEY=${SESSION_KEY//[^A-Za-z0-9]/}
SESSION_KEY=${SESSION_KEY:0:64}
[ -n "$SESSION_KEY" ] || SESSION_KEY=shared

# Within a session they belong to one prompt-turn: Claude Code's prompt_id,
# Codex's turn_id, Cursor's generation_id. A new turn starts the session's
# edits and failures over - across the turns of a long session a failure
# comes back as a matter of course. A payload without a turn id (Claude Code
# before its first prompt) counts toward the session as a whole.
TURN_KEY=$(printf '%s' "$INPUT" \
  | sed -n -E 's/.*"(prompt_id|turn_id|generation_id)"[[:space:]]*:[[:space:]]*"([^"]*)".*/\2/p' | head -1)
TURN_KEY=${TURN_KEY//[^A-Za-z0-9]/}
TURN_KEY=${TURN_KEY:0:64}
[ -n "$TURN_KEY" ] || TURN_KEY=session
TURN_FILE="$TRACK_DIR/turn-$SESSION_KEY"
[ -L "$TURN_FILE" ] && exit 0
SEEN_TURN=
EDITS=
[ -f "$TURN_FILE" ] && { IFS= read -r SEEN_TURN; IFS= read -r EDITS; } 2>/dev/null < "$TURN_FILE"
if [ "$SEEN_TURN" != "$TURN_KEY" ]; then
  find "$TRACK_DIR" -maxdepth 1 -type f \( -name "edit-$SESSION_KEY-*" -o -name "fail-$SESSION_KEY-*" \) \
    -delete 2>/dev/null
  EDITS=0
fi
count_of EDITS "$EDITS"

if [ "$KIND" = edit ]; then
  # An edit counts once toward the turn (a patch naming three files is one
  # fix attempt) and once toward each file it names, so a warning can name
  # the file the turn kept editing. Each counter it touches also restarts the
  # Bash Validator's repetition count for the commands run before it.
  while IFS= read -r EDITED; do
    [ -n "$EDITED" ] || continue
    # Create a safe filename from the path (portable: md5sum on Linux, md5 on macOS)
    if command -v md5sum > /dev/null 2>&1; then
      SAFE_NAME=$(echo "$EDITED" | md5sum | cut -d' ' -f1)
    elif command -v md5 > /dev/null 2>&1; then
      SAFE_NAME=$(echo "$EDITED" | md5 -q)
    else
      SAFE_NAME=$(echo "$EDITED" | cksum | cut -d' ' -f1)
    fi
    TRACK_FILE="$TRACK_DIR/edit-$SESSION_KEY-$SAFE_NAME"
    [ -L "$TRACK_FILE" ] && continue
    COUNT=
    [ -f "$TRACK_FILE" ] && IFS= read -r COUNT 2>/dev/null < "$TRACK_FILE"
    count_of COUNT "$COUNT"
    # The path is quoted back to the model inside JSON: no quote, backslash
    # or control character.
    SHOWN=${EDITED//[\\\"]/}
    SHOWN=${SHOWN//[[:cntrl:]]/}
    printf '%s\n%s\n' "$((COUNT + 1))" "${SHOWN:0:200}" > "$TRACK_FILE" 2>/dev/null
  done <<< "$FILE_PATH"
  printf '%s\n%s\n' "$TURN_KEY" "$((EDITS + 1))" > "$TURN_FILE" 2>/dev/null
  exit 0
fi

# The failure weighed against the turn so far: how often it has come back,
# and how many of those times an edit came before it - the fix attempts it
# survived. Met again with no edit since, it is a rerun, which the Bash
# Validator counts per command.
FAIL_FILE="$TRACK_DIR/fail-$SESSION_KEY-$SIGNATURE"
[ -L "$FAIL_FILE" ] && exit 0
SEEN= AT= REPEATS= TOLD=
[ -f "$FAIL_FILE" ] && {
  IFS= read -r SEEN; IFS= read -r AT; IFS= read -r REPEATS; IFS= read -r TOLD
} 2>/dev/null < "$FAIL_FILE"
count_of SEEN "$SEEN"
count_of AT "$AT"
count_of REPEATS "$REPEATS"
count_of TOLD "$TOLD"
if [ "$SEEN" -gt 0 ] && [ "$EDITS" -gt "$AT" ]; then
  REPEATS=$((REPEATS + 1))
fi
SEEN=$((SEEN + 1))
LEVEL=0
if [ "$REPEATS" -ge 4 ] && [ "$TOLD" -lt 2 ]; then
  LEVEL=2
elif [ "$REPEATS" -ge 2 ] && [ "$TOLD" -lt 1 ]; then
  LEVEL=1
fi
[ "$LEVEL" -gt "$TOLD" ] && TOLD=$LEVEL
printf '%s\n%s\n%s\n%s\n' "$SEEN" "$EDITS" "$REPEATS" "$TOLD" > "$FAIL_FILE" 2>/dev/null
# The turn is recorded even when no edit has written it yet, or the next
# edit would take this failure for a previous turn's.
printf '%s\n%s\n' "$TURN_KEY" "$EDITS" > "$TURN_FILE" 2>/dev/null
[ "$LEVEL" -gt 0 ] || exit 0

# The file this turn edited most, named in the message.
BUSIEST=
TOP=0
for COUNTER in "$TRACK_DIR/edit-$SESSION_KEY-"*; do
  [ -f "$COUNTER" ] && [ ! -L "$COUNTER" ] || continue
  N=
  NAME=
  { IFS= read -r N; IFS= read -r NAME; } 2>/dev/null < "$COUNTER"
  count_of N "$N"
  if [ "$N" -gt "$TOP" ]; then
    TOP=$N
    BUSIEST=$NAME
  fi
done
BUSIEST=${BUSIEST//[\\\"]/}
BUSIEST=${BUSIEST//[[:cntrl:]]/}
[ -n "$BUSIEST" ] && BUSIEST=" (most edits in this turn: '${BUSIEST:0:200}', $TOP)"

ESCALATE="or use /debugger to find the root cause."
NOTICE=
if [ "$LEVEL" -ge 2 ]; then
  WARNING="STOP: the same failure has come back $SEEN times in this turn, $REPEATS of them after an edit$BUSIEST, the earlier warning notwithstanding. Do not edit again for it: tell the user what you tried and what still fails, and ask how to proceed, $ESCALATE"
  NOTICE="Loop detection: the same failure has come back $SEEN times in this turn, $REPEATS of them after an edit. The agent was asked to stop and report; interrupt the turn if it goes on."
else
  WARNING="WARNING: the same failure has come back $SEEN times in this turn, $REPEATS of them after an edit$BUSIEST. Another edit along the same line is unlikely to fix it: re-read the whole failing output, question the assumption behind the last edits and try a materially different approach, $ESCALATE"
fi

# Claude Code and Codex hand a hook's additionalContext to the model next to
# the failed call's result and show its systemMessage to the user. Claude
# Code drops an answer that names another event than the one it answers.
ANSWERED=PostToolUse
[ "$EVENT" = PostToolUseFailure ] && ANSWERED=PostToolUseFailure
if [ -n "$NOTICE" ]; then
  printf '{"systemMessage":"%s","hookSpecificOutput":{"hookEventName":"%s","additionalContext":"%s"}}\n' \
    "$NOTICE" "$ANSWERED" "$WARNING"
else
  printf '{"hookSpecificOutput":{"hookEventName":"%s","additionalContext":"%s"}}\n' "$ANSWERED" "$WARNING"
fi
exit 0
