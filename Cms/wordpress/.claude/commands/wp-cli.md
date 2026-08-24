---
spawns: wp-cli-agent
phase: execution
flow-next: verify
flow-alternatives: [test-generator, code-reviewer, debugger]
---

# WP-CLI

Spawn `wp-cli` with `$ARGUMENTS`. It executes only the `wp-cli` skill and
stops.
