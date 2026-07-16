---
name: database-designer
description: "Spawn database-designer agent to design a relational schema and Doctrine repository access patterns (tables, keys, indexing, constraints, normalization, migrations) for a Symfony project."
---

# Database Designer

Spawn database-designer agent to design a relational schema and Doctrine repository access patterns (tables, keys, indexing, constraints, normalization, migrations) for a Symfony project.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `database-designer`
- **description:** `Design database schema`
- **prompt:** `$ARGUMENTS`

The agent will use the database-designer skill and suggest next steps when done.
