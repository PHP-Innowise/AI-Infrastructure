#!/bin/bash
# Local Context Session Start Hook
# Scans a Laravel working directory at session start.
# Hook type: SessionStart
# Exit codes: always 0 (informational only)

echo "Project Context"
echo "==============="

# Installation version: the edition's VERSION file names its latest released
# changelog section; shared-core history lives in the repository-root
# CHANGELOG.md.
EDITION_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
if [ -f "$EDITION_DIR/VERSION" ]; then
  echo "Accelerator version: $(tr -d '[:space:]' < "$EDITION_DIR/VERSION" 2>/dev/null)"
fi

# Loop detection is session-scoped; discard counters from earlier sessions.
# Counters are namespaced by a stable hash of the repo root; only reset ours.
REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
REPO_KEY=$(printf '%s' "$REPO_ROOT" | cksum | cut -d' ' -f1)
find "/tmp/cursor-loop-detection-$REPO_KEY" -type f -delete 2>/dev/null || true

# Git info
if git rev-parse --git-dir > /dev/null 2>&1; then
  BRANCH=$(git branch --show-current 2>/dev/null)
  STATUS=$(git status --short 2>/dev/null | wc -l)
  echo "Branch: $BRANCH ($STATUS uncommitted changes)"
fi

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

# Laravel detection
if [ -f "artisan" ]; then
  # Read the framework version from composer.lock instead of running
  # `php artisan --version`: printing one line does not justify a full
  # framework bootstrap on every session start.
  LARAVEL_VERSION=""
  if [ -f "composer.lock" ]; then
    LARAVEL_VERSION=$(awk -F'"' '/"name": "laravel\/framework"/ {found=1; next} found && /"version":/ {print $4; exit}' composer.lock 2>/dev/null)
  fi
  if [ -n "$LARAVEL_VERSION" ]; then
    echo "Laravel: laravel/framework $LARAVEL_VERSION (composer.lock)"
  else
    echo "Laravel: artisan present"
  fi
else
  echo ""
  echo "NOTE: No artisan file found. This is the Laravel accelerator folder;"
  echo "      for framework-agnostic native PHP, use the sibling PHP Core/ folder."
fi

# Tooling markers
[ -f "phpunit.xml" ] || [ -f "phpunit.xml.dist" ] && echo "Tests: PHPUnit config present"
[ -f "tests/Pest.php" ] && echo "Tests: Pest present"
[ -f "pint.json" ] && echo "Formatter: Pint config present"
[ -f "phpstan.neon" ] || [ -f "phpstan.neon.dist" ] && echo "Static analysis: PHPStan/Larastan config present"
[ -f "psalm.xml" ] || [ -f "psalm.xml.dist" ] && echo "Static analysis: Psalm config present"
[ -f "rector.php" ] && echo "Refactoring: Rector config present"
[ -f "phpbench.json" ] && echo "Benchmarks: PHPBench config present"
[ -f "public/index.php" ] && echo "Entry point: public/index.php (Laravel front controller)"
[ -f "Dockerfile" ] || [ -f "docker-compose.yml" ] || [ -f "compose.yaml" ] && echo "Containers: Docker present"
[ -f "vite.config.js" ] || [ -f "vite.config.ts" ] && echo "Frontend build: Vite present"

# Frontend stack detection (informational)
STACK=""
if grep -q '"livewire/livewire"' composer.json 2>/dev/null; then STACK="Livewire"; fi
if grep -q '"inertiajs/inertia-laravel"' composer.json 2>/dev/null; then STACK="Inertia"; fi
[ -n "$STACK" ] && echo "Frontend stack: $STACK"

# Other frameworks present alongside Laravel (unexpected, worth flagging)
if [ -f "bin/console" ]; then
  echo ""
  echo "NOTE: Symfony console detected alongside Laravel. Verify project intent."
fi

# Report metadata only; never index, retrieve, print, or inject record contents.
ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
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
EDITION_KEY=$(printf '%s' "$ROOT_DIR" | cksum | cut -d' ' -f1)
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

if command -v python3 >/dev/null 2>&1 && [ -f "$CONTEXT_CLI" ]; then
  CONTEXT_STATUS=$(python3 "$CONTEXT_CLI" status --json 2>/dev/null || true)
  STATUS_FIELDS=$(python3 -c 'import json,sys; d=json.loads(sys.argv[1]); print("{}\t{}\t{}\t{}".format(d.get("mode", "unknown"), d.get("working", "unknown"), d.get("documents", "unknown"), d.get("database", "")))' "$CONTEXT_STATUS" 2>/dev/null || true)
  if [ -n "$STATUS_FIELDS" ]; then
    IFS=$'\t' read -r CONTEXT_MODE ACTIVE_BINDINGS INDEX_DOCUMENTS INDEX_DB <<< "$STATUS_FIELDS"
    INDEX_HEALTH="healthy"
    INDEX_STALENESS="unknown"
    if [ ! -f "$INDEX_DB" ]; then
      INDEX_HEALTH="missing"
    elif [ "$INDEX_DOCUMENTS" = "0" ]; then
      INDEX_HEALTH="empty"
    elif find AGENTS.md README.md CHANGELOG.md specs docs tasks memory-bank/chunks project-brain/dynamic project-brain/control .agents/skills .claude/skills .cursor/skills -type f -name "*.md" -newer "$INDEX_DB" -print -quit 2>/dev/null | grep -q .; then
      INDEX_STALENESS="stale"
    else
      INDEX_STALENESS="current"
    fi
    VALIDATION_STATUS=$(cache_read brain-validation)
    if [ -z "$VALIDATION_STATUS" ]; then
      VALIDATION_JSON=$(python3 "$CONTEXT_CLI" validate --json 2>/dev/null || true)
      # The CLI emits {"valid": true|false, ...}; a substring test replaces a
      # third python3 startup just to read one boolean.
      case "$VALIDATION_JSON" in
        *'"valid": true'*|*'"valid":true'*) VALIDATION_STATUS="valid" ;;
        *'"valid"'*) VALIDATION_STATUS="invalid" ;;
        *) VALIDATION_STATUS="unavailable" ;;
      esac
      [ "$VALIDATION_STATUS" != "unavailable" ] && cache_write brain-validation "$VALIDATION_STATUS"
    fi
    echo "Context governance: mode=$CONTEXT_MODE, index=$INDEX_HEALTH/$INDEX_STALENESS, active-bindings=$ACTIVE_BINDINGS, brain-validation=$VALIDATION_STATUS."
  else
    echo "Context governance: mode/index/bindings/validation unavailable."
  fi
