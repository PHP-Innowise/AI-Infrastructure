#!/bin/bash
# Loop Detection Hook
# Tracks edit count per file to detect doom loops.
# Hook type: PostToolUse (Edit|Write|MultiEdit|NotebookEdit)
# Exit codes: 0 = pass (a warning travels as JSON additionalContext),
# 2 = block
#
# Currently: warn at 7, block at 10.

# Consume the complete hook payload without relying on external utilities.
# `read -d ''` returns nonzero at EOF, which is the expected delimiter here.
INPUT=
IFS= read -r -d '' INPUT || :

# Only file-editing tools may advance the loop counter. Codex registers this
# hook without a tool matcher, so read-only payloads that carry a file_path
# (for example Read) must not count as edits. An empty tool_name means the
# host does not send one (Cursor afterFileEdit); fail open in that case.
TOOL_NAME=$(echo "$INPUT" | sed -n 's/.*"tool_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)
if [ -n "$TOOL_NAME" ] \
  && ! printf '%s\n' "$TOOL_NAME" | grep -Eqi '(^|[.:/])(write|edit|multiedit|multi_edit|notebookedit|notebook_edit|apply_patch)$'; then
  exit 0
fi

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

if [ -z "$FILE_PATH" ]; then
  exit 0
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

STATUS=0
WARNING=""
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

  # Increment counter. Only digits are a count, read in base 10: anything
  # else starts over instead of reaching shell arithmetic.
  COUNT=0
  [ -f "$TRACK_FILE" ] && COUNT=$(cat "$TRACK_FILE" 2>/dev/null)
  case "$COUNT" in *[!0-9]*|'') COUNT=0 ;; esac
  [ "${#COUNT}" -gt 9 ] && COUNT=0
  COUNT=$((10#$COUNT + 1))
  echo "$COUNT" > "$TRACK_FILE" 2>/dev/null

  # The path is quoted back to the model inside JSON: no quote or backslash.
  SHOWN=${EDITED//[\\\"]/}
  # Check thresholds
  if [ "$COUNT" -ge 10 ]; then
    echo "BLOCKED: File '$SHOWN' edited $COUNT times this session." >&2
    echo "   This looks like a repeated-edit loop. Consider:" >&2
    echo "   - /debugger to investigate the root cause" >&2
    echo "   - Reassessing your approach" >&2
    STATUS=2
  elif [ "$COUNT" -ge 7 ]; then
    WARNING="${WARNING:+$WARNING }WARNING: File '$SHOWN' edited $COUNT times this session. If you're stuck in a loop, consider using /debugger."
  fi
done <<< "$FILE_PATH"

if [ "$STATUS" -eq 2 ]; then
  [ -n "$WARNING" ] && printf '%s\n' "$WARNING" >&2
  exit 2
fi
if [ -n "$WARNING" ]; then
  # Claude Code and Codex hand a PostToolUse hook's additionalContext to the
  # model next to the tool result. Text with a non-blocking exit code reaches
  # only the user's transcript, so the agent would never see its warning.
  printf '{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"%s"}}\n' "$WARNING"
fi
exit 0
