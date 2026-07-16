---
spawns: architecture-boundary-reviewer-agent
phase: quality
flow-next: code-reviewer
flow-alternatives: [refactorer, coder, verify]
---

# Architecture Boundary Reviewer

Review Symfony Controller -> Service -> Repository boundaries and SOLID violations.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `architecture-boundary-reviewer`
- **description:** `Review layer boundaries`
- **prompt:** `$ARGUMENTS`
