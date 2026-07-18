---
spawns: memory-bank-agent
phase: utility
flow-next: null
flow-alternatives: [docs-generator, reflect, architect]
---

# Memory Bank

Retrieve, capture, audit, supersede, archive, or initialize durable project memory.

## Input

$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:

- **subagent_type:** `memory-bank`
- **description:** `Manage durable project memory`
- **prompt:** `$ARGUMENTS`

The agent must execute one memory mode and stop.
