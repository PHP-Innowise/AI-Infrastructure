#!/bin/bash
# Bash Validator Hook
# Blocks destructive commands for Laravel projects.
# Hook type: PreToolUse:Bash
# Exit codes: 0 = pass, 1 = warn (continue), 2 = block

# Consume the complete hook payload without relying on external utilities.
# `read -d ''` returns nonzero at EOF, which is the expected delimiter here.
INPUT=
IFS= read -r -d '' INPUT || :

# Cheap self-filter before any process is forked: Codex and Cursor register
# this hook without a tool matcher, so it runs for every tool call. A real
# "command"/"cmd" JSON key always appears unescaped on the wire, while the
# same text inside a string value arrives as \"command\" and does not match,
# so payloads that cannot carry a shell command exit here for free.
case "$INPUT" in
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
  echo "bash-validator: no JSON extractor available (jq/php/python3), validation skipped" >&2
  exit 0
fi

COMMAND=$(printf '%s' "$INPUT" | extract_command 2>/dev/null) || exit 0

if [ -z "$COMMAND" ]; then
  exit 0
fi

# BLOCKED patterns - hard block (exit 2), truly destructive/irreversible
# Artisan rules are anchored to an actual `artisan` invocation: an unanchored
# `migrate.*(fresh|refresh)` also matches prose and read-only searches.
BLOCKED_PATTERNS=(
  "git[^;&|]*[[:space:]]push[^;&|]*(--force([^[:alnum:]]|$)|-f([[:space:]]|$))"
  "git[[:space:]]+reset[[:space:]]+--hard"
  "git[[:space:]]+clean[[:space:]].*-f"
  "git[[:space:]]+branch[[:space:]]+-D"
  "--no-verify"
  "DROP[[:space:]]+(TABLE|DATABASE)"
  "TRUNCATE[[:space:]]+TABLE"
  "DELETE[[:space:]]+FROM.*WHERE[[:space:]]+1[[:space:]]*=[[:space:]]*1"
  "(^|[^[:alnum:]_])artisan[[:space:]]+migrate:(fresh|reset|refresh|rollback)([^[:alnum:]:-]|$)"
  "(^|[^[:alnum:]_])artisan[[:space:]]+db:wipe([^[:alnum:]:-]|$)"
  "(^|[^[:alnum:]_])artisan[[:space:]]+schema:drop([^[:alnum:]:-]|$)"
  "(^|[^[:alnum:]_])artisan[^;&|]*[[:space:]]db:seed[^;&|]*--force"
  "(^|[^[:alnum:]_])artisan[[:space:]]+model:prune"
  "composer config.*github-oauth"
  "composer config.*http-basic"
  "rm[[:space:]]+-rf[[:space:]]+(/|~|\.)[[:space:]]*$"
  "gh repo delete"
  "gh repo archive"
  "gh issue delete"
  "gh release delete"
  "gh api.*DELETE"
)

# One combined grep decides pass/block instead of one grep fork per pattern;
# every alternative keeps its own group so "^" anchors keep their standalone
# meaning. The per-pattern loop runs only after a match, to name the pattern
# in the block message.
BLOCKED_REGEX=""
for PATTERN in "${BLOCKED_PATTERNS[@]}"; do
  BLOCKED_REGEX="${BLOCKED_REGEX:+$BLOCKED_REGEX|}($PATTERN)"
done

if printf '%s\n' "$COMMAND" | grep -Eqi -- "$BLOCKED_REGEX"; then
  echo "BLOCKED: Destructive command detected (rule category: destructive command)." >&2
  echo "   This operation is blocked. See AGENTS.md." >&2
  exit 2
fi

# OUTWARD patterns - not destructive, but they publish outside this checkout:
# a pull request, a comment, a release, a push. They are reversible only in the
# sense that a retraction is itself public, so they need the user's word first.
# Warn rather than block: the user is often the one asking for exactly this,
# and a hook cannot ask. The corresponding rule for the agent is in AGENTS.md.
OUTWARD_PATTERNS=(
  "gh[[:space:]]+pr[[:space:]]+(create|merge|comment|review|close|reopen|ready)"
  "gh[[:space:]]+issue[[:space:]]+(create|comment|close|reopen)"
  "gh[[:space:]]+release[[:space:]]+(create|edit|upload)"
  "gh[[:space:]]+api[^;&|]*(-X|--method)[[:space:]]+(POST|PUT|PATCH)"
  "gh[[:space:]]+workflow[[:space:]]+run"
  "git[[:space:]]+push"
)

OUTWARD_REGEX=""
for PATTERN in "${OUTWARD_PATTERNS[@]}"; do
  OUTWARD_REGEX="${OUTWARD_REGEX:+$OUTWARD_REGEX|}($PATTERN)"
done

if printf '%s\n' "$COMMAND" | grep -Eqi -- "$OUTWARD_REGEX"; then
  echo "CONFIRM: outward-facing action detected (publishes outside this repository)." >&2
  echo "   Run it only with the user's explicit approval for this specific action." >&2
  exit 1
fi

# Repetition guard. The file-edit counterpart lives in loop-detection.sh; a
# command loop is invisible to it, because rerunning one failing command
# forever touches no file. This hook sees the call before it runs and cannot
# see its result, so identical invocations are the only signal available -
# and they are counted per exact command string, so any real change of
# approach starts its own count. The counter directory is the one the session
# start hook clears, which makes the window a session.
REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
REPO_KEY=$(printf '%s' "$REPO_ROOT" | cksum | cut -d' ' -f1)
TRACK_DIR="/tmp/claude-loop-detection-$REPO_KEY"

if mkdir -p "$TRACK_DIR" 2>/dev/null; then
  if command -v md5sum > /dev/null 2>&1; then
    COMMAND_KEY=$(printf '%s' "$COMMAND" | md5sum | cut -d' ' -f1)
  elif command -v md5 > /dev/null 2>&1; then
    COMMAND_KEY=$(printf '%s' "$COMMAND" | md5 -q)
  else
    COMMAND_KEY=$(printf '%s' "$COMMAND" | cksum | tr -d ' ')
  fi
  TRACK_FILE="$TRACK_DIR/cmd-$COMMAND_KEY"

  COUNT=0
  [ -f "$TRACK_FILE" ] && COUNT=$(cat "$TRACK_FILE" 2>/dev/null)
  case "$COUNT" in *[!0-9]*|'') COUNT=0 ;; esac
  COUNT=$((COUNT + 1))
  echo "$COUNT" > "$TRACK_FILE" 2>/dev/null

  if [ "$COUNT" -ge 12 ]; then
    {
      printf 'BLOCKED: this exact command has run %s times this session.\n' "$COUNT"
      printf '   Repeating it again is not a new attempt. Either change the\n'
      printf '   command (narrow it, add the failing case, read the output\n'
      printf '   differently) or escalate to /debugger for a root cause.\n'
    } >&2
    exit 2
  elif [ "$COUNT" -ge 6 ]; then
    printf 'WARNING: this exact command has run %s times this session. If it keeps failing, /debugger instead of another rerun.\n' "$COUNT" >&2
    exit 1
  fi
fi

exit 0
