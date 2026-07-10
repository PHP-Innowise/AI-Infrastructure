---
spawns: architect-agent
phase: planning
flow-next: api-designer
flow-alternatives: [architecture-implementer, writing-plans, coder]
---

# Architect

Spawn architect agent to make Laravel architecture decisions (Actions vs Services vs model logic, Eloquent boundaries, Service Provider bindings, queues/async, scalability, security).

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `architect`
- **description:** `Architecture decisions`
- **prompt:** `$ARGUMENTS`

The agent will use the architect skill and suggest next steps when done.
