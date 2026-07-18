---
spawns: infra-scan-agent
phase: orchestration
flow-next: infra-generate
flow-alternatives: [infra-build]
---

# /infra-scan

Scan a target PHP project and produce one reviewable Project Profile. Read-only on the target.

Usage: `/infra-scan <path-to-target-php-project>`

The target path is passed as `$ARGUMENTS` and is required. This spawns the `infra-scan-agent`, which fans out the six PHP scanners, runs grounded research, asks the minimal clarifying questions (including which AI tool your team uses), and synthesizes `tasks/TASK-{N}/infra-scan-project-profile.md`.

Review that profile, then run `/infra-generate <path>`.
