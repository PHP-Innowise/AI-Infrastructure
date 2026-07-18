---
name: infra-build
description: One-shot path that scans then generates in a single command, pausing only when a blocking ambiguity or a collision is detected.
---

# /infra-build

One-shot: scan then generate in a single command. Pauses only if a blocking ambiguity or a collision is detected.

Usage: `/infra-build <path-to-target-php-project>`

The target path is passed as `$ARGUMENTS` and is required. This spawns the `infra-build-agent`, which runs `infra-scan`, checks the profile at the human checkpoint (stopping only when needed), then runs `infra-generate`. Use this when you trust the scan and want the whole build in one step.
