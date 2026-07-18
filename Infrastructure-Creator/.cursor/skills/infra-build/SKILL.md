---
name: infra-build
description: One-shot convenience orchestrator that chains infra-scan then infra-generate against a target PHP project, pausing at the profile checkpoint only when a blocking ambiguity or a collision is detected. Use when the user wants the whole build in one step and trusts the scan. Triggers on "infra-build", "scan and generate in one go", "build my accelerator end to end", "do the whole thing".
phase: orchestration
flow-next: null
flow-alternatives: [infra-scan, infra-generate]
related: [infra-scan, infra-generate, profile-synthesizer, bootstrap-verifier]
---

# Infra Build

## Overview

`infra-build` is the low-friction path (see `AGENTS.md`'s "Orchestration Exception"). It runs `infra-scan` and then `infra-generate` back-to-back so the user gets a finished accelerator from a single command. It preserves safety by pausing at the human checkpoint only when it must: if the scan produced blocking ambiguity (unresolved `unknown` items that change generation) or if the collision guard trips.

This skill is a convenience wrapper. It adds no generation logic of its own - it delegates entirely to the two phase orchestrators and enforces the checkpoint policy between them.

## Generated File Naming Convention (MANDATORY)

No new file naming of its own. It relies on `infra-scan` (which writes `tasks/TASK-{N}/...`) and `infra-generate` (which writes into the target and `tasks/TASK-{N}/infra-generate-report.md`). It appends a short `tasks/TASK-{N}/infra-build-report.md` summarizing the end-to-end run.

## Process

1. **Validate the target** exactly as `infra-scan` does (path exists, is PHP, is not this folder).
2. **Run `infra-scan`** against the target, producing the Project Profile.
3. **Evaluate the checkpoint gate:**
   - If the profile has unresolved `unknown` items that affect what gets generated, OR the AI-tool selection is somehow unset, STOP and hand the profile to the user for review before continuing.
   - Otherwise, surface a one-line summary of the profile and proceed.
4. **Run `infra-generate`** against the same target.
   - If the collision guard trips (target already has an accelerator), STOP and ask overwrite/merge/abort - never auto-decide.
5. **Verify** via `infra-generate`'s built-in `bootstrap-verifier` step; do not report success on failure.
6. **Report** the combined result in `tasks/TASK-{N}/infra-build-report.md`.

## Output Template

```markdown
# Infra Build Complete: [target_name]

**Target:** [target path]
**Paused for review:** [yes/no - reason if yes]
**Edition(s) generated:** [selected]
**Verification:** [pass/fail summary]

## Next
Open [target path]; start with [suggested first generated command].
```

## Guardrails

- MUST enforce the checkpoint when blocking ambiguity exists - ease of use never overrides safety.
- MUST honor the collision guard; MUST NOT auto-overwrite a pre-existing accelerator.
- MUST generate only the selected edition(s).
- MUST NOT report success while verification is failing.

## Final Output

Return whether it paused for review (and why), the editions generated, the verification result, and the first command the user should run in the target.
