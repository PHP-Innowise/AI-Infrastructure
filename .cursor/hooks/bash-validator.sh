#!/bin/bash
# Bash Validator Hook
# Blocks destructive commands for Symfony projects.
# Hook type: PreToolUse:Bash
# Exit codes: 0 = pass, 1 = warn (continue), 2 = block

INPUT=$(cat)

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

COMMAND=$(printf '%s' "$INPUT" | extract_command 2>/dev/null) || exit 0

if [ -z "$COMMAND" ]; then
  exit 0
fi

# BLOCKED patterns - hard block (exit 2), truly destructive/irreversible
BLOCKED_PATTERNS=(
  "git[^;&|]*[[:space:]]push[^;&|]*(--force([^[:alnum:]]|$)|-f([[:space:]]|$))"
  "git[[:space:]]+reset[[:space:]]+--hard"
  "git[[:space:]]+clean[[:space:]].*-f"
  "git[[:space:]]+branch[[:space:]]+-D"
  "--no-verify"
  "DROP[[:space:]]+(TABLE|DATABASE)"
  "TRUNCATE[[:space:]]+TABLE"
  "DELETE[[:space:]]+FROM.*WHERE[[:space:]]+1[[:space:]]*=[[:space:]]*1"
  "doctrine:(database|schema):drop"
  "doctrine:migrations:execute.*--down"
  "doctrine:fixtures:load([^[:alnum:]]|$).*--purge"
  "messenger:failed:remove.*--all"
  "composer config.*github-oauth"
  "composer config.*http-basic"
  "rm[[:space:]]+-rf[[:space:]]+(/|~|\.)[[:space:]]*$"
  "gh repo delete"
  "gh repo archive"
  "gh issue delete"
  "gh release delete"
  "gh api.*DELETE"
)

if printf '%s\n' "$COMMAND" | grep -Eqi -- 'doctrine:fixtures:load'; then
  FIXTURE_TAIL=${COMMAND#*doctrine:fixtures:load}
  FIXTURE_CODE=${FIXTURE_TAIL%%#*}
  if ! printf '%s\n' "$FIXTURE_CODE" | grep -Eqi -- "(^|[[:space:]])--append([[:space:]\"']|$)"; then
    echo "BLOCKED: Doctrine fixtures load purges existing data unless append mode is explicit." >&2
    echo "   Command: $COMMAND" >&2
    exit 2
  fi
fi

for PATTERN in "${BLOCKED_PATTERNS[@]}"; do
  if printf '%s\n' "$COMMAND" | grep -Eqi -- "$PATTERN"; then
    echo "BLOCKED: Destructive command detected: matches pattern '$PATTERN'" >&2
    echo "   Command: $COMMAND" >&2
    echo "   This operation is blocked. See AGENTS.md." >&2
    exit 2
  fi
done

exit 0
