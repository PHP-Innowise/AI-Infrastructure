---
spawns: code-reviewer-agent
phase: quality
flow-next: verify
flow-alternatives: [coder, security-reviewer, performance-optimization]
---

# Code Reviewer

Spawn the `code-reviewer` agent with `$ARGUMENTS`. It executes only the
`code-reviewer` skill, returns its requested output and stops.

