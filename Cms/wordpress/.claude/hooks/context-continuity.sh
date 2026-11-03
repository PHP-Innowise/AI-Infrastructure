#!/bin/sh
# Compatibility entrypoint; the complete hook implementation is Rust.
TOOLING_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P) || exit 127
exec "$TOOLING_ROOT/memory-bank/bin/infra-engine" hook claude context-continuity --root "$TOOLING_ROOT" "$@"
