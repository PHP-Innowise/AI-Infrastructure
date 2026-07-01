#!/bin/bash
# Local Context Session Start Hook
# Scans a Laravel/PHP working directory at session start.
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

# Package managers and framework markers
MANAGERS=""
[ -f "composer.lock" ] && MANAGERS="$MANAGERS composer"
[ -f "package-lock.json" ] && MANAGERS="$MANAGERS npm"
[ -f "yarn.lock" ] && MANAGERS="$MANAGERS yarn"
[ -f "pnpm-lock.yaml" ] && MANAGERS="$MANAGERS pnpm"
[ -n "$MANAGERS" ] && echo "Package managers:$MANAGERS"

[ -f "artisan" ] && echo "Framework: Laravel"
[ -f "composer.json" ] && echo "PHP project: composer.json present"
[ -f "phpunit.xml" ] && echo "Tests: PHPUnit config present"
[ -f "pint.json" ] && echo "Formatter: Laravel Pint config present"
[ -f "phpstan.neon" ] || [ -f "phpstan.neon.dist" ] && echo "Static analysis: PHPStan config present"
[ -f "psalm.xml" ] && echo "Static analysis: Psalm config present"
[ -f "vite.config.js" ] || [ -f "vite.config.ts" ] && echo "Frontend build: Vite"
[ -f "Makefile" ] && echo "Build system: Make"

# Project structure
echo ""
echo "Structure:"
for DIR in app routes database tests resources config tasks specs examples .claude; do
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
