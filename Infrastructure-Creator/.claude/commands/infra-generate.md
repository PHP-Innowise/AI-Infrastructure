---
spawns: infra-generate-agent
phase: orchestration
flow-next: null
flow-alternatives: []
---

# /infra-generate

Build the approved evidence-contracted accelerator in staging, validate it, and
publish only the selected editions transactionally.

Usage: `/infra-generate <path-to-target-php-project>`

The agent re-validates profile evidence and per-skill contracts, rejects generic
or duplicated staged skills, derives routing, verifies the complete bundle,
then publishes explicit paths with rollback and verifies the target again.
