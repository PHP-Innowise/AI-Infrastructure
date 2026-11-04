#!/bin/sh
# Launcher; the complete hook implementation is Rust. It runs the engine of
# this checkout or, in a linked worktree, the one of its main checkout.
case $0 in */*) TOOLING_ROOT=${0%/*}/../.. ;; *) TOOLING_ROOT=../.. ;; esac
TOOLING_ROOT=$(CDPATH= cd -- "$TOOLING_ROOT" && pwd -P) || exit 127
ENGINE=$TOOLING_ROOT/memory-bank/bin/infra-engine
if [ ! -x "$ENGINE" ] && common=$(git -C "$TOOLING_ROOT" rev-parse --path-format=absolute --git-common-dir 2>/dev/null) && below=$(git -C "$TOOLING_ROOT" rev-parse --show-prefix 2>/dev/null) && main=$(CDPATH= cd -- "$(dirname -- "$common")/$below" && pwd -P); then
  TOOLING_ROOT=$main
  ENGINE=$TOOLING_ROOT/memory-bank/bin/infra-engine
fi
if [ ! -x "$ENGINE" ]; then
  echo "BLOCKED: no accelerator engine is installed for $TOOLING_ROOT. From an accelerator clone run: infra-engine install --edition <edition> --target <this checkout> --merge-existing" >&2
  exit 2
fi
exec "$ENGINE" hook claude bash-validator --root "$TOOLING_ROOT" "$@"
