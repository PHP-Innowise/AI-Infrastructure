#!/bin/sh
# Launcher; the complete hook implementation is Rust. It runs the engine of
# this checkout or, in a linked worktree, the one of its main checkout.
TOOLING_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P) || exit 127
ENGINE=$TOOLING_ROOT/memory-bank/bin/infra-engine
if [ ! -x "$ENGINE" ] && common=$(git -C "$TOOLING_ROOT" rev-parse --path-format=absolute --git-common-dir 2>/dev/null) && below=$(git -C "$TOOLING_ROOT" rev-parse --show-prefix 2>/dev/null) && main=$(CDPATH= cd -- "$(dirname -- "$common")/$below" && pwd -P); then
  TOOLING_ROOT=$main
  ENGINE=$TOOLING_ROOT/memory-bank/bin/infra-engine
fi
if [ ! -x "$ENGINE" ]; then
  exit 0
fi
exec "$ENGINE" hook claude subagent-dispatch --root "$TOOLING_ROOT" "$@"
