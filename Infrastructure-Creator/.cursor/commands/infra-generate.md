---
name: infra-generate
description: Turn an approved Project Profile into a working accelerator inside the target PHP project, for the selected AI-tool edition(s) only.
---

# /infra-generate

Turn an approved Project Profile into a working accelerator inside the target PHP project, for only the selected AI-tool edition(s).

Usage: `/infra-generate <path-to-target-php-project>`

The target path is passed as `$ARGUMENTS` and is required. This spawns the `infra-generate-agent`, which re-validates the profile, runs the collision guard, fans out the forges, wraps skills as agents/commands, composes the flow, and runs `bootstrap-verifier`. Run `/infra-scan <path>` first and review its profile.
