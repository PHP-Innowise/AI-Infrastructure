---
name: debugger
description: "Spawn systematic-debugger agent to find root cause before fixing bugs."
---

# Systematic Debugger

Spawn systematic-debugger agent to find root cause before fixing bugs.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `systematic-debugger`
- **description:** `Debug issue`
- **prompt:** `$ARGUMENTS`

The agent will use the systematic-debugger skill and suggest next steps when done.
