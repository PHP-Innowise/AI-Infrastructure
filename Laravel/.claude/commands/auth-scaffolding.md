---
spawns: auth-scaffolding-agent
phase: execution
flow-next: security-reviewer
flow-alternatives: [test-generator, code-reviewer]
---

# Auth Scaffolding

Spawn auth-scaffolding agent to set up Laravel web/session authentication - starter kits, multi-guard configuration, and Policy/Gate authorization patterns.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `auth-scaffolding`
- **description:** `Scaffold Laravel web authentication`
- **prompt:** `$ARGUMENTS`

The agent will use the auth-scaffolding skill and suggest next steps when done.
