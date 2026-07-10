---
spawns: filament-agent
phase: execution
flow-next: code-reviewer
flow-alternatives: [test-generator, browser-verify]
---

# Filament

Spawn filament agent to build or extend a Filament admin panel: Resources, Schemas (Forms/Infolists), Tables, Relation Managers, Actions, and Widgets.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `filament`
- **description:** `Build Filament admin panel feature`
- **prompt:** `$ARGUMENTS`

The agent will use the filament skill and suggest next steps when done.
