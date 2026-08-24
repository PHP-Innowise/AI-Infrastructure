---
spawns: coder-agent
phase: execution
flow-next: code-reviewer
flow-alternatives: [test-generator, security-reviewer, debugger]
---

# Coder

Spawn the `coder` agent with `$ARGUMENTS`. It executes only the
`coder` skill, returns its requested output and stops.

