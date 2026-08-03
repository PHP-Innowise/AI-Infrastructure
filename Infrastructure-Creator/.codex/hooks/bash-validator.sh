#!/bin/bash
# Bash Validator Hook (Infrastructure-Creator)
# Blocks destructive or unsafe shell commands before they run.
# Hook type: PreToolUse:Bash / beforeShellExecution
# Exit codes: 0 = allow, 2 = block (message on stderr)
#
# Reads the tool input JSON on stdin and extracts the command string.

set -uo pipefail

input="$(cat 2>/dev/null || true)"

# Cheap self-filter before any process is forked: Codex and Cursor register
# this hook without a tool matcher, so it runs for every tool call. A real
# "command"/"cmd" JSON key always appears unescaped on the wire, while the
# same text inside a string value arrives as \"command\" and does not match,
# so payloads that cannot carry a shell command exit here for free.
case "$input" in
  *'"command"'*|*'"cmd"'*) ;;
  *) exit 0 ;;
esac

# Decode JSON instead of scraping quoted strings; nested shell commands contain escaped quotes.
extract_command() {
  if command -v jq >/dev/null 2>&1; then
    jq -r '[.. | objects | .command?, .cmd? | select(type == "string" and length > 0)] | join("\n")'
  elif command -v php >/dev/null 2>&1; then
    php -r '$v=json_decode(stream_get_contents(STDIN), true); $found=[]; $find=function($v) use (&$find, &$found) { if (!is_array($v)) return; foreach (["command", "cmd"] as $k) if (isset($v[$k]) && is_string($v[$k]) && $v[$k] !== "") $found[]=$v[$k]; foreach ($v as $child) $find($child); }; $find($v); echo implode("\n", $found);'
  elif command -v python3 >/dev/null 2>&1; then
    python3 -c 'import json,sys
found=[]
def find(v):
    if isinstance(v, dict):
        for key in ("command", "cmd"):
            if isinstance(v.get(key), str) and v[key]: found.append(v[key])
        for child in v.values():
            find(child)
    elif isinstance(v, list):
        for child in v:
            find(child)
find(json.load(sys.stdin))
print("\n".join(found), end="")'
  else
    return 1
  fi
}

if ! command -v jq >/dev/null 2>&1 && ! command -v php >/dev/null 2>&1 && ! command -v python3 >/dev/null 2>&1; then
  echo "[bash-validator] no JSON extractor available (jq/php/python3), validation skipped" >&2
  exit 0
fi

cmd="$(printf '%s' "$input" | extract_command 2>/dev/null)" || exit 0
if [ -z "$cmd" ]; then
  exit 0
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

# Irreversible GitHub operations. This generator only ever reads a target
# project, so nothing it does legitimately requires destroying a remote
# repository, issue, or release.
case "$cmd" in
  *"gh repo delete"*|*"gh repo archive"*) block "destroying or archiving a GitHub repository needs explicit user consent" ;;
  *"gh issue delete"*|*"gh release delete"*) block "deleting GitHub issues or releases needs explicit user consent" ;;
esac
case "$cmd" in
  *"gh api"*)
    case "$cmd" in
      *DELETE*|*delete*) block "gh api DELETE needs explicit user consent" ;;
    esac
    ;;
esac

exit 0
