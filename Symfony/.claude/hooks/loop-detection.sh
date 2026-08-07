#!/bin/bash
# Loop Detection Hook
# Tracks edit count per file to detect doom loops.
# Hook type: PostToolUse:Edit
# Exit codes: 0 = pass, 1 = warn (continue), 2 = block
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

if [ -z "$FILE_PATH" ]; then
  exit 0
fi

# Use a per-repository temp directory for tracking edit counts, namespaced by a
# stable hash of the repo root so parallel projects do not share counters.
REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
REPO_KEY=$(printf '%s' "$REPO_ROOT" | cksum | cut -d' ' -f1)
TRACK_DIR="/tmp/claude-loop-detection-$REPO_KEY"
mkdir -p "$TRACK_DIR"

# Create a safe filename from the path (portable: md5sum on Linux, md5 on macOS)
if command -v md5sum > /dev/null 2>&1; then
  SAFE_NAME=$(echo "$FILE_PATH" | md5sum | cut -d' ' -f1)
elif command -v md5 > /dev/null 2>&1; then
  SAFE_NAME=$(echo "$FILE_PATH" | md5 -q)
else
  SAFE_NAME=$(echo "$FILE_PATH" | cksum | cut -d' ' -f1)
fi
TRACK_FILE="$TRACK_DIR/$SAFE_NAME"

# Increment counter
if [ -f "$TRACK_FILE" ]; then
  COUNT=$(cat "$TRACK_FILE")
  COUNT=$((COUNT + 1))
else
  COUNT=1
fi

echo "$COUNT" > "$TRACK_FILE"

# Check thresholds
if [ "$COUNT" -ge 10 ]; then
  echo "BLOCKED: File '$FILE_PATH' edited $COUNT times this session." >&2
  echo "   This looks like a repeated-edit loop. Consider:" >&2
  echo "   - /debugger to investigate the root cause" >&2
  echo "   - Reassessing your approach" >&2
  exit 2
elif [ "$COUNT" -ge 7 ]; then
  echo "⚠️  WARNING: File '$FILE_PATH' edited $COUNT times this session."
  echo "   If you're stuck in a loop, consider using /debugger."
  exit 1
fi

exit 0
