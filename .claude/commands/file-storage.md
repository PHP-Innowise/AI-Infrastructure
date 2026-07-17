---
spawns: file-storage-agent
phase: execution
flow-next: security-reviewer
flow-alternatives: [test-generator, code-reviewer]
---

# File Storage

Spawn file-storage agent to implement Laravel file storage and uploads - disk configuration, secure upload handling, and signed/temporary URLs.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `file-storage`
- **description:** `Implement file storage feature`
- **prompt:** `$ARGUMENTS`

The agent will use the file-storage skill and suggest next steps when done.
