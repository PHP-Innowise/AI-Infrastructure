---
name: refactorer
description: "Spawn refactorer agent to perform behavior-preserving refactors or safe PHP upgrades under a test safety net (extract, de-duplicate, improve types, reviewed Rector rules)."
---

# Refactorer

Spawn refactorer agent to perform behavior-preserving refactors or safe PHP upgrades under a test safety net (extract, de-duplicate, improve types, reviewed Rector rules).

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `refactorer`
- **description:** `Refactor safely`
- **prompt:** `$ARGUMENTS`

The agent will use the refactorer skill and suggest next steps when done.
