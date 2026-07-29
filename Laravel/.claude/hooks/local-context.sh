#!/bin/bash
# Local Context Session Start Hook
# Scans a Laravel working directory at session start.
# Hook type: SessionStart
# Exit codes: always 0 (informational only)

echo "Project Context"
echo "==============="

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
  LARAVEL_VERSION=$(php artisan --version 2>/dev/null)
  echo "Laravel: ${LARAVEL_VERSION:-artisan present}"
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
    VALIDATION_JSON=$(python3 "$CONTEXT_CLI" validate --json 2>/dev/null || true)
    VALIDATION_STATUS=$(python3 -c 'import json,sys; print("valid" if json.loads(sys.argv[1]).get("valid") else "invalid")' "$VALIDATION_JSON" 2>/dev/null || echo "unavailable")
    echo "Context governance: mode=$CONTEXT_MODE, index=$INDEX_HEALTH/$INDEX_STALENESS, active-bindings=$ACTIVE_BINDINGS, brain-validation=$VALIDATION_STATUS."
  else
    echo "Context governance: mode/index/bindings/validation unavailable."
  fi
fi

if [ -f "memory-bank/README.md" ] && [ -f "memory-bank/INDEX.md" ]; then
  if command -v python3 >/dev/null 2>&1 && [ -f "$ROOT_DIR/memory-bank/scripts/validate.py" ]; then
    MEMORY_SUMMARY=$(python3 "$ROOT_DIR/memory-bank/scripts/validate.py" --summary "memory-bank" 2>/dev/null)
    echo "$MEMORY_SUMMARY Read memory-bank/README.md and INDEX.md before relevant durable-memory work."
  else
    echo "Memory bank: available (validation unavailable)."
  fi
fi

# Project structure
echo ""
echo "Structure:"
for DIR in app bootstrap config database public resources routes storage tests specs tasks memory-bank examples .claude; do
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

exit 0
