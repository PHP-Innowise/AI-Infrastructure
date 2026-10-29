#!/bin/bash
# Local Context Session Start Hook
# Scans a native PHP working directory at session start.
# Hook type: SessionStart
# Exit codes: always 0 (informational only)

# Internal draft recovery is part of the current turn, not a new memory session.
[ "${CONTEXT_MEMORY_RECOVERY:-}" = "1" ] && exit 0

echo "Project Context"
echo "==============="

# Installation version: the edition's VERSION file names its latest released
# changelog section; shared-core history lives in the repository-root
# CHANGELOG.md.
EDITION_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
ROOT_DIR=$EDITION_DIR
# Installed, the accelerator, the project and its state are all ROOT_DIR.
# Attached - a launcher lends this clone's edition to a project and names it
# in ACCELERATOR_HOME - the project and the state directory are the
# launcher's, so nothing is written into the clone or the project.
PROJECT_DIR=$ROOT_DIR
STATE_DIR=$ROOT_DIR
accelerator_absolute() { case "$1" in /*|[A-Za-z]:[\\/]*) return 0 ;; esac; return 1; }
if accelerator_absolute "${ACCELERATOR_STATE_DIR:-}" && accelerator_absolute "${ACCELERATOR_PROJECT_DIR:-}" \
  && ROOT_REAL=$(cd "$ROOT_DIR" && pwd -P) \
  && [ "$(cd "${ACCELERATOR_HOME:-/nonexistent}" 2>/dev/null && pwd -P)" = "$ROOT_REAL" ]; then
  PROJECT_DIR=$ACCELERATOR_PROJECT_DIR
  STATE_DIR=$ACCELERATOR_STATE_DIR
  # A state that is this installation, however it is spelled (C:\... where
  # Git Bash says /c/..., a trailing separator), is a worktree's, not attached.
  [ "$(cd "$STATE_DIR" 2>/dev/null && pwd -P)" = "$ROOT_REAL" ] && STATE_DIR=$ROOT_DIR
fi
# The hook's input, read before the banner: a session that entered a worktree
# of this repository - Claude Code's EnterWorktree - starts again there after
# /clear or a compaction, still with these hooks, and names the worktree as its
# "cwd". A worktree that has none of this installation is described and served
# as the dispatcher serves a session started in it; the other hooks do the same.
LOOP_INPUT=
[ -t 0 ] || IFS= read -r -d '' LOOP_INPUT || :
SESSION_CWD=
[[ $LOOP_INPUT =~ \"cwd\"[[:space:]]*:[[:space:]]*\"([^\"]*)\" ]] && SESSION_CWD=${BASH_REMATCH[1]//\\\\/\\}
checkout_identity() {
  local directory=$1 output top common
  output=$(git -C "$directory" rev-parse --show-toplevel --git-common-dir 2>/dev/null) || return 1
  case "$output" in *$'\n'*) ;; *) return 1 ;; esac
  top=${output%%$'\n'*}
  common=${output#*$'\n'}
  case "$common" in /*|[A-Za-z]:[\\/]*) ;; *) common="$directory/$common" ;; esac
  top=$(cd -- "$top" 2>/dev/null && pwd -P) || return 1
  common=$(cd -- "$common" 2>/dev/null && pwd -P) || return 1
  printf '%s\n%s' "$top" "$common"
}
# Cursor delivers the capsule through a rule file in the project, which in a
# worktree would be written into it: a Cursor session there is not served.
if [ "$PROJECT_DIR" = "$ROOT_DIR" ] && [ "$STATE_DIR" = "$ROOT_DIR" ] && [ -n "$SESSION_CWD" ] \
  && [ "$SESSION_CWD" != "$ROOT_DIR" ] && [ -d "$SESSION_CWD" ] \
  && case "$LOOP_INPUT" in *'"cursor_version"'*) false ;; *) true ;; esac \
  && THEIRS=$(checkout_identity "$SESSION_CWD") && MINE=$(checkout_identity "$ROOT_DIR") \
  && [ "${THEIRS%%$'\n'*}" != "${MINE%%$'\n'*}" ] && [ "${THEIRS#*$'\n'}" = "${MINE#*$'\n'}" ] \
  && ROOT_REAL=$(cd "$ROOT_DIR" && pwd -P) && SESSION_REAL=$(cd "$SESSION_CWD" && pwd -P); then
  MINE_TOP=${MINE%%$'\n'*}
  SERVED=${THEIRS%%$'\n'*}${ROOT_REAL:${#MINE_TOP}}
  case "$SESSION_REAL/" in
    "$SERVED"/*)
      if [ -d "$SERVED" ] && [ ! -f "$SERVED/memory-bank/scripts/context.py" ]; then
        PROJECT_DIR=$SERVED
        # The runtime reads these as Windows writes them: Git Bash spells C:\ as /c/,
        # which Python on Windows takes for a folder on the current drive, and the
        # drive letter in upper case, as the save step names the root beside them.
        NATIVE_ROOT=$ROOT_DIR NATIVE_SERVED=$SERVED
        if [ "${OS:-}" = "Windows_NT" ]; then
          case "$NATIVE_ROOT" in /[A-Za-z]|/[A-Za-z]/*) letter=${NATIVE_ROOT:1:1}; NATIVE_ROOT="${letter^^}:${NATIVE_ROOT:2}" ;; esac
          case "$NATIVE_SERVED" in /[A-Za-z]|/[A-Za-z]/*) letter=${NATIVE_SERVED:1:1}; NATIVE_SERVED="${letter^^}:${NATIVE_SERVED:2}" ;; esac
        fi
        export ACCELERATOR_HOME="$NATIVE_ROOT" ACCELERATOR_STATE_DIR="$NATIVE_ROOT" ACCELERATOR_PROJECT_DIR="$NATIVE_SERVED"
      fi ;;
  esac
fi
if [ "$PROJECT_DIR" != "$ROOT_DIR" ]; then
  # Attached, or a worktree its main checkout's installation serves:
  # describe the project the session works in, whatever the cwd.
  cd "$PROJECT_DIR" 2>/dev/null || exit 0
fi
if [ -f "$EDITION_DIR/VERSION" ]; then
  # The policy digest identifies the model-facing surface: policy, skills,
  # agents, commands, settings. VERSION names a release and moves rarely, so
  # on its own it says nothing about whether the prompt layer changed. The
  # digest is the RECORDED one, not a recomputed one: it is honest here,
  # where CI refuses a surface change without regenerating the lock, and it
  # is as stale as VERSION in a project that edited a skill locally without
  # running `policy_lock.py --write`.
  #
  # Deliberately not `skill_tree_fingerprint`: that is computed from mtime
  # and size, which makes it a cache-invalidation key rather than a statement
  # about content — a touched file moves it and an edit that preserves size
  # may not.
  POLICY_LOCK="$EDITION_DIR/.accelerator-policy-lock.json"
  POLICY_DIGEST=""
  if [ -f "$POLICY_LOCK" ]; then
    POLICY_DIGEST=$(sed -n 's/.*"policy_digest"[[:space:]]*:[[:space:]]*"\([0-9a-f]\{8\}\).*/\1/p' "$POLICY_LOCK" 2>/dev/null | head -1)
  fi
  if [ -n "$POLICY_DIGEST" ]; then
    echo "Accelerator version: $(tr -d '[:space:]' < "$EDITION_DIR/VERSION" 2>/dev/null) (policy $POLICY_DIGEST)"
  else
    echo "Accelerator version: $(tr -d '[:space:]' < "$EDITION_DIR/VERSION" 2>/dev/null)"
  fi
fi

if [ "$STATE_DIR" != "$ROOT_DIR" ]; then
  echo "Accelerator: attached from $ROOT_DIR (nothing is installed in this project)."
  echo "Accelerator state (Project Brain, Memory Bank, index): $STATE_DIR"
elif [ "$PROJECT_DIR" != "$ROOT_DIR" ]; then
  echo "Accelerator: the installation in $ROOT_DIR, whose memory this worktree shares (Git gives the worktree none of it): the memory-bank/ and project-brain/ paths the policy and skills name are there."
fi

# Edit and repetition counts are session-scoped (loop-detection.sh and the
# guard at the end of bash-validator.sh). Their counters sit in this user's
# directory for this repository, keyed by session: this session's start over,
# any session's left untouched for a day are pruned, and another session still
# running in this checkout keeps its own. A directory this user does not own,
# or a symbolic link in its place, is left alone.
SESSION_KEY=$(printf '%s' "$LOOP_INPUT" \
  | sed -n -E 's/.*"(session_id|conversation_id)"[[:space:]]*:[[:space:]]*"([^"]*)".*/\2/p' | head -1)
SESSION_KEY=${SESSION_KEY//[^A-Za-z0-9]/}
SESSION_KEY=${SESSION_KEY:0:64}
[ -n "$SESSION_KEY" ] || SESSION_KEY=shared
REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
REPO_KEY=$(printf '%s' "$REPO_ROOT" | cksum | cut -d' ' -f1)
TRACK_BASE=${TMPDIR:-/tmp}
TRACK_DIR="${TRACK_BASE%/}/cursor-loop-detection-${EUID:-0}-$REPO_KEY"
if [ -d "$TRACK_DIR" ] && [ ! -L "$TRACK_DIR" ] && [ -O "$TRACK_DIR" ]; then
  find "$TRACK_DIR" -maxdepth 1 -type f \( -name "cmd-$SESSION_KEY-*" -o -name "edit-$SESSION_KEY-*" -o -mtime +0 \) \
    -delete 2>/dev/null || true
fi

# Git info. The branch line waits for the runtime's status below, which
# tells the team's uncommitted changes from the accelerator's own files: a
# project that neither commits nor ignores the accelerator otherwise reads
# as holding uncommitted work in every session.
BRANCH_LINE=""
if git rev-parse --git-dir > /dev/null 2>&1; then
  BRANCH_LINE="Branch: $(git branch --show-current 2>/dev/null)"
fi
UNCOMMITTED="-"
ACCELERATOR_UNCOMMITTED="-"
branch_line() {
  [ -n "$BRANCH_LINE" ] || return 0
  case "$UNCOMMITTED" in
    ''|*[!0-9]*)
      echo "$BRANCH_LINE ($(git status --short 2>/dev/null | wc -l | tr -d ' ') uncommitted changes)" ;;
    *)
      case "$ACCELERATOR_UNCOMMITTED" in
        ''|0|*[!0-9]*) echo "$BRANCH_LINE ($UNCOMMITTED uncommitted changes)" ;;
        *) echo "$BRANCH_LINE ($UNCOMMITTED uncommitted changes; $ACCELERATOR_UNCOMMITTED more are the accelerator's own files)" ;;
      esac ;;
  esac
}

