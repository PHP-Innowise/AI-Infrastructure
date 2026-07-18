---
name: test-generator
description: "Spawn test-generator agent to create comprehensive tests."
---

# Test Generator

Spawn test-generator agent to create comprehensive tests.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `test-generator`
- **description:** `Generate tests`
- **prompt:** `$ARGUMENTS`

The agent will use the test-generator skill and suggest next steps when done.
