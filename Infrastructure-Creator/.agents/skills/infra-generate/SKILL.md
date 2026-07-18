---
name: infra-generate
description: Turn an approved Project Profile into a real, working accelerator inside the target PHP project - policy, skills, agents, commands, hooks, and a seeded memory bank - for only the selected AI-tool edition(s). Use after infra-scan has produced and the user has reviewed tasks/TASK-{N}/infra-scan-project-profile.md. Triggers on "infra-generate", "generate the accelerator now", "build the infrastructure for my project", "run phase two".
phase: orchestration
flow-next: null
flow-alternatives: []
related: [policy-forge, skill-forge, agent-forge, command-forge, hook-forge, memory-seed, skill-flow-composer, bootstrap-verifier, infra-scan]
---

# Infra Generate

## Overview

`infra-generate` is the second orchestrator (see `AGENTS.md`'s "Orchestration Exception"). It is the only point in the pipeline that writes into the target project. It consumes an approved `infra-scan-project-profile.md`, fans out the forge skills, wraps their output with agents/commands, composes the flow, and runs a final verification pass before declaring the target's new accelerator ready - writing only the AI-tool edition(s) the profile selected.

## Generated File Naming Convention (MANDATORY)

This skill's own run notes live in `tasks/TASK-{N}/infra-generate-report.md`. Everything else it produces is written into the **target project's own root**, using the conventions that `policy-forge` just wrote for it.

## Process

1. **Locate the profile.** Require the target project path (must match a profile from `infra-scan`); if more than one `TASK-{N}/` exists for that target, use the most recent unless the user specifies one.
2. **Re-validate before trusting it.** Re-check the profile's cited evidence against the target's *current* files. If something changed since the scan (a dependency removed, a file moved), flag the drift in `infra-generate-report.md` and ask whether to re-scan or proceed with the stale parts explicitly accepted.
3. **Read the selected editions** from the profile's section 1 (AI Tool Selection). Only these editions will be produced.
4. **Collision guard.** If the target already has `AGENTS.md` or any selected edition folder, STOP and ask: overwrite, merge (add only what is missing, never touch existing files), or abort. Do not proceed on assumption.
5. **Fan out the four independent forges.**
   - **Parallel-capable tools:** spawn `policy-forge`, `skill-forge`, `hook-forge`, and `memory-seed` together - none need each other's output, only the profile.
   - **Single-threaded tools:** run the same four sequentially in one session.
6. **Wrap the generated skills.** Once `skill-forge` has produced the final skill list, run `agent-forge` (generates matching agent wrappers for editions that carry them), then `command-forge` (wraps for editions with a command layer; skipped for Codex).
7. **Compose the flow.** Run `skill-flow-composer` once every skill/agent/command exists, to build the target's own `SKILL FLOW.md`.
8. **Verify.** Run `bootstrap-verifier` last. Treat a failed verification as generation not done - auto-fix what is safe (e.g. a missing executable bit) and re-run; escalate anything it cannot safely fix (e.g. a dangling cross-reference) to the user.
9. **Report.** Write `tasks/TASK-{N}/infra-generate-report.md` summarizing what was written where, for which edition(s), and the verification results.

## Output Template

```markdown
# Infra Generate Complete: [target_name]

**Target:** [target path]
**Edition(s) generated:** [only the selected ones]
**Skills generated:** [count] ([list])

## Verification
[bootstrap-verifier pass/fail summary]

## What To Do Next
The target now has its own working `AGENTS.md` + [selected edition folder(s)] + `memory-bank/`. Open [target path] and start with [suggested first generated command].

## Risks / Follow-Ups
[Anything bootstrap-verifier flagged, any `unknown` items still unresolved]
```

## Guardrails

- MUST NOT write into the target without the collision guard passing (explicit overwrite/merge/abort).
- MUST generate ONLY the selected edition(s) - never an unselected edition, never skip a selected one.
- MUST NOT invent content beyond what the profile supports - a forge needing missing data is a signal to re-scan/re-interview, not to guess.
- MUST NOT report success while `bootstrap-verifier` has unresolved failures.

## Final Output

Return what was generated, where, for which edition(s), the verification results, remaining risks, and the concrete first command the target team should run inside their project.
