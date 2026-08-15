#!/bin/bash
# Subagent completion observer for orchestrated flows.
# Hook event: SubagentStop (Claude Code) / subagentStop (Cursor); the Codex
# mirror exists for parity but stays unregistered while multi-agent is off.
# Exit codes: always 0 (observation must never break a turn).
#
# When a subagent finishes, append a `completion` entry to the task's agent
# channel (msg-dispatch) so the dispatch journal records the finish without
# relying on the orchestrating conversation to remember, and release the
# write-agent lock the subagent gate took for a write-capable agent. Both
# steps degrade to no-ops without python3, without the context runtime, or
# without a resolvable task.

set -u

INPUT=
IFS= read -r -d '' INPUT || :

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
CONTEXT_CLI="$ROOT_DIR/memory-bank/scripts/context.py"
BUDGET_SECONDS="${CONTEXT_HOOK_BUDGET:-5}"

extract_string() {  # $1 = key; first string value found walking the payload
  key=$1
  if command -v python3 > /dev/null 2>&1; then
    # Structure-aware, top-down: a key quoted inside message TEXT can never
    # shadow the real payload field.
    printf '%s' "$INPUT" | python3 -c 'import json, sys
key = sys.argv[1]
try:
    payload = json.load(sys.stdin)
except Exception:
    sys.exit(0)
def walk(value):
    if isinstance(value, dict):
        if isinstance(value.get(key), str) and value[key]:
            return value[key]
        for child in value.values():
            found = walk(child)
            if found:
                return found
    elif isinstance(value, list):
        for child in value:
            found = walk(child)
            if found:
                return found
    return ""
print(walk(payload), end="")' "$key" 2>/dev/null
  else
    # Degraded fallback (lock release only - the journal write below needs
    # python3 anyway): greedy, may match a quoted key inside message text.
    printf '%s' "$INPUT" \
      | sed -n 's/.*"'"$key"'"[[:space:]]*:[[:space:]]*"\(\(\\.\|[^"\\]\)*\)".*/\1/p' \
      | head -n 1
  fi
}

AGENT=$(extract_string agent_type)
[ -n "$AGENT" ] || AGENT=$(extract_string subagent_type)
[ -z "$AGENT" ] && exit 0

# Release the write-agent lock this agent holds, whichever host took it.
LOCK_ROOT=$(git -C "$ROOT_DIR" rev-parse --show-toplevel 2>/dev/null || printf '%s' "$ROOT_DIR")
LOCK_KEY=$(printf '%s' "$LOCK_ROOT" | cksum | cut -d' ' -f1)
for lock in "/tmp/claude-write-agent-lock-$LOCK_KEY" "/tmp/cursor-write-agent-lock-$LOCK_KEY"; do
  if [ -f "$lock" ] && [ "$(cat "$lock" 2>/dev/null)" = "$AGENT" ]; then
    rm -f "$lock" 2>/dev/null
  fi
done

command -v python3 > /dev/null 2>&1 || exit 0
[ -f "$CONTEXT_CLI" ] || exit 0

TASK_ID="${CONTEXT_TASK_ID:-$(git -C "$ROOT_DIR" branch --show-current 2>/dev/null)}"
[ -n "$TASK_ID" ] || exit 0

# One sanitized line: the final assistant message (Claude) or the reported
# status (Cursor); msg-dispatch itself falls back to "complete: <agent>".
NOTE=$(extract_string last_assistant_message)
[ -n "$NOTE" ] || NOTE=$(extract_string status)
NOTE=$(printf '%s' "$NOTE" | tr '\n\t' '  ' | sed 's/\\[nt]/ /g; s/  */ /g; s/^ //; s/ $//' | cut -c1-160)

if command -v timeout > /dev/null 2>&1; then
  DISPATCH_ERROR=$(timeout "$BUDGET_SECONDS" python3 "$CONTEXT_CLI" --root "$ROOT_DIR" \
    msg-dispatch --task-id "$TASK_ID" --agent "$AGENT" --event complete \
    ${NOTE:+--note "$NOTE"} 2>&1 > /dev/null)
else
  DISPATCH_ERROR=$(python3 "$CONTEXT_CLI" --root "$ROOT_DIR" \
    msg-dispatch --task-id "$TASK_ID" --agent "$AGENT" --event complete \
    ${NOTE:+--note "$NOTE"} 2>&1 > /dev/null)
fi
DISPATCH_STATUS=$?

# A journal that fails silently is worse than one that does not exist: the
# orchestrator is told completions are recorded automatically, so a lost
# write leaves it reading an incomplete channel as a complete one. Report and
# still exit 0 - observation must never break a turn.
case "$DISPATCH_ERROR" in
  # No task for this branch means no channel and no orchestrator reading it:
  # nothing was lost, so there is nothing to report. Every other failure is a
  # completion the channel should have held and does not.
  *"task not found"*) DISPATCH_STATUS=0 ;;
esac

if [ "$DISPATCH_STATUS" -ne 0 ]; then
  DETAIL=$(printf '%s' "$DISPATCH_ERROR" | tr '\n\t' '  ' | cut -c1-160)
  MESSAGE="subagent-dispatch: completion of \"$AGENT\" was NOT recorded in the \
channel (exit $DISPATCH_STATUS${DETAIL:+ — $DETAIL}). Record it with \
\`context.py msg-dispatch --event complete\` before trusting msg-read."
  echo "$MESSAGE"
  echo "$MESSAGE" >&2
fi

exit 0
