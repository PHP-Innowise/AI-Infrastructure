---
spawns: documentation-generator-agent
phase: finalization
flow-next: release
flow-alternatives: [finishing-branch]
---

# Documentation Generator

Spawn documentation-generator agent to create and maintain documentation.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `documentation-generator`
- **description:** `Generate documentation`
- **prompt:** `$ARGUMENTS`

The agent will use the documentation-generator skill and suggest next steps when done.
