#!/bin/bash
# Local Context Session Start Hook
# Scans a WordPress working directory at session start.
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
  && [ "$(cd "${ACCELERATOR_HOME:-/nonexistent}" 2>/dev/null && pwd -P)" = "$(cd "$ROOT_DIR" && pwd -P)" ]; then
  PROJECT_DIR=$ACCELERATOR_PROJECT_DIR
  STATE_DIR=$ACCELERATOR_STATE_DIR
fi
if [ "$PROJECT_DIR" != "$ROOT_DIR" ]; then
  # Attached, or a worktree its main checkout's installation serves:
  # describe the project the session works in, whatever the cwd.
  cd "$PROJECT_DIR" 2>/dev/null || exit 0
fi
if [ -f "$EDITION_DIR/VERSION" ]; then
  echo "Accelerator version: $(tr -d '[:space:]' < "$EDITION_DIR/VERSION" 2>/dev/null)"
fi

if [ "$STATE_DIR" != "$ROOT_DIR" ]; then
  echo "Accelerator: attached from $ROOT_DIR (nothing is installed in this project)."
  echo "Accelerator state (Project Brain, Memory Bank, index): $STATE_DIR"
elif [ "$PROJECT_DIR" != "$ROOT_DIR" ]; then
  echo "Accelerator: the installation in $ROOT_DIR, whose memory this worktree shares (Git gives the worktree none of it)."
fi

# Edit and repetition counts are session-scoped (loop-detection.sh and the
# guard at the end of bash-validator.sh). Their counters sit in this user's
# directory for this repository, keyed by session: this session's start over,
# any session's left untouched for a day are pruned, and another session still
# running in this checkout keeps its own. A directory this user does not own,
# or a symbolic link in its place, is left alone.
LOOP_INPUT=
[ -t 0 ] || IFS= read -r -d '' LOOP_INPUT || :
SESSION_KEY=$(printf '%s' "$LOOP_INPUT" \
  | sed -n -E 's/.*"(session_id|conversation_id)"[[:space:]]*:[[:space:]]*"([^"]*)".*/\2/p' | head -1)
SESSION_KEY=${SESSION_KEY//[^A-Za-z0-9]/}
SESSION_KEY=${SESSION_KEY:0:64}
[ -n "$SESSION_KEY" ] || SESSION_KEY=shared
REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
REPO_KEY=$(printf '%s' "$REPO_ROOT" | cksum | cut -d' ' -f1)
TRACK_BASE=${TMPDIR:-/tmp}
TRACK_DIR="${TRACK_BASE%/}/claude-loop-detection-${EUID:-0}-$REPO_KEY"
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
[ -f "wp-config.php" ] && echo "WordPress: full-site configuration present"
[ -f "config/application.php" ] && echo "WordPress: Bedrock-style configuration present"
[ -f "style.css" ] && echo "WordPress: theme stylesheet/header present"
[ -f "theme.json" ] && echo "WordPress: block or hybrid theme.json present"
find . -maxdepth 2 -type f -name 'block.json' -print -quit 2>/dev/null | grep -q . && echo "WordPress: block metadata present"
command -v wp >/dev/null 2>&1 && echo "WP-CLI: available"
[ -f "Makefile" ] && echo "Build system: Make"
[ -f "Dockerfile" ] || [ -f "docker-compose.yml" ] || [ -f "compose.yaml" ] && echo "Containers: Docker present"

# Project-shape hints. These are orientation only; canonical files decide.
[ -f "woocommerce.php" ] || grep -qi 'woocommerce' composer.json 2>/dev/null && echo "WordPress specialty: WooCommerce evidence present"
grep -qi 'wordpress-plugin' composer.json 2>/dev/null && echo "WordPress shape: Composer plugin package"
grep -qi 'wordpress-theme' composer.json 2>/dev/null && echo "WordPress shape: Composer theme package"

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
for DIR in includes src app blocks build assets templates parts patterns tests specs tasks memory-bank project-brain examples .claude; do
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

# Capsule delivery: this client receives the Task Capsule at prompt time
# through working-memory-read.sh; session start reports metadata only.

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
