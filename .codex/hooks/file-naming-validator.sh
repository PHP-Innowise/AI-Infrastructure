#!/bin/bash
# Enforces task/spec Markdown naming and zero-padded task directories.
# Codex hook event: PreToolUse (self-filters to file-edit payloads).
# Exit codes: 0 = allow, 2 = block.

INPUT=$(cat)

extract_paths() {
  if command -v jq >/dev/null 2>&1; then
    jq -r '[
      (.. | objects | .file_path?, .path? | select(type == "string" and length > 0)),
      (.. | strings | scan("\\*\\*\\* (?:Add File|Update File|Move to): ([^\\r\\n]+)") | .[0])
    ] | unique | .[]'
  elif command -v php >/dev/null 2>&1; then
    php -r '$v=json_decode(stream_get_contents(STDIN), true); $found=[]; $walk=function($v) use (&$walk, &$found) { if (is_string($v)) { preg_match_all("/\\*\\*\\* (?:Add File|Update File|Move to): ([^\\r\\n]+)/", $v, $m); foreach ($m[1] as $path) $found[$path]=true; return; } if (!is_array($v)) return; foreach (["file_path", "path"] as $k) if (isset($v[$k]) && is_string($v[$k]) && $v[$k] !== "") $found[$v[$k]]=true; foreach ($v as $child) $walk($child); }; $walk($v); echo implode("\n", array_keys($found));'
  elif command -v python3 >/dev/null 2>&1; then
    python3 -c 'import json,re,sys
found=[]
def walk(v):
    if isinstance(v, str):
        for path in re.findall(r"\*\*\* (?:Add File|Update File|Move to): ([^\r\n]+)", v):
            if path not in found: found.append(path)
        return
    if isinstance(v, dict):
        for key in ("file_path", "path"):
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

TOOL_NAMES=$(printf '%s' "$INPUT" | extract_tool_names 2>/dev/null) || exit 0
if [ -n "$TOOL_NAMES" ] \
  && ! printf '%s\n' "$TOOL_NAMES" | grep -Eqi '(^|[.:/])(write|edit|multiedit|multi_edit|notebookedit|notebook_edit|apply_patch)$'; then
  exit 0
fi

FILE_PATHS=$(printf '%s' "$INPUT" | extract_paths 2>/dev/null) || exit 0
[ -z "$FILE_PATHS" ] && exit 0

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
SKILLS_DIR="$ROOT_DIR/.agents/skills"

block() {
  printf 'BLOCKED: %s\n' "$1" >&2
  printf '  See AGENTS.md for repository file-naming rules.\n' >&2
  return 2
}

validate_path() {
  local file_path=$1
  local normalized filename relative task_dir skill_dir prefix

  [[ "$file_path" == *.md ]] || return 0
  if [[ "$file_path" == "$ROOT_DIR/"* ]]; then
    normalized=${file_path#"$ROOT_DIR/"}
  else
    normalized=${file_path#./}
  fi
  filename=$(basename "$normalized")

  if [[ "$normalized" == memory-bank/chunks/* ]]; then
    if [[ "$(dirname "$normalized")" != "memory-bank/chunks" ]]; then
      block "Memory chunks must be stored directly in memory-bank/chunks/: '$file_path'"
      return 2
    fi
    if [[ ! "$filename" =~ ^MEM-[0-9]{4,}-[a-z0-9]+(-[a-z0-9]+)*\.md$ ]]; then
      block "Memory chunks must use memory-bank/chunks/MEM-0001-short-slug.md: '$file_path'"
      return 2
    fi
    return 0
  fi

  if [[ "$normalized" == tasks/* ]]; then
    relative=${normalized#tasks/}

    if [[ "$relative" == */* ]]; then
      task_dir=${relative%%/*}
      if [[ ! "$task_dir" =~ ^TASK-[0-9]{3,}$ ]]; then
        block "Task Markdown must be inside a zero-padded directory such as tasks/TASK-001/: '$file_path'"
        return 2
      fi
    elif [[ "$filename" != "README.md" && "$filename" != "CHANGELOG.md" ]]; then
      block "Task Markdown must be inside tasks/TASK-N/: '$file_path'"
      return 2
    fi
  elif [[ "$normalized" != specs/* ]]; then
    return 0
  fi

  case "$filename" in
    README.md|CHANGELOG.md|MANIFEST.md) return 0 ;;
  esac

  while IFS= read -r skill_dir; do
    prefix="$(basename "$skill_dir")-"
    [[ "$filename" == "$prefix"* ]] && return 0
  done < <(find "$SKILLS_DIR" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort)

  block "'$filename' must start with a discovered skill name followed by '-'"
  return 2
}

while IFS= read -r FILE_PATH; do
  [ -z "$FILE_PATH" ] && continue
  validate_path "$FILE_PATH" || exit $?
done <<< "$FILE_PATHS"

exit 0
