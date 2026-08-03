#!/bin/bash
# File Naming Validator Hook (Infrastructure-Creator)
# Ensures .md files in tasks/ and specs/ follow the skill-prefix naming convention.
# Hook type: PreToolUse:Write|Edit / afterFileEdit
# Exit codes: 0 = pass, 1 = warn (continue), 2 = block
#
# Currently in WARN mode (exit 1). Switch to exit 2 after tuning.

set -uo pipefail

input="$(cat 2>/dev/null || true)"

# Only file-editing tools are subject to naming checks. Codex registers this
# hook without a tool matcher, so read-only payloads that carry a file_path
# (for example Read) must not be warned about. An empty tool_name means the
# host does not send one (Cursor afterFileEdit); fail open in that case.
tool_name="$(printf '%s' "$input" | sed -n 's/.*"tool_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
if [ -n "$tool_name" ] \
  && ! printf '%s\n' "$tool_name" | grep -Eqi '(^|[.:/])(write|edit|multiedit|multi_edit|notebookedit|notebook_edit|apply_patch)$'; then
  exit 0
fi

# Extract the target file path (Claude: file_path; Cursor: filePath/path).
path="$(printf '%s' "$input" | sed -n 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
if [ -z "$path" ]; then
  path="$(printf '%s' "$input" | sed -n 's/.*"filePath"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
fi
if [ -z "$path" ]; then
  exit 0
fi

base="$(basename "$path")"

# Only enforce inside this generator's own tasks/ and specs/ trees.
case "$path" in
  *"/tasks/"*|tasks/*|*"/specs/"*|specs/*) ;;
  *) exit 0 ;;
esac

# Allowed unprefixed files.
case "$base" in
  README.md|CHANGELOG.md|MANIFEST.md|.task-counter) exit 0 ;;
esac

# Non-markdown files are out of scope for this check.
case "$base" in
  *.md) ;;
  *) exit 0 ;;
esac

# Require a skill-name prefix like "infra-scan-...", "stack-scanner-findings.md".
if printf '%s' "$base" | grep -Eq '^[a-z][a-z0-9]*(-[a-z0-9]+)*-[a-z0-9].*\.md$'; then
  exit 0
fi

echo "[file-naming-validator] WARN: '$base' should be prefixed with the skill name, e.g. 'infra-scan-project-profile.md'." >&2
exit 1
