---
name: brainstorm
description: "Spawn brainstorming agent to explore ideas and create designs."
---

# Brainstorming

Spawn brainstorming agent to explore ideas and create designs.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `brainstorming`
- **description:** `Brainstorm design`
- **prompt:** `$ARGUMENTS`

The agent will use the brainstorming skill and suggest next steps when done.
