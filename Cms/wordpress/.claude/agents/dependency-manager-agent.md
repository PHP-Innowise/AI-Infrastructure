---
name: dependency-manager
description: "Manage WordPress Composer, npm, WordPress.org, bundled, build, and release dependencies safely."
model: sonnet
invokes: dependency-manager
phase: quality
writes: true
---

# Dependency Manager Agent

Invoke the `dependency-manager` skill, execute only that skill, and stop. Return the
skill's requested output, evidence where applicable, a concise Context Summary,
and next-step alternatives without automatically chaining another skill.

