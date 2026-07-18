---
spawns: writing-plans-agent
phase: planning
flow-next: git-worktrees
flow-alternatives: [coder, coder-frontend]
---

# Writing Plans

Spawn writing-plans agent to create detailed implementation plans.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `writing-plans`
- **description:** `Create implementation plan`
- **prompt:** `$ARGUMENTS`

The agent will use the writing-plans skill and suggest next steps when done.
