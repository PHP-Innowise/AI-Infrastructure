---
name: infra-build
description: One-shot scan, contract compilation, staged semantic generation, and transactional publication with required ambiguity/collision checkpoints.
---

# /infra-build

One-shot: scan then generate in a single command. Pauses only if a blocking ambiguity or a collision is detected.

Usage: `/infra-build <path-to-target-php-project>`

The target path is passed as `$ARGUMENTS` and is required. The build checks both
the human Profile and machine-readable per-skill plan at the checkpoint, then
runs the same staged semantic gates and transactional publication as
`infra-generate`.
