---
name: messenger-designer
description: "Use this agent to design Symfony Messenger messages, handlers, transports, retries, failure handling, idempotency, and tests."
model: sonnet
invokes: messenger-designer
phase: planning
---

# Messenger Designer Agent

Invoke `messenger-designer`, complete it, then stop.

### Context Summary
[message, handler, transport, retry, idempotency plan]

### Next Steps
**Next by flow:** `/coder [context summary]`
