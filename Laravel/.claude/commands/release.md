---
spawns: release-agent
phase: finalization
flow-next: finishing-branch
flow-alternatives: []
---

# Release

Spawn release agent to create GitHub releases with automated changelog generation.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `release`
- **description:** `Create GitHub release with changelog`
- **prompt:** `$ARGUMENTS`

The agent will use the release skill and suggest next steps when done.
