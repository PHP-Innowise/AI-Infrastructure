---
spawns: project-brain-agent
phase: utility
flow-next: null
flow-alternatives: [memory-bank, docs-generator, reflect]
---

# Project Brain

Manage governed shared tasks, handoffs, retrieval, records, compaction, or promotion proposals.

## Input

$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:

- **subagent_type:** `project-brain`
- **description:** `Manage governed project context`
- **prompt:** `$ARGUMENTS`

The agent must execute one Project Brain operation and stop.