# Dependency managers
MANAGERS=""
[ -f "composer.json" ] && MANAGERS="$MANAGERS composer"
[ -f "composer.lock" ] && MANAGERS="$MANAGERS composer.lock"
[ -f "package-lock.json" ] && MANAGERS="$MANAGERS npm"
[ -f "yarn.lock" ] && MANAGERS="$MANAGERS yarn"
[ -f "pnpm-lock.yaml" ] && MANAGERS="$MANAGERS pnpm"
[ -n "$MANAGERS" ] && echo "Dependencies:$MANAGERS"

# PHP runtime
command -v php > /dev/null 2>&1 && echo "PHP: $(php -r 'echo PHP_VERSION;' 2>/dev/null)"

# Tooling markers
[ -f "phpunit.xml" ] || [ -f "phpunit.xml.dist" ] && echo "Tests: PHPUnit config present"
[ -f "tests/Pest.php" ] && echo "Tests: Pest present"
[ -f ".php-cs-fixer.php" ] || [ -f ".php-cs-fixer.dist.php" ] && echo "Formatter: PHP-CS-Fixer config present"
[ -f "phpcs.xml" ] || [ -f "phpcs.xml.dist" ] && echo "Formatter: PHP_CodeSniffer config present"
[ -f "phpstan.neon" ] || [ -f "phpstan.neon.dist" ] && echo "Static analysis: PHPStan config present"
[ -f "psalm.xml" ] || [ -f "psalm.xml.dist" ] && echo "Static analysis: Psalm config present"
[ -f "rector.php" ] && echo "Refactoring: Rector config present"
[ -f "phpbench.json" ] && echo "Benchmarks: PHPBench config present"
[ -f "public/index.php" ] && echo "Entry point: public/index.php (front controller)"
[ -f "Makefile" ] && echo "Build system: Make"
[ -f "Dockerfile" ] || [ -f "docker-compose.yml" ] || [ -f "compose.yaml" ] && echo "Containers: Docker present"

