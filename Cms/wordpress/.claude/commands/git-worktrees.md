---
spawns: using-git-worktrees-agent
phase: execution
flow-next: coder
flow-alternatives: [coder-frontend]
---

# Git Worktrees

Spawn using-git-worktrees agent to create isolated git workspaces.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `using-git-worktrees`
- **description:** `Create git worktree`
- **prompt:** `$ARGUMENTS`

The agent will use the using-git-worktrees skill and suggest next steps when done.
