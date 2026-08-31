---
spawns: plugin-development-agent
phase: execution
flow-next: code-reviewer
flow-alternatives: [test-generator, security-reviewer, verify]
---

# Plugin Development

Spawn `plugin-development` with `$ARGUMENTS`. It executes only the
`plugin-development` skill and returns its result and next-step alternatives.