# Framework detection: this folder (PHP Core/) targets native PHP.
# If a framework is present, prefer the matching sibling accelerator folder.
FRAMEWORK=""
[ -f "artisan" ] && FRAMEWORK="Laravel"
[ -f "bin/console" ] && FRAMEWORK="Symfony"
if grep -qi "cakephp/cakephp" composer.json 2>/dev/null; then FRAMEWORK="CakePHP"; fi
if grep -qi "yiisoft/yii2" composer.json 2>/dev/null; then FRAMEWORK="Yii"; fi
if [ -n "$FRAMEWORK" ]; then
  echo ""
  echo "NOTE: $FRAMEWORK detected. This is the native-PHP base folder (PHP Core/)."
  echo "      For framework-specific conventions, switch to the matching sibling folder (Laravel/, Symfony/)."
fi

# Report metadata only; never index, retrieve, print, or inject record contents.
CONTEXT_CLI="$ROOT_DIR/memory-bank/scripts/context.py"

# Validation results are cached per repository state so an unchanged checkout
# does not re-run the full Project Brain and memory-bank validation walks on
# every session start. Cache key = git HEAD + a working-tree fingerprint
# (porcelain status + tracked-content diff): any commit, stage, or edit of a
# tracked file invalidates it. Content-only edits to untracked files are not
# fingerprinted and may serve one stale summary; this banner is informational
# and `context.py validate` stays authoritative. The directory name is
# deliberately tool-neutral: the Claude, Cursor and Codex mirrors of this
# hook share one cache per edition checkout.
EDITION_KEY=$(printf '%s' "$STATE_DIR" | cksum | cut -d' ' -f1)
VALIDATION_CACHE_DIR="${TMPDIR:-/tmp}/ai-accelerator-session-cache-$EDITION_KEY"
VALIDATION_CACHE_KEY=""
if git rev-parse --git-dir >/dev/null 2>&1; then
  VALIDATION_CACHE_KEY=$({ git rev-parse HEAD; git status --porcelain=v1 --untracked-files=all; git diff HEAD; } 2>/dev/null | cksum | cut -d' ' -f1)
