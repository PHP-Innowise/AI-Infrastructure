---
name: architecture-implementer
description: "Spawn architecture-implementer agent to scaffold and wire an approved architecture into native PHP (PSR-4 structure, boundary interfaces, skeleton classes, DI wiring, entry points), ready for feature code."
---

# Architecture Implementer

Spawn architecture-implementer agent to scaffold and wire an approved architecture into native PHP (PSR-4 structure, boundary interfaces, skeleton classes, DI wiring, entry points), ready for feature code.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `architecture-implementer`
- **description:** `Scaffold architecture`
- **prompt:** `$ARGUMENTS`

The agent will use the architecture-implementer skill and suggest next steps when done.
