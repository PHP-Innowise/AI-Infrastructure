#!/bin/bash
# Bash Validator Hook (Infrastructure-Creator)
# Blocks destructive or unsafe shell commands before they run.
# Hook type: PreToolUse:Bash / beforeShellExecution
# Exit codes: 0 = allow, 2 = block (message on stderr)
#
# Reads the tool input JSON on stdin and extracts the command string.

set -uo pipefail

input="$(cat 2>/dev/null || true)"

# Extract the command field without requiring jq (best-effort).
cmd="$(printf '%s' "$input" | sed -n 's/.*"command"[[:space:]]*:[[:space:]]*"\(.*\)".*/\1/p' | head -1)"
if [ -z "$cmd" ]; then
  cmd="$input"
fi

block() {
  echo "[bash-validator] BLOCKED: $1" >&2
  exit 2
}

# Recursive force-delete of a broad path.
case "$cmd" in
  *"rm -rf /"*|*"rm -rf ~"*|*"rm -rf ."*|*"rm -fr /"*) block "refusing 'rm -rf' on a broad/root path" ;;
esac

# Destructive git operations require explicit user consent, not a hook.
case "$cmd" in
  *"git push --force"*|*"git push -f"*) block "force-push is not permitted from a hook-run command" ;;
  *"git reset --hard"*) block "hard reset can destroy work; run it manually with intent" ;;
  *"--no-verify"*) block "skipping hooks with --no-verify is not permitted" ;;
esac

# Never touch secrets.
case "$cmd" in
  *".env"*) 
    case "$cmd" in
      *"cat "*|*"echo "*|*" > "*|*" >> "*) block "commands must not read or write .env files" ;;
    esac
    ;;
esac

# Dropping database tables/schemas is destructive to a target.
case "$cmd" in
  *"DROP TABLE"*|*"DROP DATABASE"*|*"drop table"*|*"drop database"*) block "destructive SQL (DROP) needs explicit user consent" ;;
esac

exit 0
