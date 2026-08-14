---
spawns: stack-adapter-agent
phase: orchestration
flow-next: null
flow-alternatives: [infra-scan]
---

# /infra-adapt

Build an independent sibling generator - `Infrastructure-Creator-[Stack]` - for a target project that is not PHP. Use this directly when you already know the target uses a different stack; otherwise `infra-scan` will offer this automatically when it detects a non-PHP stack.

Usage: `/infra-adapt <path-to-target-project>`

The adapter researches the stack, re-authors the skills and all six contract
catalogs, copies the generic evidence/semantic/staging quality architecture,
mirrors editions, runs negative/positive fixtures plus a synthetic generation
rehearsal, and rejects stubs or mechanical substitutions.
