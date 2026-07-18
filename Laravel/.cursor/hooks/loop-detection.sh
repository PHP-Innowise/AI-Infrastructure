#!/bin/bash
# Loop Detection Hook
# Tracks edit count per file to detect doom loops.
# Cursor hook event: afterFileEdit.
# Exit codes: 0 = pass, 1 = warn (continue), 2 = block
#
# Currently: warn at 7, block at 10.

INPUT=$(cat)

# Extract file path from JSON input (POSIX-compatible, no grep -P).
# Cursor payloads may use "file_path" or "path"; try both.
FILE_PATH=$(echo "$INPUT" | sed -n 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)
if [ -z "$FILE_PATH" ]; then
  FILE_PATH=$(echo "$INPUT" | sed -n 's/.*"path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)
fi

if [ -z "$FILE_PATH" ]; then
  exit 0
fi

# Use a temp directory for tracking edit counts
TRACK_DIR="/tmp/cursor-loop-detection"
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
  echo "🚫 BLOCKED: File '$FILE_PATH' edited $COUNT times this session."
  echo "   This looks like a doom loop. Consider:"
  echo "   - /debugger to investigate the root cause"
  echo "   - Reassessing your approach"
  exit 2
elif [ "$COUNT" -ge 7 ]; then
  echo "⚠️  WARNING: File '$FILE_PATH' edited $COUNT times this session."
  echo "   If you're stuck in a loop, consider using /debugger."
  exit 1
fi

exit 0
