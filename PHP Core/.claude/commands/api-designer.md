---
spawns: api-designer-agent
phase: planning
flow-next: frontend-design
flow-alternatives: [writing-plans, architecture-implementer, coder]
---

# API Designer

Spawn api-designer agent to design native PHP REST APIs with routing, PSR-7 requests/responses, input validation, response serializers, authorization, pagination, error contracts, and OpenAPI docs.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `api-designer`
- **description:** `Design REST API`
- **prompt:** `$ARGUMENTS`

The agent will use the api-designer skill and suggest next steps when done.
