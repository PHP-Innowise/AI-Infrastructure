#!/bin/sh
# Compatibility entrypoint for the native shared-core changelog gate.
exec "$(dirname -- "$0")/../infra-engine" repo changelog "$@"
