---
name: infra-scan
description: Scan a target PHP project read-only and compile a human Profile plus validated evidence and per-skill generation contracts.
---

# /infra-scan

Scan a target PHP project and produce a human Project Profile plus a validated
evidence ledger and per-skill generation plan. Read-only on the target.

Usage: `/infra-scan <path-to-target-php-project>`

The target path is passed as `$ARGUMENTS` and is required. The scan compiles
`infra-scan-project-profile.md` and `skill-generation-plan.json`, pruning
unsupported or overlapping skills before generation.

Review that profile, then run `/infra-generate <path>`.
