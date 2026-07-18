---
name: browser-verify
description: "Spawn browser-verify agent to visually verify UI changes in the running Laravel app (Blade, Livewire, or Inertia.js)."
---

# Browser Verify

Spawn browser-verify agent to visually verify UI changes in the running Laravel app (Blade, Livewire, or Inertia.js).

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `browser-verify`
- **description:** `Verify UI changes in browser`
- **prompt:** `$ARGUMENTS`

The agent will open the app, verify the change, iterate fixes if needed, and suggest next steps when done.
