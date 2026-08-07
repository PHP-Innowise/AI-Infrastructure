---
spawns: memory-bank-agent
phase: utility
flow-next: null
flow-alternatives: [project-brain, docs-generator, reflect]
---

# Memory Bank

Retrieve, capture, audit, supersede/archive durable memory, or apply a governed automatic or independently reviewed Project Brain promotion.

## Input

$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:

- **subagent_type:** `memory-bank`
- **description:** `Manage durable project memory`
- **prompt:** `$ARGUMENTS`

The agent must execute one memory mode and stop.
