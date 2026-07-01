#!/bin/bash
# Bash Validator Hook
# Blocks destructive commands for Laravel/PHP projects.
# Hook type: PreToolUse:Bash
# Exit codes: 0 = pass, 1 = warn (continue), 2 = block

INPUT=$(cat)

# Extract command from JSON input (POSIX-compatible, no grep -P)
COMMAND=$(echo "$INPUT" | sed -n 's/.*"command"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)

if [ -z "$COMMAND" ]; then
  exit 0
fi

# BLOCKED patterns - hard block (exit 2), truly destructive/irreversible
BLOCKED_PATTERNS=(
  "git push.*--force"
  "git push.*-f"
  "git reset --hard"
  "git clean -f"
  "DROP TABLE"
  "DROP DATABASE"
  "TRUNCATE TABLE"
  "DELETE FROM .*WHERE 1=1"
  "php artisan migrate:fresh"
  "php artisan migrate:reset"
  "php artisan db:wipe"
  "php artisan schema:dump.*--prune"
  "composer config.*github-oauth"
  "composer config.*http-basic"
  "composer publish"
  "rm -rf /"
  "rm -rf ~"
  "rm -rf \."
  "git branch -D"
  "--no-verify"
  "gh repo delete"
  "gh repo archive"
  "gh issue delete"
  "gh release delete"
  "gh api.*DELETE"
)

for PATTERN in "${BLOCKED_PATTERNS[@]}"; do
  if echo "$COMMAND" | grep -Eqi "$PATTERN"; then
    echo "BLOCKED: Destructive command detected: matches pattern '$PATTERN'"
    echo "   Command: $COMMAND"
    echo "   This operation is blocked. See AGENTS.md."
    exit 2
  fi
done

exit 0
