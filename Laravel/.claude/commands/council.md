---
spawns: council-agent
phase: planning
flow-next: architect
flow-alternatives: [researcher, writing-plans, architecture-implementer]
---

# Council

Spawn council agent to weigh a high-stakes or ambiguous decision from multiple expert perspectives (architecture, security, performance, testing, maintainability, pragmatism) and recommend an option.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `council`
- **description:** `Multi-perspective decision`
- **prompt:** `$ARGUMENTS`

The agent will use the council skill and suggest next steps when done.
