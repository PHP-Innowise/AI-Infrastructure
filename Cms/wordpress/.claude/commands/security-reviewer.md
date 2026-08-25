---
spawns: security-reviewer-agent
phase: quality
flow-next: verify
flow-alternatives: [coder, debugger, test-generator]
---

# Security Reviewer

Spawn the `security-reviewer` agent with `$ARGUMENTS`. It executes only the
`security-reviewer` skill, returns its requested output and stops.

