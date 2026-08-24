---
name: docs-generator
description: "Spawn documentation-generator agent to create and maintain documentation."
---

# Documentation Generator

Spawn documentation-generator agent to create and maintain documentation.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `documentation-generator`
- **description:** `Generate documentation`
- **prompt:** `$ARGUMENTS`

The agent will use the documentation-generator skill and suggest next steps when done.
