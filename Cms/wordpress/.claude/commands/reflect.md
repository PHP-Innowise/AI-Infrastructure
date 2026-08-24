---
spawns: reflect-agent
phase: utility
flow-next: null
flow-alternatives: [verify, skill-creator]
---

# Reflect

Turn agent mistakes into permanent rules. Automates the STABILIZATION.md cycle.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `reflect`
- **description:** `Stabilize error into rule`
- **prompt:** `$ARGUMENTS`

The agent will analyze the error, draft a rule, and apply it after approval.
