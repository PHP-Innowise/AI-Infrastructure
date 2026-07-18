---
spawns: refactorer-agent
phase: execution
flow-next: verify
flow-alternatives: [code-reviewer, test-generator, performance-optimization]
---

# Refactorer

Spawn refactorer agent to perform behavior-preserving refactors or safe Laravel/PHP upgrades under a test safety net (extract Actions/Services, de-duplicate, improve types, reviewed Rector/rector-laravel rules).

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `refactorer`
- **description:** `Refactor safely`
- **prompt:** `$ARGUMENTS`

The agent will use the refactorer skill and suggest next steps when done.
