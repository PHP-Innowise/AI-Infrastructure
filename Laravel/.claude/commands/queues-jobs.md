---
spawns: queues-jobs-agent
phase: execution
flow-next: test-generator
flow-alternatives: [code-reviewer, verify]
---

# Queues & Jobs

Spawn queues-jobs agent to design and implement Laravel queued Jobs: job classes, job middleware, unique jobs, batching, chaining, failed-job handling and retries, and Horizon configuration.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `queues-jobs`
- **description:** `Implement Laravel queued job`
- **prompt:** `$ARGUMENTS`

The agent will use the queues-jobs skill and suggest next steps when done.
