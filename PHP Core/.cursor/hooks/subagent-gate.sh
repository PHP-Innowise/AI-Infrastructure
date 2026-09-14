#!/bin/bash
# Restricts subagent spawning to the accelerator's own agent roster.
# Cursor hook event: subagentStart.
#
# Cursor has no setting that disables its built-in subagents (explore,
# shell/bash, browser, generalPurpose), so this hook is the enforcement
# point: it allows only subagent types whose definition exists in
# `.cursor/agents/` (frontmatter `name:`) and denies everything else.
#
# Contract: JSON on stdout, always exit 0.
#   {"permission": "allow"} | {"permission": "deny", "user_message": "..."}
# ("ask" is treated as deny by Cursor for this event.)
#
# Known host caveat: before Cursor 3.4 the payload reported every subagent
# as "general-purpose", which would deny project agents too. On such
# versions rely on .cursor/rules/subagent-policy.mdc and temporarily remove
# this hook from .cursor/hooks.json.

INPUT=
IFS= read -r -d '' INPUT || :

extract_sub_type() {
  if command -v jq >/dev/null 2>&1; then
    jq -r '[.. | objects | .subagent_type? | select(type == "string" and length > 0)] | unique | .[0] // empty'
  elif command -v php >/dev/null 2>&1; then
    php -r '$v=json_decode(stream_get_contents(STDIN), true); $found=[]; $walk=function($v) use (&$walk, &$found) { if (!is_array($v)) return; if (isset($v["subagent_type"]) && is_string($v["subagent_type"]) && $v["subagent_type"] !== "") $found[$v["subagent_type"]]=true; foreach ($v as $child) $walk($child); }; $walk($v); echo array_key_first($found) ?? "";'
  elif command -v python3 >/dev/null 2>&1; then
    python3 -c 'import json,sys
found=[]
def walk(v):
    if isinstance(v, dict):
        if isinstance(v.get("subagent_type"), str) and v["subagent_type"] and v["subagent_type"] not in found: found.append(v["subagent_type"])
        for child in v.values(): walk(child)
    elif isinstance(v, list):
        for child in v: walk(child)
walk(json.load(sys.stdin))
print(found[0] if found else "", end="")'
  else
    return 1
  fi
}

allow() { printf '{"permission":"allow"}\n'; exit 0; }

SUB_TYPE=$(printf '%s' "$INPUT" | extract_sub_type 2>/dev/null) || {
  echo "subagent-gate: no JSON extractor (jq/php/python3); failing open" >&2
  allow
}
# No type in the payload: nothing to judge; do not brick delegation.
[ -z "$SUB_TYPE" ] && allow

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
AGENTS_DIR="$ROOT_DIR/.cursor/agents"

# `writes: true` frontmatter marks a write-capable agent; those run one at a
# time, serialized by a TTL lock cleared by subagent-dispatch.sh.
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

[ -z "$ROSTER" ] && allow

if printf '%s' "$ROSTER" | grep -qxF -- "$SUB_TYPE"; then
  [ "$WRITES" = "true" ] || allow
  LOCK_ROOT=$(git -C "$ROOT_DIR" rev-parse --show-toplevel 2>/dev/null \
    || printf '%s' "$ROOT_DIR")
  LOCK_KEY=$(printf '%s' "$LOCK_ROOT" | cksum | cut -d' ' -f1)
  LOCK_DIR="${SUBAGENT_WRITE_LOCK_DIR:-/tmp}"
  case "$LOCK_DIR" in
    /*) ;;
    *)
      printf '{"permission":"deny","user_message":"Write-agent lock directory configuration is invalid."}\n'
      exit 0
      ;;
  esac
  if [ ! -d "$LOCK_DIR" ] || [ ! -w "$LOCK_DIR" ] || [ ! -x "$LOCK_DIR" ]; then
    printf '{"permission":"deny","user_message":"Write-agent lock directory configuration is invalid."}\n'
    exit 0
  fi
  LOCK_FILE="$LOCK_DIR/cursor-write-agent-lock-$LOCK_KEY"
  LOCK_TTL_MINUTES="${SUBAGENT_WRITE_LOCK_TTL_MINUTES:-30}"
  case "$LOCK_TTL_MINUTES" in
    *[!0-9]*|'') LOCK_TTL_MINUTES=30 ;;  # a bogus TTL must not become "never expires"
  esac
  # Serialize the check-and-take below: Cursor issues parallel spawns as
  # multiple Task calls in one message, so two write-capable agents would
  # otherwise both read an unlocked state. The guard descriptor is released
  # when this script exits, immediately after the decision; without flock, or
  # without a writable guard path, the window remains and the gate degrades to
  # its previous behavior. The appendability probe runs first because a
  # failing `exec` redirection ends a non-interactive shell — and no
  # redirection may be attached to `exec` itself, which would apply it to the
  # whole script.
  if command -v flock > /dev/null 2>&1 && : >> "$LOCK_FILE.guard" 2>/dev/null; then
    exec 9>>"$LOCK_FILE.guard"
    flock 9 2>/dev/null || :
  fi
  if [ -f "$LOCK_FILE" ] \
    && [ -z "$(find "$LOCK_FILE" -mmin +"$LOCK_TTL_MINUTES" 2>/dev/null)" ]; then
    HOLDER=$(cat "$LOCK_FILE" 2>/dev/null)
    # Any live holder blocks, including another instance of the same agent
    # type. Exempting the same name let N concurrent `coder` runs all pass,
    # each merely refreshing the lock. A fresh lock means the holder is still
    # running — subagent-dispatch.sh clears it on completion — so a same-named
    # spawn is a second writer, not a respawn. A crashed run is covered by
    # LOCK_TTL_MINUTES.
    if [ -n "$HOLDER" ]; then
      SAFE_HOLDER=$(printf '%s' "$HOLDER" | tr -cd 'A-Za-z0-9 _.:/-' | cut -c1-64)
      printf '{"permission":"deny","user_message":"Write-capable agent \\"%s\\" is already running; write-capable agents run one at a time. Wait for its completion or remove the stale write-agent lock."}\n' "$SAFE_HOLDER"
      exit 0
    fi
  fi
  printf '%s' "$SUB_TYPE" > "$LOCK_FILE" 2>/dev/null
  allow
fi

# Keep the interpolated type JSON-safe.
SAFE_TYPE=$(printf '%s' "$SUB_TYPE" | tr -cd 'A-Za-z0-9 _.:/-' | cut -c1-64)
printf '{"permission":"deny","user_message":"Blocked subagent \\"%s\\": built-in Cursor subagents are disabled in this project. Delegate via the project agents in .cursor/agents/ (invoke with /agent-name) or work in the main conversation."}\n' "$SAFE_TYPE"
exit 0
