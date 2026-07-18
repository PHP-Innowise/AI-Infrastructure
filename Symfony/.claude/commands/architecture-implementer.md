---
spawns: architecture-implementer-agent
phase: execution
flow-next: coder
flow-alternatives: [test-generator, code-reviewer, verify]
---

# Architecture Implementer

Spawn architecture-implementer agent to scaffold and wire an approved architecture into Symfony (Symfony controller-service-repository structure, controllers, services, repositories, DTOs, voters, messages, DI wiring, entry points), ready for feature code.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `architecture-implementer`
- **description:** `Scaffold architecture`
- **prompt:** `$ARGUMENTS`

The agent will use the architecture-implementer skill and suggest next steps when done.