fi

cache_read() {
  [ -n "$VALIDATION_CACHE_KEY" ] && cat "$VALIDATION_CACHE_DIR/$1-$VALIDATION_CACHE_KEY" 2>/dev/null
}

cache_write() {
  [ -n "$VALIDATION_CACHE_KEY" ] || return 0
  mkdir -p "$VALIDATION_CACHE_DIR" 2>/dev/null || return 0
  find "$VALIDATION_CACHE_DIR" -type f -mtime +7 -delete 2>/dev/null
  printf '%s\n' "$2" > "$VALIDATION_CACHE_DIR/$1-$VALIDATION_CACHE_KEY" 2>/dev/null || true
}

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
resolve_python
# The policy names the memory CLI as `python3 memory-bank/scripts/context.py`,
# which fails where python3 is not Python - native Windows with the
# python.org installer. The session hears the name that works here.
if [ -n "${PYTHON[0]:-}" ] && [ "${PYTHON[*]}" != python3 ] && [ -f "$CONTEXT_CLI" ]; then
  echo "Python here is \`${PYTHON[*]}\`, not python3: run the memory CLI commands the policy names with \`${PYTHON[*]}\` in place of \`python3\`."
fi
# The policy and skills name the CLI by its path from the root, which a
# worktree this installation serves has none of, and which a session started
# below the root - its commands run there, though the dispatcher runs this
# hook in the root - does not reach: the session hears where it is. Run from
# where the session is, it finds the memory and the task by itself.
CLI_WHERE=
if [ "$PROJECT_DIR" != "$ROOT_DIR" ] && [ "$STATE_DIR" = "$ROOT_DIR" ] \
  && [ ! -f "$PROJECT_DIR/memory-bank/scripts/context.py" ]; then
  CLI_WHERE="this worktree has none"
