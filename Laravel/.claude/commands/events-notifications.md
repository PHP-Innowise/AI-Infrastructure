---
spawns: events-notifications-agent
phase: execution
flow-next: test-generator
flow-alternatives: [code-reviewer]
---

# Events & Notifications

Spawn events-notifications agent to design and implement Laravel Events, Listeners, model Observers, Notifications (mail/database/broadcast/Slack), and Mailables.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `events-notifications`
- **description:** `Implement Events, Listeners, and Notifications`
- **prompt:** `$ARGUMENTS`

The agent will use the events-notifications skill and suggest next steps when done.
