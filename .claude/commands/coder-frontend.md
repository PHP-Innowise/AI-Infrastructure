---
spawns: coder-frontend-agent
phase: execution
flow-next: code-reviewer
flow-alternatives: [browser-verify, test-generator]
---

# Coder Frontend

Spawn coder-frontend agent to implement frontend features.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `coder-frontend`
- **description:** `Implement frontend feature`
- **prompt:** `$ARGUMENTS`

The agent will use the coder-frontend skill and suggest next steps when done.
