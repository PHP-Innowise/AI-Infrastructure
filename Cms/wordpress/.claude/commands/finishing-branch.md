---
spawns: finishing-branch-agent
phase: execution
flow-next:
flow-alternatives: [docs-generator]
---

# Finishing Branch

Spawn finishing-branch agent to complete development work.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `finishing-branch`
- **description:** `Complete branch work`
- **prompt:** `$ARGUMENTS`

The agent will use the finishing-branch skill and suggest next steps when done.
