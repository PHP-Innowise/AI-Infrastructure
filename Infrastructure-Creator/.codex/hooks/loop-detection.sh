#!/bin/bash
# Loop Detection Hook (Infrastructure-Creator)
# Warns when the same file is edited repeatedly in quick succession, a common
# sign of an agent stuck in a retry loop.
# Hook type: PostToolUse:Edit / afterFileEdit
# Exit codes: 0 = pass, 1 = warn (continue)

set -uo pipefail

input="$(cat 2>/dev/null || true)"

# Only file-editing tools may advance the loop counter. Codex registers this
# hook without a tool matcher, so read-only payloads that carry a file_path
# (for example Read) must not count as edits. An empty tool_name means the
# host does not send one (Cursor afterFileEdit); fail open in that case.
tool_name="$(printf '%s' "$input" | sed -n 's/.*"tool_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
if [ -n "$tool_name" ] \
  && ! printf '%s\n' "$tool_name" | grep -Eqi '(^|[.:/])(write|edit|multiedit|multi_edit|notebookedit|notebook_edit|apply_patch)$'; then
  exit 0
fi

path="$(printf '%s' "$input" | sed -n 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
if [ -z "$path" ]; then
  path="$(printf '%s' "$input" | sed -n 's/.*"filePath"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
fi
if [ -z "$path" ]; then
  exit 0
fi

# Namespace the state dir by a stable hash of the repo root so parallel
# projects do not share counters.
repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
repo_key="$(printf '%s' "$repo_root" | cksum | cut -d' ' -f1)"
state_dir="${TMPDIR:-/tmp}/infra-creator-loopdetect-$repo_key"
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
