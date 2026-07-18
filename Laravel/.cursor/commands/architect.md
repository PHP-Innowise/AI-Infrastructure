---
name: architect
description: "Spawn architect agent to make Laravel architecture decisions (Actions vs Services vs model logic, Eloquent boundaries, Service Provider bindings, queues/async, scalability, security)."
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