elif [ "$PROJECT_DIR" = "$ROOT_DIR" ] && [ -n "$SESSION_CWD" ] && [ "$SESSION_CWD" != "$ROOT_DIR" ] \
  && SESSION_REAL=$(cd "$SESSION_CWD" 2>/dev/null && pwd -P) && ROOT_REAL=$(cd "$ROOT_DIR" && pwd -P) \
  && [ "${SESSION_REAL#"$ROOT_REAL"/}" != "$SESSION_REAL" ]; then
  CLI_WHERE="this session started below the project root"
fi
if [ -n "$CLI_WHERE" ] && [ -f "$CONTEXT_CLI" ]; then
  # Bare, or in single quotes: a double-quoted path is one the dispatcher's
  # allow for the memory CLI does not take, since bash expands inside it.
  CLI_NAME=$CONTEXT_CLI
  case "$CLI_NAME" in
    *[!A-Za-z0-9_./-]*) CLI_NAME="'$(printf '%s' "$CLI_NAME" | sed "s/'/'\\\\''/g")'" ;;
  esac
  echo "Memory CLI: $CLI_WHERE - run the commands the policy and skills name with \`${PYTHON[*]:-python3} $CLI_NAME\` in place of \`python3 memory-bank/scripts/context.py\`."
fi

if [ -n "${PYTHON[0]:-}" ] && [ -f "$CONTEXT_CLI" ]; then
  CONTEXT_STATUS=$("${PYTHON[@]}" "$CONTEXT_CLI" status --json 2>/dev/null || true)
  # The two counts come before the database path, which may be empty, and
  # are "-" when unknown: read splits on tabs and would shift an empty field.
  STATUS_FIELDS=$("${PYTHON[@]}" -c 'import json,sys; d=json.loads(sys.argv[1]); print("{}\t{}\t{}\t{}\t{}\t{}".format(d.get("mode", "unknown"), d.get("working", "unknown"), d.get("documents", "unknown"), d.get("uncommitted", "-"), d.get("accelerator_uncommitted", "-"), d.get("database", "")))' "$CONTEXT_STATUS" 2>/dev/null || true)
  if [ -n "$STATUS_FIELDS" ]; then
    IFS=$'\t' read -r CONTEXT_MODE ACTIVE_BINDINGS INDEX_DOCUMENTS UNCOMMITTED ACCELERATOR_UNCOMMITTED INDEX_DB <<< "$STATUS_FIELDS"
    branch_line
    INDEX_HEALTH="healthy"
    INDEX_STALENESS="unknown"
    if [ ! -f "$INDEX_DB" ]; then
      INDEX_HEALTH="missing"
    elif [ "$INDEX_DOCUMENTS" = "0" ]; then
      INDEX_HEALTH="empty"
    elif find "$PROJECT_DIR/AGENTS.md" "$PROJECT_DIR/README.md" "$PROJECT_DIR/CHANGELOG.md" "$PROJECT_DIR/specs" "$PROJECT_DIR/docs" "$PROJECT_DIR/tasks" "$STATE_DIR/memory-bank/chunks" "$STATE_DIR/project-brain/dynamic" "$STATE_DIR/project-brain/control" "$ROOT_DIR/AGENTS.md" "$ROOT_DIR/.agents/skills" "$ROOT_DIR/.claude/skills" "$ROOT_DIR/.cursor/skills" -type f -name "*.md" -newer "$INDEX_DB" -print -quit 2>/dev/null | grep -q .; then
      INDEX_STALENESS="stale"
    else
      INDEX_STALENESS="current"
    fi
    VALIDATION_STATUS=$(cache_read brain-validation)
    if [ -z "$VALIDATION_STATUS" ]; then
      VALIDATION_JSON=$("${PYTHON[@]}" "$CONTEXT_CLI" validate --json 2>/dev/null || true)
      # The CLI emits {"valid": true|false, ...}; a substring test replaces a
      # third Python startup just to read one boolean.
      case "$VALIDATION_JSON" in
        *'"valid": true'*|*'"valid":true'*) VALIDATION_STATUS="valid" ;;
        *'"valid"'*) VALIDATION_STATUS="invalid" ;;
        *) VALIDATION_STATUS="unavailable" ;;
      esac
      [ "$VALIDATION_STATUS" != "unavailable" ] && cache_write brain-validation "$VALIDATION_STATUS"
    fi
    echo "Context governance: mode=$CONTEXT_MODE, index=$INDEX_HEALTH/$INDEX_STALENESS, active-bindings=$ACTIVE_BINDINGS, brain-validation=$VALIDATION_STATUS."
  else
    branch_line
    echo "Context governance: mode/index/bindings/validation unavailable."
  fi
