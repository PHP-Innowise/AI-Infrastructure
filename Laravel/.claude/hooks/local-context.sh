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
  echo "No accelerator engine is installed for $TOOLING_ROOT: its hooks and memory are off. From an accelerator clone run: infra-engine install --edition <edition> --target <this checkout> --merge-existing"
  exit 0
fi
exec "$ENGINE" hook claude local-context --root "$TOOLING_ROOT" "$@"
