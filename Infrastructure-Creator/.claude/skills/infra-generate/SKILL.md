---
name: infra-generate
description: Turn an approved Project Profile into a real, working accelerator inside the target PHP project - policy, skills, agents, commands, hooks, and a seeded memory bank - for only the selected AI-tool edition(s). Use after infra-scan has produced and the user has reviewed tasks/TASK-{N}/infra-scan-project-profile.md. Triggers on "infra-generate", "generate the accelerator now", "build the infrastructure for my project", "run phase two".
phase: orchestration
flow-next: null
flow-alternatives: []
related: [policy-forge, skill-forge, agent-forge, command-forge, hook-forge, memory-seed, skill-flow-composer, bootstrap-verifier, infra-scan, infra-update]
---

# Infra Generate

## Overview

`infra-generate` is the second orchestrator (see `AGENTS.md`'s "Orchestration Exception"). It is the only point in the pipeline that writes into the target project. It consumes an approved `infra-scan-project-profile.md`, fans out the forge skills, wraps their output with agents/commands, composes the flow, and runs a final verification pass before declaring the target's new accelerator ready - writing only the AI-tool edition(s) the profile selected.

## Generated File Naming Convention (MANDATORY)

This skill's own run notes live in `tasks/TASK-{N}/infra-generate-report.md`. Everything else it produces is written into the **target project's own root**, using the conventions that `policy-forge` just wrote for it.

## Process

1. **Locate the profile.** Require the target project path (must match a profile from `infra-scan`); if more than one `TASK-{N}/` exists for that target, use the most recent unless the user specifies one.
2. **Re-validate before trusting it.** Re-check cited evidence against current files, including section 8's canonical behavioral sources and section 12's memory concepts. Preserve contradictions and source type. If something changed, flag drift and ask whether to re-scan or explicitly accept stale parts.
3. **Read the selected editions** from the profile's section 1 (AI Tool Selection). Only these editions will be produced.
4. **Collision guard and write plan.** If the target already has `AGENTS.md` or any selected edition folder, STOP and ask: overwrite, merge (add only what is missing, never touch existing files), or abort. Do not proceed on assumption. Before any forge writes, create `tasks/TASK-{N}/infra-generate-write-plan.txt`. Every forge MUST append the target-relative path of each file it actually creates or replaces, exactly once. In merge mode, snapshot the pre-existing surface in `tasks/TASK-{N}/preexisting-files.txt` first and never write or add any listed path to the write plan. In full mode, append only paths this run actually writes - never infer ownership afterward by walking a managed root. Runtime state excluded below is never added.
5. **Fan out the four independent forges.**
   - **Parallel-capable tools:** spawn `policy-forge`, `skill-forge`, `hook-forge`, and `memory-seed` together - none need each other's output, only the profile.
   - **Single-threaded tools:** run the same four sequentially in one session.
6. **Wrap the generated skills.** Once `skill-forge` has produced the final skill list, run `agent-forge` (generates matching agent wrappers for editions that carry them), then `command-forge` (wraps for editions with a command layer; skipped for Codex).
7. **Compose the flow.** Run `skill-flow-composer` once every skill/agent/command exists, to build the target's own `SKILL FLOW.md`.
8. **Stamp and write the manifest.** Once every forge is done and before verification (see "Version Stamp & Generation Manifest" below): if this run wrote `AGENTS.md`, prepend its version-stamp comment and ensure `AGENTS.md` is in the write plan. Then write `.infra-manifest.json` from that explicit plan using the helper below. `infra-update` depends on this manifest - a generation without it is a legacy target that can never be upgraded safely.
9. **Verify.** Run `bootstrap-verifier` last - it checks manifest schema, membership, hashes, tracked `AGENTS.md` stamping, and placeholders in manifest-owned text only. Treat a failed verification as generation not done - auto-fix what is safe (e.g. a missing executable bit), refresh only the affected manifest-owned hash after any content auto-fix, and re-run; escalate anything it cannot safely fix (e.g. a dangling cross-reference) to the user.
10. **Report.** Write `tasks/TASK-{N}/infra-generate-report.md` summarizing what was written where, for which edition(s), the manifest file count, and the verification results.

## Version Stamp & Generation Manifest (MANDATORY)

The generator's version has a single source of truth: the `VERSION` file at this generator's root. Never hardcode a version and never recall one from the changelog.

**Step A - stamp `AGENTS.md`.** Applies only to an `AGENTS.md` this run created: in merge mode a pre-existing `AGENTS.md` is never touched (it stays unstamped and untracked; `bootstrap-verifier` skips the stamp check for it). When this run did create it, make the FIRST line of the target's generated `AGENTS.md` this comment (real values, no placeholders), before hashing anything:

```markdown
<!-- Generated by Infrastructure-Creator v1.4.0 | TASK-003 | 2026-08-02 -->
```

- version = contents of this generator's `VERSION` file;
- task id = this run's `tasks/TASK-{N}/`;
- date = today, ISO `YYYY-MM-DD`.

**Step B - write `.infra-manifest.json`.** The manifest records the generator version, the profile this run consumed, the generation `mode` (`full` or `merge`), and the sha256 of exactly the paths in `infra-generate-write-plan.txt`. Manifest membership is the sole ownership authority in both modes. Runtime state is deliberately NOT tracked - `memory-bank/chunks/`, `memory-bank/INDEX.md`, `memory-bank/.memory-counter`, `memory-bank/local/`, `project-brain/indexes/`, and the `project-brain/` record/state directories (`archive/`, `control/`, `dynamic/`, `local/`) belong to the target team from the moment they are seeded, and `infra-update` never touches a file the manifest does not list.

Exact recipe - run from this generator's root after Step A, substituting the target path, task, selected editions, mode, and write-plan path:

```bash
python3 .agents/skills/bootstrap-verifier/scripts/infra_ownership.py manifest \
  --target "<path-to-target>" \
  --write-plan "tasks/TASK-003/infra-generate-write-plan.txt" \
  --version "$(cat VERSION)" \
  --task "TASK-003" \
  --profile "tasks/TASK-003/infra-scan-project-profile.md" \
  --editions "claude,cursor" \
  --mode "full"
```

The helper hashes only explicit write-plan members and refuses missing files, absolute/traversal paths, caches, the manifest itself, and runtime state. It never walks the target's managed roots. Consequently, team files absent from the plan remain untracked and invisible to `infra-update` in both full and merge modes. `validate_generated.py` validates every tracked file's existence/hash and scans every tracked text file for placeholders, while ignoring unmanifested team files.

## Output Template

```markdown
# Infra Generate Complete: [target_name]

**Target:** [target path]
**Edition(s) generated:** [only the selected ones]
**Skills generated:** [count] ([list, including operational memory-bank and any domain skills])

## Verification
[bootstrap-verifier pass/fail summary]

## Upgrade Contract
`.infra-manifest.json` written: [file count] files, mode [full / merge], generator version [from VERSION], profile [TASK id]. `AGENTS.md` [stamped / pre-existing, untouched]. Future generator releases can be applied with `infra-update`.

## What To Do Next
The target now has its own working `AGENTS.md` + [selected edition folder(s)] + `memory-bank/`. Open [target path] and start with [suggested first generated command].

## Risks / Follow-Ups
[Anything bootstrap-verifier flagged, unresolved behavioral contradictions/unknowns, memory drift]
```

## Guardrails

- MUST NOT write into the target without the collision guard passing (explicit overwrite/merge/abort).
- MUST generate ONLY the selected edition(s) - never an unselected edition, never skip a selected one.
- MUST NOT invent content beyond what the profile supports - a forge needing missing data is a signal to re-scan/re-interview, not to guess.
- MUST stamp `AGENTS.md` only when this run wrote it, and write `.infra-manifest.json` from `infra-generate-write-plan.txt` before running `bootstrap-verifier`, taking the version only from this generator's root `VERSION` file.
- MUST NOT list runtime state (memory chunks, indexes, counters, `local/` dirs, Project Brain records) in the manifest - `infra-update` treats everything the manifest lists as generator-owned.
- MUST maintain an explicit generated write plan in both modes. In merge mode, snapshot `preexisting-files.txt` before any forge writes, record `"mode": "merge"`, and never write or track a pre-existing team file. In full mode, track only files actually written; never infer ownership with a target-root walk.
- MUST NOT report success while `bootstrap-verifier` has unresolved failures.

## Final Output

Return what was generated, where, for which edition(s), the verification results, remaining risks, and the concrete first command the target team should run inside their project.
