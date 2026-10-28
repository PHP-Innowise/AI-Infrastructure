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

# Python 3.9+ by whichever name this shell knows it. Native Windows (Git Bash)
# often has no working python3: the python.org installer provides python and
# py, and the python3 found there may be the Microsoft Store stub, which only
# offers to install Python. Elsewhere python3 is taken without a probe.
resolve_python() {
  PYTHON=()
  if [ "${OS:-}" != "Windows_NT" ] && command -v python3 > /dev/null 2>&1; then
    PYTHON=(python3)
    return 0
  fi
  # On Windows the name found is remembered per user, and taken again while
  # the shell finds it at the same path (the hash table, no process): every
  # hook otherwise started an interpreter just to ask its version.
  local name path remembered="${TMPDIR:-/tmp}/ai-accelerator-python-${UID:-0}"
  if [ "${OS:-}" = "Windows_NT" ] && [ -f "$remembered" ] && [ ! -L "$remembered" ] \
      && IFS=$'\t' read -r name path < "$remembered" && [ -n "$path" ]; then
    case "$name" in
      python|py|python3)
        if hash "$name" 2> /dev/null && [ "${BASH_CMDS[$name]:-}" = "$path" ]; then
          PYTHON=("$name")
          [ "$name" = py ] && PYTHON+=(-3)
          return 0
        fi
        ;;
    esac
  fi
  for name in python py python3; do
    PYTHON=("$name")
    [ "$name" = py ] && PYTHON+=(-3)
    if "${PYTHON[@]}" -I -S -c 'import sys; sys.exit(sys.version_info < (3, 9))' > /dev/null 2>&1; then
      if [ "${OS:-}" = "Windows_NT" ] && [ ! -L "$remembered" ] && hash "$name" 2> /dev/null; then
        printf '%s\t%s\n' "$name" "${BASH_CMDS[$name]:-}" > "$remembered" 2> /dev/null
      fi
      return 0
    fi
  done
  PYTHON=()
  return 1
}

extract_tool_names() {
  if command -v jq >/dev/null 2>&1; then
    jq -r '[.. | objects | .tool_name? | select(type == "string" and length > 0)] | unique | .[]'
  elif command -v php >/dev/null 2>&1; then
    php -r '$v=json_decode(stream_get_contents(STDIN), true); $found=[]; $walk=function($v) use (&$walk, &$found) { if (!is_array($v)) return; if (isset($v["tool_name"]) && is_string($v["tool_name"])) $found[$v["tool_name"]]=true; foreach ($v as $child) $walk($child); }; $walk($v); echo implode("\n", array_keys($found));'
  elif [ -n "${PYTHON[0]:-}" ]; then
    # Bytes both ways: on Windows a text stream reads by the ANSI code page,
    # which a Russian word does not fit, and writes \n as \r\n. Isolated
    # (-I): run in the session's folder, Python imported a project's own
    # json.py before the standard one.
    "${PYTHON[@]}" -I -c 'import json,sys
found=[]
def walk(v):
    if isinstance(v, dict):
        if isinstance(v.get("tool_name"), str) and v["tool_name"] not in found: found.append(v["tool_name"])
        for child in v.values(): walk(child)
    elif isinstance(v, list):
        for child in v: walk(child)
walk(json.loads(sys.stdin.buffer.read()))
sys.stdout.buffer.write("\n".join(found).encode("utf-8", "replace"))'
  else
    return 1
  fi
}

# Python only where neither jq nor php is: found once, here.
command -v jq >/dev/null 2>&1 || command -v php >/dev/null 2>&1 || resolve_python
TOOL_NAMES=$(printf '%s' "$INPUT" | extract_tool_names 2>/dev/null) || {
  echo "subagent-gate: no JSON extractor (jq/php/Python 3.9+); failing open" >&2
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
