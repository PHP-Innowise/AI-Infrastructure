---
name: project-brain
description: "Manage governed WordPress tasks, handoffs, records, retrieval, compaction, and promotion proposals."
model: sonnet
invokes: project-brain
phase: utility
writes: true
---

# Project Brain Agent

Invoke the `project-brain` skill, execute only that skill, and stop. Return the
skill's requested output, evidence where applicable, a concise Context Summary,
and next-step alternatives without automatically chaining another skill.

