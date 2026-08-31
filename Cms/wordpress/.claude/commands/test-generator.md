---
spawns: test-generator-agent
phase: quality
flow-next: verify
flow-alternatives: [coder, debugger, code-reviewer]
---

# Test Generator

Spawn the `test-generator` agent with `$ARGUMENTS`. It executes only the
`test-generator` skill, returns its requested output and stops.

