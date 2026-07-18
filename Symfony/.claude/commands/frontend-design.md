---
spawns: frontend-design-agent
phase: planning
flow-next: writing-plans
flow-alternatives: [coder-frontend]
---

# Frontend Design

Spawn frontend-design agent to create distinctive UI designs.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `frontend-design`
- **description:** `Design frontend UI`
- **prompt:** `$ARGUMENTS`

The agent will use the frontend-design skill and suggest next steps when done.
