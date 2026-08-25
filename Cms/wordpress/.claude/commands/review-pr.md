---
spawns: review-pr-agent
phase: execution
flow-next: debugger
flow-alternatives: [coder, code-reviewer]
---

# PR Reviewer

Spawn review-pr agent to review a GitHub pull request using gh CLI.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `review-pr`
- **description:** `Review GitHub PR`
- **prompt:** `$ARGUMENTS`

The agent will use the review-pr skill and suggest next steps when done.
