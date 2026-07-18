#!/bin/bash
# File Naming Validator Hook
# Validates that .md files in tasks/ and specs/ follow skill-prefix naming convention.
# Cursor hook event: afterFileEdit (fires after any file edit).
# Exit codes: 0 = pass, 1 = warn (continue), 2 = block
#
# Currently in WARN mode (exit 1). Switch to exit 2 after tuning.

# Read the tool input from stdin
INPUT=$(cat)

# Extract file path from the JSON input (POSIX-compatible, no grep -P).
# Cursor payloads may use "file_path" or "path"; try both.
FILE_PATH=$(echo "$INPUT" | sed -n 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)
if [ -z "$FILE_PATH" ]; then
  FILE_PATH=$(echo "$INPUT" | sed -n 's/.*"path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)
fi

# If no file path found, pass through
if [ -z "$FILE_PATH" ]; then
  exit 0
fi

# Only check .md files
if [[ "$FILE_PATH" != *.md ]]; then
  exit 0
fi

# Only check files in tasks/ and specs/ directories
if [[ "$FILE_PATH" != *tasks/* ]] && [[ "$FILE_PATH" != *specs/* ]]; then
  exit 0
fi

# Extract just the filename
FILENAME=$(basename "$FILE_PATH")

# Allowlist: files that don't need skill prefix
ALLOWLIST=(
  "README.md"
  "CHANGELOG.md"
  "MANIFEST.md"
)

for ALLOWED in "${ALLOWLIST[@]}"; do
  if [ "$FILENAME" = "$ALLOWED" ]; then
    exit 0
  fi
done

# Known skill prefixes
SKILL_PREFIXES=(
  "requirements-analyst-"
  "brainstorming-"
  "writing-plans-"
  "architect-"
  "api-designer-"
  "frontend-design-"
  "coder-"
  "coder-frontend-"
  "code-reviewer-"
  "test-generator-"
  "systematic-debugger-"
  "finishing-branch-"
  "documentation-generator-"  # Full skill name
  "docs-generator-"           # Short alias (used by skill output convention)
  "release-"
  "skill-creator-"
  "accelerator-update-"
  "verify-"
)

for PREFIX in "${SKILL_PREFIXES[@]}"; do
  if [[ "$FILENAME" == ${PREFIX}* ]]; then
    exit 0
  fi
done

# File doesn't match any skill prefix - WARN
echo "⚠️  File naming violation: '$FILENAME' in $(dirname "$FILE_PATH")"
echo "   Expected skill-prefixed name like: {skill-name}-{purpose}.md"
echo "   See AGENTS.md for naming rules."
exit 1
