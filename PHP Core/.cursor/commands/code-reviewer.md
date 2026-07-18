---
name: code-reviewer
description: "Spawn code-reviewer agent to review code for quality and issues."
---

# Code Reviewer

Spawn code-reviewer agent to review code for quality and issues.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `code-reviewer`
- **description:** `Review code quality`
- **prompt:** `$ARGUMENTS`

The agent will use the code-reviewer skill and suggest next steps when done.
