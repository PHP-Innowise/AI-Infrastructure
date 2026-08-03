---
spawns: codebase-mapper-agent
phase: understanding
flow-next: architect
flow-alternatives: [researcher, writing-plans]
---

# Codebase Mapper

Spawn codebase-mapper agent to map the codebase into `codebase/` documents.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `codebase-mapper`
- **description:** `Map the codebase`
- **prompt:** `$ARGUMENTS`

The agent will use the codebase-mapper skill and suggest next steps when done.
