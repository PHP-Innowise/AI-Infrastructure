---
spawns: infra-update-agent
phase: orchestration
flow-next: null
flow-alternatives: []
---

# /infra-update

Upgrade a previously generated accelerator to this generator's current version without losing the target team's local edits.

Usage: `/infra-update <path-to-target-php-project>`

The agent reads `.infra-manifest.json`, re-validates the Profile and generation
plan, runs the current semantic/routing gates in staging, honors manifest
ownership and standing decisions, applies approved changes with rollback, and
verifies the published target.

Targets generated before manifests existed (generator v1.3.x or earlier) have no `.infra-manifest.json`; the run aborts with recovery options instead of guessing which files are yours.
