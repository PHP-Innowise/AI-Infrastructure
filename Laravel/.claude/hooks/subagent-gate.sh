#!/bin/bash
# Restricts subagent spawning to the accelerator's own agent roster.
# Claude hook event: PreToolUse (Agent|Task).
#
# Allows only subagent types whose definition exists in `.claude/agents/`
# (frontmatter `name:`). Claude Code's built-in agents (Explore, Plan,
# general-purpose, claude, ...) are denied, so every delegation goes through
# the accelerator's command -> agent -> skill pipeline. Works together with
# the `Agent(...)` deny rules and CLAUDE_CODE_DISABLE_EXPLORE_PLAN_AGENTS in
# `.claude/settings.json`: the hook is the version-proof layer that also
# covers built-in agents added after the deny list was written.
#
# Exit codes: 0 = allow, 2 = block.

# Consume the complete hook payload without relying on external utilities.
# `read -d ''` returns nonzero at EOF, which is the expected delimiter here.
INPUT=
IFS= read -r -d '' INPUT || :

# Recursively collect every string value stored under the given key, so the
# check is robust to payload nesting differences between harness versions.
extract_key() {
  local key=$1
  if command -v jq >/dev/null 2>&1; then
    jq -r --arg key "$key" \
      '[.. | objects | .[$key]? | select(type == "string" and length > 0)] | unique | .[]'
  elif command -v php >/dev/null 2>&1; then
    KEY="$key" php -r '$key=getenv("KEY"); $v=json_decode(stream_get_contents(STDIN), true); $found=[]; $walk=function($v) use (&$walk, &$found, $key) { if (!is_array($v)) return; if (isset($v[$key]) && is_string($v[$key]) && $v[$key] !== "") $found[$v[$key]]=true; foreach ($v as $child) $walk($child); }; $walk($v); echo implode("\n", array_keys($found));'
  elif command -v python3 >/dev/null 2>&1; then
    KEY="$key" python3 -c 'import json,os,sys
key=os.environ["KEY"]
found=[]
def walk(v):
    if isinstance(v, dict):
        if isinstance(v.get(key), str) and v[key] and v[key] not in found: found.append(v[key])
        for child in v.values(): walk(child)
    elif isinstance(v, list):
        for child in v: walk(child)
walk(json.load(sys.stdin))
print("\n".join(found), end="")'
  else
    return 1
  fi
}

TOOL_NAMES=$(printf '%s' "$INPUT" | extract_key tool_name 2>/dev/null) || {
  echo "subagent-gate: no JSON extractor (jq/php/python3); failing open" >&2
  exit 0
}
# Self-filter: act only on the subagent-spawning tool (Agent, formerly Task).
if [ -n "$TOOL_NAMES" ] \
  && ! printf '%s\n' "$TOOL_NAMES" | grep -Eqi '(^|[.:/])(agent|task)$'; then
  exit 0
fi

SUB_TYPE=$(printf '%s' "$INPUT" | extract_key subagent_type 2>/dev/null | head -n 1) || exit 0
# No subagent type in the payload: nothing to judge, let permission rules act.
[ -z "$SUB_TYPE" ] && exit 0

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
AGENTS_DIR="$ROOT_DIR/.claude/agents"

# Roster = every frontmatter `name:` in the accelerator's agent definitions.
# `writes: true` marks a write-capable agent (its skill edits repository
# files); those run one at a time, serialized by a TTL lock below.
# Only the FIRST frontmatter block is metadata. A sed line range would
# restart at every later `---` pair, so a horizontal rule in an agent's body
# could declare `writes:` and change the gate's decision.
first_frontmatter() {
  awk 'NR==1 && $0!="---" {exit} NR==1 {next} $0=="---" {exit} {print}' "$1"
}

ROSTER=""
WRITES="false"
for agent_file in "$AGENTS_DIR"/*.md; do
  [ -f "$agent_file" ] || continue
  case "${agent_file##*/}" in README.md) continue ;; esac
  front=$(first_frontmatter "$agent_file")
  name=$(printf '%s\n' "$front" | sed -n 's/^name:[[:space:]]*//p' \
    | head -n 1 | tr -d '"' | sed 's/[[:space:]]*$//')
  [ -n "$name" ] || continue
  ROSTER="$ROSTER$name
"
  if [ "$name" = "$SUB_TYPE" ]; then
    flag=$(printf '%s\n' "$front" | sed -n 's/^writes:[[:space:]]*//p' \
      | head -n 1 | tr -d '"' | sed 's/[[:space:]]*$//')
    [ "$flag" = "true" ] && WRITES="true"
  fi
done

# An unreadable roster must not brick delegation entirely.
[ -z "$ROSTER" ] && exit 0

if printf '%s' "$ROSTER" | grep -qxF -- "$SUB_TYPE"; then
  [ "$WRITES" = "true" ] || exit 0
  # Serialize write-capable agents: one at a time per repository. The lock
  # is cleared by subagent-dispatch.sh when the holder finishes and expires
  # by TTL, so a crashed run cannot deadlock delegation.
  LOCK_ROOT=$(git -C "$ROOT_DIR" rev-parse --show-toplevel 2>/dev/null \
    || printf '%s' "$ROOT_DIR")
  LOCK_KEY=$(printf '%s' "$LOCK_ROOT" | cksum | cut -d' ' -f1)
  LOCK_FILE="/tmp/claude-write-agent-lock-$LOCK_KEY"
  LOCK_TTL_MINUTES="${SUBAGENT_WRITE_LOCK_TTL_MINUTES:-30}"
  case "$LOCK_TTL_MINUTES" in
    *[!0-9]*|'') LOCK_TTL_MINUTES=30 ;;  # a bogus TTL must not become "never expires"
  esac
  # Serialize the check-and-take below: two write-capable agents spawned in
  # one message would otherwise both read an unlocked state and both proceed.
  # The guard descriptor is released when this script exits, which happens
  # immediately after the decision. Without flock, or without a writable
  # guard path, the window remains and the gate degrades to its previous
  # behavior rather than failing closed. The appendability probe runs first
  # because a failing `exec` redirection ends a non-interactive shell — and
  # no redirection may be attached to `exec` itself, which would apply it to
  # the whole script (silencing the block message below).
  if command -v flock > /dev/null 2>&1 && : >> "$LOCK_FILE.guard" 2>/dev/null; then
    exec 9>>"$LOCK_FILE.guard"
    flock 9 2>/dev/null || :
  fi
  if [ -f "$LOCK_FILE" ] \
    && [ -z "$(find "$LOCK_FILE" -mmin +"$LOCK_TTL_MINUTES" 2>/dev/null)" ]; then
    HOLDER=$(cat "$LOCK_FILE" 2>/dev/null)
    if [ -n "$HOLDER" ] && [ "$HOLDER" != "$SUB_TYPE" ]; then
      {
        printf 'BLOCKED: write-capable agent "%s" is already running.\n' "$HOLDER"
        printf '  Write-capable agents run one at a time. Wait for it to\n'
        printf '  finish (its completion clears the lock), or remove a stale\n'
        printf '  lock: %s\n' "$LOCK_FILE"
      } >&2
      exit 2
    fi
  fi
  printf '%s' "$SUB_TYPE" > "$LOCK_FILE" 2>/dev/null
  exit 0
fi

{
  printf 'BLOCKED: subagent type "%s" is not part of this accelerator.\n' "$SUB_TYPE"
  printf '  Delegate only to the project agents defined in .claude/agents/\n'
  printf '  (see its README for the roster), or do the work in the main\n'
  printf '  conversation instead of spawning a built-in agent.\n'
} >&2
exit 2
