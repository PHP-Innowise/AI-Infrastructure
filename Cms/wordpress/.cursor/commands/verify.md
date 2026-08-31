---
name: verify
description: "Run full Definition of Done checklist and report pass/fail status."
---

# Verify

Run full Definition of Done checklist and report pass/fail status.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `verify`
- **description:** `Run DoD verification`
- **prompt:** `$ARGUMENTS`

The agent will run all applicable checks and suggest next steps when done.
