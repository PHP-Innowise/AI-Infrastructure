---
name: skill-creator
description: "Spawn skill-creator agent to create or update Claude skills."
---

# Skill Creator

Spawn skill-creator agent to create or update Claude skills.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `skill-creator`
- **description:** `Create Claude skill`
- **prompt:** `$ARGUMENTS`

The agent will use the skill-creator skill and suggest next steps when done.
