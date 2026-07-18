---
spawns: console-scheduler-agent
phase: execution
flow-next: test-generator
flow-alternatives: [code-reviewer, verify]
---

# Console Scheduler

Spawn console-scheduler agent to build a custom Artisan console command and/or schedule a recurring task: command signature/output, routes/console.php scheduling, overlap prevention, and failure handling.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `console-scheduler`
- **description:** `Build Artisan command and schedule`
- **prompt:** `$ARGUMENTS`

The agent will use the console-scheduler skill and suggest next steps when done.
