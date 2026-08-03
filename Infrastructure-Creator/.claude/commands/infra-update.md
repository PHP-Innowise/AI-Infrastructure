---
spawns: infra-update-agent
phase: orchestration
flow-next: null
flow-alternatives: []
---

# /infra-update

Upgrade a previously generated accelerator to this generator's current version without losing the target team's local edits.

Usage: `/infra-update <path-to-target-php-project>`

The target path is passed as `$ARGUMENTS` and is required. This spawns the `infra-update-agent`, which reads the target's `.infra-manifest.json`, re-validates the profile, regenerates into staging, replaces only files untouched since generation (sha256 match against the manifest), honors standing keep/merge decisions recorded in the manifest's `decisions` map (a file you chose to keep is never silently replaced on a later run), routes every user-modified file to you for an explicit decision, rewrites the manifest, and re-runs `bootstrap-verifier`.

Targets generated before manifests existed (generator v1.3.x or earlier) have no `.infra-manifest.json`; the run aborts with recovery options instead of guessing which files are yours.
