---
spawns: rest-api-agent
phase: execution
flow-next: security-reviewer
flow-alternatives: [test-generator, documentation-generator, verify]
---

# WordPress REST API

Spawn `rest-api` with `$ARGUMENTS`. It executes only the `rest-api` skill and
stops.