elif [ -f "$CONTEXT_CLI" ]; then
  branch_line
  # The memory hooks stay silent without Python, prompt after prompt; this is
  # the one place a session hears why memory is neither read nor written.
  echo "Context governance: unavailable - no Python 3.9+ found (python3, python or py -3), so memory is neither read nor written."
else
  branch_line
fi

if [ -f "$ROOT_DIR/memory-bank/README.md" ] && [ -f "$STATE_DIR/memory-bank/INDEX.md" ]; then
  if [ -n "${PYTHON[0]:-}" ] && [ -f "$ROOT_DIR/memory-bank/scripts/validate.py" ]; then
    MEMORY_SUMMARY=$(cache_read memory-summary)
    if [ -z "$MEMORY_SUMMARY" ]; then
      MEMORY_SUMMARY=$("${PYTHON[@]}" "$ROOT_DIR/memory-bank/scripts/validate.py" --summary "$STATE_DIR/memory-bank" 2>/dev/null)
      [ -n "$MEMORY_SUMMARY" ] && cache_write memory-summary "$MEMORY_SUMMARY"
    fi
    if [ "$PROJECT_DIR" = "$ROOT_DIR" ]; then
      echo "$MEMORY_SUMMARY Read memory-bank/README.md and INDEX.md before relevant durable-memory work."
    else
      echo "$MEMORY_SUMMARY Read $ROOT_DIR/memory-bank/README.md and $STATE_DIR/memory-bank/INDEX.md before relevant durable-memory work."
    fi
  else
    echo "Memory bank: available (validation unavailable)."
  fi
fi

# Project structure
echo ""
echo "Structure:"
for DIR in src app public config bin templates views tests specs tasks memory-bank project-brain examples .cursor; do
  if [ -d "$DIR" ]; then
    COUNT=$(find "$DIR" -maxdepth 1 -type f 2>/dev/null | wc -l | tr -d ' ')
    echo "  $DIR/ ($COUNT files)"
  fi
done

# Task and spec counts
if [ -d "tasks" ]; then
  TASK_COUNT=$(find tasks -maxdepth 1 -type d -name "TASK-*" 2>/dev/null | wc -l | tr -d ' ')
  echo "  Tasks: $TASK_COUNT"
fi
if [ -d "specs" ]; then
  SPEC_COUNT=$(find specs -maxdepth 1 -type f -name "*.md" ! -name "MANIFEST.md" 2>/dev/null | wc -l | tr -d ' ')
  echo "  Specs: $SPEC_COUNT"
fi

# Cursor's direct-host channel is the owned local working-memory.mdc rule.
[ "$STATE_DIR" = "$ROOT_DIR" ] || exit 0
cursor_rule_run() {
  local CURSOR_MAINTENANCE_STATUS=0 CURSOR_MAINTENANCE_REPORT CURSOR_PROJECTION_STATUS=0
  if command -v timeout >/dev/null 2>&1; then
    CURSOR_MAINTENANCE_REPORT=$(timeout 3 "${PYTHON[@]}" "$CONTEXT_CLI" "$@" --diagnostic 2>/dev/null) || CURSOR_MAINTENANCE_STATUS=$?
  else
    CURSOR_MAINTENANCE_REPORT=$("${PYTHON[@]}" "$CONTEXT_CLI" "$@" --diagnostic 2>/dev/null) || CURSOR_MAINTENANCE_STATUS=$?
  fi
  if [ "$CURSOR_MAINTENANCE_STATUS" -ne 0 ]; then
    printf '%s\n' 'Cursor memory rule was not updated: maintenance_unavailable.' >&2
    return "$CURSOR_MAINTENANCE_STATUS"
  fi
  # 4096 limits projection admission, not the shell capture of trusted CLI
  # stdout. Both subprocess stderr streams are discarded before fixed reports.
  printf '%s' "$CURSOR_MAINTENANCE_REPORT" | "${PYTHON[@]}" -I -S -c '
import json,sys
try:
    raw=sys.stdin.buffer.read(4097)
    if len(raw)>4096: raise ValueError("bounded outcome required")
    value=json.loads(raw)
    state=value.get("state") if isinstance(value,dict) else None
    if state not in ("written","unchanged","kept","removed","absent","attached","preserved","incomplete"):
        raise ValueError("known state required")
    print(json.dumps({"state":state}))
    sys.exit(5 if state=="preserved" and value.get("reason")=="linked_directory"
             else 3 if state=="preserved" else 4 if state=="incomplete" else 0)
except (ValueError,TypeError,RecursionError):
    sys.exit(1)
' 2>/dev/null || CURSOR_PROJECTION_STATUS=$?
  case "$CURSOR_PROJECTION_STATUS" in
    0) return 0 ;;
    3) printf '%s\n' 'Cursor memory rule was not updated: custom_or_unavailable.' >&2 ;;
    5) printf '%s\n' 'Cursor memory rule was not updated: linked_directory.' >&2 ;;
    4) printf '%s\n' 'Cursor memory rule was not updated: maintenance_incomplete.' >&2 ;;
    *) printf '%s\n' 'Cursor memory rule was not updated: maintenance_unavailable.' >&2; return 1 ;;
  esac
  return 0
}
if [ "${CONTEXT_CAPSULE_DELIVERED:-}" = "1" ]; then
  cursor_rule_run cursor-rule-retire >/dev/null || true
  exit 0
