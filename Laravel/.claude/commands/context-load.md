---
name: context-load
description: "Read and validate an explicit saved continuation handoff for the current task."
---

# Context Load

Read `.claude/skills/context-load/SKILL.md`, execute its one read-only workflow
in the current agent, and stop. Do not continue work unless the user's current
request asks for it.
