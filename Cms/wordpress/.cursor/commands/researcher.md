---
name: researcher
description: "Spawn researcher agent to investigate a question (libraries, approaches, standards, or codebase area), compare options against project constraints, and produce a sourced recommendation."
---

# Researcher

Spawn researcher agent to investigate a question (libraries, approaches, standards, or codebase area), compare options against project constraints, and produce a sourced recommendation.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `researcher`
- **description:** `Research options`
- **prompt:** `$ARGUMENTS`

The agent will use the researcher skill and suggest next steps when done.
