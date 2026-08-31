---
name: requirements-analyst
description: "Spawn requirements-analyst agent to analyze and decompose requirements."
---

# Requirements Analyst

Spawn requirements-analyst agent to analyze and decompose requirements.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `requirements-analyst`
- **description:** `Analyze requirements`
- **prompt:** `$ARGUMENTS`

The agent will use the requirements-analyst skill and suggest next steps when done.
