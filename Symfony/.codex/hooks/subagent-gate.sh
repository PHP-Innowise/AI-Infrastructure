#!/bin/bash
# Blocks Codex's built-in multi-agent tools entirely.
# Codex hook event: PreToolUse (self-filters to multi-agent tool calls).
#
# In this edition Codex delegates through skills (`.agents/skills/`) and has
# deliberately no agent wrappers, so every spawn_agent call would reach a
# Codex built-in role (default/worker/explorer). Multi-agent collaboration
# is already disabled in `.codex/config.toml` ([features] multi_agent and
# [agents] enabled); this hook is the version-proof backstop for CLI/model
# combinations that ignore those flags.
#
# Exit codes: 0 = allow, 2 = block.

INPUT=
IFS= read -r -d '' INPUT || :

extract_tool_names() {
  if command -v jq >/dev/null 2>&1; then
    jq -r '[.. | objects | .tool_name? | select(type == "string" and length > 0)] | unique | .[]'
  elif command -v php >/dev/null 2>&1; then
    php -r '$v=json_decode(stream_get_contents(STDIN), true); $found=[]; $walk=function($v) use (&$walk, &$found) { if (!is_array($v)) return; if (isset($v["tool_name"]) && is_string($v["tool_name"])) $found[$v["tool_name"]]=true; foreach ($v as $child) $walk($child); }; $walk($v); echo implode("\n", array_keys($found));'
  elif command -v python3 >/dev/null 2>&1; then
    python3 -c 'import json,sys
found=[]
def walk(v):
    if isinstance(v, dict):
        if isinstance(v.get("tool_name"), str) and v["tool_name"] not in found: found.append(v["tool_name"])
        for child in v.values(): walk(child)
    elif isinstance(v, list):
        for child in v: walk(child)
walk(json.load(sys.stdin))
print("\n".join(found), end="")'
  else
    return 1
  fi
}

TOOL_NAMES=$(printf '%s' "$INPUT" | extract_tool_names 2>/dev/null) || {
  echo "subagent-gate: no JSON extractor (jq/php/python3); failing open" >&2
  exit 0
}
[ -z "$TOOL_NAMES" ] && exit 0

if printf '%s\n' "$TOOL_NAMES" \
  | grep -Eqi '^(spawn_agent|Agent|send_input|resume_agent|wait_agent|close_agent)$'; then
  {
    printf 'BLOCKED: Codex built-in subagents are disabled in this project.\n'
    printf '  Use the skills in .agents/skills/ (see AGENTS.md) or do the\n'
    printf '  work in the main conversation instead of spawning agents.\n'
  } >&2
  exit 2
fi

exit 0
