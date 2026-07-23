---
spawns: stack-adapter-agent
phase: orchestration
flow-next: null
flow-alternatives: [infra-scan]
---

# /infra-adapt

Build an independent sibling generator - `Infrastructure-Creator-[Stack]` - for a target project that is not PHP. Use this directly when you already know the target uses a different stack; otherwise `infra-scan` will offer this automatically when it detects a non-PHP stack.

Usage: `/infra-adapt <path-to-target-project>`

The target path is passed as `$ARGUMENTS` and is required. This spawns the `stack-adapter-agent`, which confirms the detected stack, guards against collisions, researches the stack live, replicates Infrastructure-Creator's own architecture, re-authors all 21 skills and reference docs for the new stack, mirrors the three editions, self-verifies, and reports the new generator's path and next command.