fi
CAPSULE_BUDGET_SECONDS="${CONTEXT_HOOK_BUDGET:-5}"
CAPSULE_TASK_ID="${CONTEXT_TASK_ID:-$(git -C "$PROJECT_DIR" branch --show-current 2>/dev/null)}"
CAPSULE_IDENTITY_ARGUMENTS=()
if [ -z "${CONTEXT_TASK_ID:-}" ] && [ -n "$CAPSULE_TASK_ID" ]; then
  CAPSULE_IDENTITY_ARGUMENTS=(--expected-branch "$CAPSULE_TASK_ID")
fi
CAPSULE_STATUS=3
if [ -n "${PYTHON[0]:-}" ] && [ -f "$CONTEXT_CLI" ] && [ -n "$CAPSULE_TASK_ID" ]; then
  if command -v timeout >/dev/null 2>&1; then
    CAPSULE=$(timeout "$CAPSULE_BUDGET_SECONDS" "${PYTHON[@]}" "$CONTEXT_CLI" hook-context --host cursor --delivery-profile cursor-rule --task-id "$CAPSULE_TASK_ID" "${CAPSULE_IDENTITY_ARGUMENTS[@]}" 2>/dev/null)
    CAPSULE_STATUS=$?
  else
    CAPSULE=$("${PYTHON[@]}" "$CONTEXT_CLI" hook-context --host cursor --delivery-profile cursor-rule --task-id "$CAPSULE_TASK_ID" "${CAPSULE_IDENTITY_ARGUMENTS[@]}" 2>/dev/null)
    CAPSULE_STATUS=$?
  fi
fi
if [ "$CAPSULE_STATUS" -eq 3 ]; then
  cursor_rule_run cursor-rule-retire >/dev/null || true
elif [ "$CAPSULE_STATUS" -eq 0 ]; then
  printf '%s' "$CAPSULE" | cursor_rule_run cursor-rule --action write --phase session --task-id "$CAPSULE_TASK_ID" >/dev/null || true
else
  cursor_rule_run cursor-rule --action keep --task-id "$CAPSULE_TASK_ID" >/dev/null || true
fi

# Claude Code started in a folder below this project reads none of its
# settings - only the starting folder's - so such a session ran none of these
# hooks. This keeps a small dispatcher in your own Claude Code settings that
# runs this project's hooks for it (memory-bank/scripts/claude_subfolders.py):
# installed projects only, written only for the Claude Code that started this
# hook, and in the background, so the session's start never waits for it.
if [ "$STATE_DIR" = "$ROOT_DIR" ] && [ -n "${PYTHON[0]:-}" ] && [ -f "$ROOT_DIR/memory-bank/scripts/claude_subfolders.py" ]; then
  printf '%s' "$LOOP_INPUT" | "${PYTHON[@]}" "$ROOT_DIR/memory-bank/scripts/claude_subfolders.py" > /dev/null 2>&1 &
fi

exit 0
