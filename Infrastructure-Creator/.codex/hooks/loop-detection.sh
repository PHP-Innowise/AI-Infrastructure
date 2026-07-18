#!/bin/bash
# Loop Detection Hook (Infrastructure-Creator)
# Warns when the same file is edited repeatedly in quick succession, a common
# sign of an agent stuck in a retry loop.
# Hook type: PostToolUse:Edit / afterFileEdit
# Exit codes: 0 = pass, 1 = warn (continue)

set -uo pipefail

input="$(cat 2>/dev/null || true)"

path="$(printf '%s' "$input" | sed -n 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
if [ -z "$path" ]; then
  path="$(printf '%s' "$input" | sed -n 's/.*"filePath"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
fi
if [ -z "$path" ]; then
  exit 0
fi

state_dir="${TMPDIR:-/tmp}/infra-creator-loopdetect"
mkdir -p "$state_dir" 2>/dev/null || exit 0

# Hash the path to a state file name.
key="$(printf '%s' "$path" | cksum | awk '{print $1}')"
state_file="$state_dir/$key"

now="$(date +%s)"
count=1
if [ -f "$state_file" ]; then
  last_ts="$(sed -n '1p' "$state_file" 2>/dev/null || echo 0)"
  last_count="$(sed -n '2p' "$state_file" 2>/dev/null || echo 0)"
  # Reset the counter if more than 120s elapsed since the last edit.
  if [ $(( now - last_ts )) -le 120 ]; then
    count=$(( last_count + 1 ))
  fi
fi

printf '%s\n%s\n' "$now" "$count" > "$state_file"

if [ "$count" -ge 5 ]; then
  echo "[loop-detection] WARN: '$(basename "$path")' edited $count times in under 2 minutes - possible loop. Re-read the file and reconsider the approach before editing again." >&2
  exit 1
fi

exit 0
