---
spawns: coder-frontend-agent
phase: execution
flow-next: browser-verify
flow-alternatives: [test-generator, code-reviewer, verify]
---

# Coder Frontend

Spawn the `coder-frontend` agent with `$ARGUMENTS`. It executes only the
`coder-frontend` skill, returns its requested output and stops.

