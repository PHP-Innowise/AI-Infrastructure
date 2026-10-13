#!/bin/bash
# Automatic local conversation continuity. The client supplies hook JSON on
# stdin; successful capture is silent and restore uses native context output.
# This optional hook always fails open and never drives another model turn.
set -u

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd) || exit 0
RUNTIME="$ROOT_DIR/memory-bank/scripts/context_continuity.py"
command -v python3 >/dev/null 2>&1 || exit 0
[ -f "$RUNTIME" ] || exit 0
case "${1:-}" in
  capture) ARGS=(--event capture) ;;
  restore) ARGS=(--event restore --json) ;;
  *) exit 0 ;;
esac
ARGS+=(--root "$ROOT_DIR" --host codex)
if command -v timeout >/dev/null 2>&1; then
  timeout "${CONTEXT_HOOK_BUDGET:-5}" python3 "$RUNTIME" "${ARGS[@]}" 2>/dev/null
else
  python3 "$RUNTIME" "${ARGS[@]}" 2>/dev/null
fi
exit 0
