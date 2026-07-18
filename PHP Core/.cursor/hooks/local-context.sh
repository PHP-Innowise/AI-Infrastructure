#!/bin/bash
# Local Context Session Start Hook
# Scans a native PHP working directory at session start.
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

# Report memory metadata only; never print chunk contents from a hook.
if [ -f "memory-bank/README.md" ] && [ -f "memory-bank/INDEX.md" ]; then
  ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
  if command -v python3 >/dev/null 2>&1 && [ -f "$ROOT_DIR/memory-bank/scripts/validate.py" ]; then
    MEMORY_SUMMARY=$(python3 "$ROOT_DIR/memory-bank/scripts/validate.py" --summary "memory-bank" 2>/dev/null)
    echo "$MEMORY_SUMMARY Read memory-bank/README.md and INDEX.md before relevant work."
  else
    echo "Memory bank: available (counts unavailable). Read memory-bank/README.md and INDEX.md before relevant work."
  fi
fi

# Project structure
echo ""
echo "Structure:"
for DIR in src app public config bin templates views tests specs tasks memory-bank examples .cursor; do
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
