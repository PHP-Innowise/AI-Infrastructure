---
name: memory-bank
description: "Manage durable, source-backed WordPress project memory without storing active task state."
model: sonnet
invokes: memory-bank
phase: utility
writes: true
---

# Memory Bank Agent

Invoke the `memory-bank` skill, execute only that skill, and stop. Return the
skill's requested output, evidence where applicable, a concise Context Summary,
and next-step alternatives without automatically chaining another skill.

