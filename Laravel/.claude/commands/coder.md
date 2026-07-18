---
spawns: coder-agent
phase: execution
flow-next: code-reviewer
flow-alternatives: [test-generator]
---

# Coder Backend

Spawn coder agent to implement Laravel backend features using routes/controllers, Form Request validation, Eloquent models, Actions/Services, migrations, and tests.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `coder`
- **description:** `Implement backend feature`
- **prompt:** `$ARGUMENTS`

The agent will use the coder skill and suggest next steps when done.
