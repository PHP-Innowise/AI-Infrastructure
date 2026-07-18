---
spawns: database-designer-agent
phase: planning
flow-next: architecture-implementer
flow-alternatives: [coder, writing-plans, api-designer]
---

# Database Designer

Spawn database-designer agent to design a relational schema and safe Eloquent access patterns (tables, keys, indexing, constraints, normalization, migrations) for a Laravel project.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `database-designer`
- **description:** `Design database schema`
- **prompt:** `$ARGUMENTS`

The agent will use the database-designer skill and suggest next steps when done.
