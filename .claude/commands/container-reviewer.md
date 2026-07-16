---
spawns: container-reviewer-agent
phase: quality
flow-next: verify
flow-alternatives: [coder, code-reviewer]
---

# Container Reviewer

Review Symfony service container config, autowiring, aliases, tags, decorators, env vars, and visibility.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn `container-reviewer`.