fi

if [ -f "memory-bank/README.md" ] && [ -f "memory-bank/INDEX.md" ]; then
  if command -v python3 >/dev/null 2>&1 && [ -f "$ROOT_DIR/memory-bank/scripts/validate.py" ]; then
    MEMORY_SUMMARY=$(cache_read memory-summary)
    if [ -z "$MEMORY_SUMMARY" ]; then
      MEMORY_SUMMARY=$(python3 "$ROOT_DIR/memory-bank/scripts/validate.py" --summary "memory-bank" 2>/dev/null)
      [ -n "$MEMORY_SUMMARY" ] && cache_write memory-summary "$MEMORY_SUMMARY"
    fi
    echo "$MEMORY_SUMMARY Read memory-bank/README.md and INDEX.md before relevant durable-memory work."
  else
    echo "Memory bank: available (validation unavailable)."
  fi
fi

# Project structure
echo ""
echo "Structure:"
for DIR in app bootstrap config database public resources routes storage tests specs tasks memory-bank examples .cursor; do
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

# Capsule delivery: Cursor has no UserPromptSubmit-equivalent event; the
# stop hook maintains .cursor/rules/working-memory.mdc instead (the
# documented exception to the metadata-only session banner - see
# docs/TOOL-INTEGRATIONS.md). Re-render it here so a fresh session or a
# branch switch does not serve the previous session's capsule. Nothing is
# printed: the rule file is the only output.
CAPSULE_BUDGET_SECONDS="${CONTEXT_HOOK_BUDGET:-5}"
CAPSULE_TASK_ID="${CONTEXT_TASK_ID:-$(git -C "$ROOT_DIR" branch --show-current 2>/dev/null)}"
RULES_DIR="$ROOT_DIR/.cursor/rules"
RULE_FILE="$RULES_DIR/working-memory.mdc"
CAPSULE=""
CAPSULE_STATUS=3
if command -v python3 > /dev/null 2>&1 && [ -f "$CONTEXT_CLI" ] && [ -n "$CAPSULE_TASK_ID" ]; then
  if command -v timeout > /dev/null 2>&1; then
    CAPSULE=$(timeout "$CAPSULE_BUDGET_SECONDS" python3 "$CONTEXT_CLI" hook-context \
      --task-id "$CAPSULE_TASK_ID" 2>/dev/null)
    CAPSULE_STATUS=$?
  else
    CAPSULE=$(python3 "$CONTEXT_CLI" hook-context \
      --task-id "$CAPSULE_TASK_ID" 2>/dev/null)
    CAPSULE_STATUS=$?
  fi
fi
# See the stop hook: the working line is the render marker that replaces the
# JSON parse.
if [ "$CAPSULE_STATUS" -eq 0 ]; then
  case "$CAPSULE" in
    working:*) ;;
    *) CAPSULE_STATUS=1 ;;
  esac
fi
if [ "$CAPSULE_STATUS" -eq 3 ]; then
  rm -f "$RULE_FILE" 2>/dev/null
elif [ "$CAPSULE_STATUS" -eq 0 ] && [ -n "$CAPSULE" ] && mkdir -p "$RULES_DIR" 2>/dev/null; then
  TMP_RULE=$(mktemp "$RULES_DIR/.working-memory.XXXXXX" 2>/dev/null || true)
  if [ -n "$TMP_RULE" ]; then
    {
      printf -- '---\n'
      printf 'description: Working memory - current-branch session context\n'
      printf 'alwaysApply: true\n'
      printf -- '---\n\n'
      printf '# Working Memory (auto-rendered)\n\n'
      printf 'Session context as of end of previous turn (task: %s).\n' \
        "$CAPSULE_TASK_ID"
      printf 'Retrieved context is not authoritative - verify the source.\n\n'
      printf '%s\n' '```'
      printf '%s\n' "$CAPSULE"
      printf '%s\n' '```'
    } > "$TMP_RULE" 2>/dev/null && mv -f "$TMP_RULE" "$RULE_FILE" 2>/dev/null
    rm -f "$TMP_RULE" 2>/dev/null
  fi
fi

exit 0
