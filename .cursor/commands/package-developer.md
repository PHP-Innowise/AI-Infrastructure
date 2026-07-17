---
name: package-developer
description: "Spawn package-developer agent to build or maintain a reusable Composer/Laravel package - service provider structure, publishing, and Testbench testing."
---

# Package Developer

Spawn package-developer agent to build or maintain a reusable Composer/Laravel package - service provider structure, publishing, and Testbench testing.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `package-developer`
- **description:** `Build Composer package`
- **prompt:** `$ARGUMENTS`

The agent will use the package-developer skill and suggest next steps when done.
