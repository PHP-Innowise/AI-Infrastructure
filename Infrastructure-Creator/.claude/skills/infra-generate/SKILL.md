---
name: infra-generate
description: Compile an approved Project Profile and skill-generation plan into a semantically validated accelerator in task staging, then publish selected editions transactionally with rollback. Use after reviewing both Phase 1 artifacts.
phase: orchestration
flow-next: null
flow-alternatives: []
related: [policy-forge, skill-forge, agent-forge, command-forge, hook-forge, memory-seed, skill-flow-composer, bootstrap-verifier, infra-scan, infra-update]
---

# Infra Generate

## Overview

`infra-generate` is the second orchestrator (see `AGENTS.md`'s
"Orchestration Exception"). It compiles an approved Project Profile and
`skill-generation-plan.json` into a complete candidate accelerator in
task-scoped staging. Nothing is published into the target until evidence,
skill semantics, routing, structure, and the complete staged bundle pass.

## Generated File Naming Convention (MANDATORY)

This skill's own run notes live in
`tasks/TASK-{N}/infra-generate-report.md`. Candidate output is first written
under `tasks/TASK-{N}/infra-generate-staging/`, laid out exactly like the
target root. The staging tree is generator runtime output and is never a
target manifest member.

## Process

1. **Locate the profile.** Require the target project path (must match a profile from `infra-scan`); if more than one `TASK-{N}/` exists for that target, use the most recent unless the user specifies one.
2. **Validate evidence and the generation plan.** Require the matching
   `skill-generation-plan.json`. Re-check every target-relative evidence path,
   containment, fingerprint/range, authority, and supported claim against the
   current target. Require one complete contract per proposed skill and reject
   grouped substitutes, unjustified inventory entries, duplicate ownership,
   missing negative routing, and stale evidence. If anything drifted, stop for
   re-scan rather than accepting generic fallback content.
3. **Read the selected editions** from the profile's section 1 (AI Tool Selection). Only these editions will be produced.
4. **Collision guard and baseline.** If the target already has `AGENTS.md` or
   any selected edition folder, STOP and ask: overwrite, merge, or abort. Record
   the choice. Snapshot the existence and sha256 of every path the candidate
   may replace in `collision-baseline.json`; in merge mode also record the
   full pre-existing surface and exclude it from both writes and ownership.
5. **Create clean staging and explicit plans.** Refuse a non-empty staging root
   unless it belongs to this task and the user approved clearing it. Create
   `infra-generate-publication-plan.txt` for every staged file and
   `infra-generate-write-plan.txt` for the manifest-ownable subset. Every forge
   writes only beneath staging and appends each target-relative path exactly
   once to the publication plan; generator-owned non-runtime paths also enter
   the write plan. Never infer either set by walking the target.
6. **Forge evidence-independent surfaces.** Policy, hooks, and initial memory
   surfaces may run in parallel, but all writes are redirected to staging.
7. **Generate skills in evidence-scoped batches.** `skill-forge` authors one
   skill or a small sibling set from each validated contract. After every batch,
   run the semantic validator; after all batches, run inventory-wide evidence,
   ownership, repeated-block, and similarity checks. Stop immediately on any
   failure. No agent, command, flow, manifest, or target skill may exist yet.
8. **Wrap only validated skills.** Run `agent-forge`, then `command-forge`,
   using the validated contracts and routing fixtures. Run `skill-flow-composer`
   after wrappers exist. Validate adaptive specialist selection and write
   serialization.
9. **Stamp and stage the manifest.** Stamp only a staged `AGENTS.md` produced
   by this run. Build `.infra-manifest.json` against staging from the explicit
   write plan. Runtime state remains excluded from ownership.
10. **Verify the complete staged bundle.** Run `bootstrap-verifier` against
    staging while validating skill evidence against the real target root.
    Treat every semantic, routing, structural, hash, hook, runtime, or placeholder
    failure as generation failure.
11. **Recheck and publish transactionally.** Recompute the baseline immediately
    before publication. If any candidate destination changed, stop and rerun
    collision handling. Create a rollback journal, publish only explicit
    approved publication-plan paths, remove no team-owned path, and restore the
    previous state on any copy/rename failure. Copy the staged manifest last.
12. **Verify the published target.** Run the complete bootstrap gate again
    against the real target. A failure triggers rollback and cannot be reported
    as success.
13. **Report.** Record evidence/plan, per-category skill quality, routing,
    staged bundle, publication, rollback, and post-publication results.

## Version Stamp & Generation Manifest (MANDATORY)

The generator's version has a single source of truth: the `VERSION` file at this generator's root. Never hardcode a version and never recall one from the changelog.

Before authoring any skill, run the plan-only gate:

```bash
python3 .agents/skills/bootstrap-verifier/scripts/validate_skill_quality.py \
  --plan "tasks/TASK-003/skill-generation-plan.json" \
  --target "<path-to-real-target>" \
  --registry ".agents/skills/skill-forge/references/candidate-registry.json" \
  --plan-only
```

After each skill batch, run the same script with `--skills-dir` pointing to
each selected staged skills tree. A nonzero result blocks wrappers and
publication.

**Step A - stamp `AGENTS.md`.** Applies only to an `AGENTS.md` this run created: in merge mode a pre-existing `AGENTS.md` is never touched (it stays unstamped and untracked; `bootstrap-verifier` skips the stamp check for it). When this run did create it, make the FIRST line of the target's generated `AGENTS.md` this comment (real values, no placeholders), before hashing anything:

```markdown
<!-- Generated by Infrastructure-Creator v<version-from-VERSION> | <task-id> | <current-ISO-date> -->
```

- version = contents of this generator's `VERSION` file;
- task id = this run's `tasks/TASK-{N}/`;
- date = today, ISO `YYYY-MM-DD`.

**Step B - write `.infra-manifest.json`.** The manifest records the generator version, the profile this run consumed, the generation `mode` (`full` or `merge`), and the sha256 of exactly the paths in `infra-generate-write-plan.txt`. Manifest membership is the sole ownership authority in both modes. Runtime state is deliberately NOT tracked - `memory-bank/chunks/`, `memory-bank/INDEX.md`, `memory-bank/.memory-counter`, `memory-bank/local/`, `project-brain/indexes/`, and the `project-brain/` record/state directories (`archive/`, `control/`, `dynamic/`, `local/`) belong to the target team from the moment they are seeded, and `infra-update` never touches a file the manifest does not list.

Exact recipe - run from this generator's root after Step A, substituting the
**staging root**, task, selected editions, mode, and write-plan path. The
resulting manifest is copied to the target only after staged verification:

```bash
python3 .agents/skills/bootstrap-verifier/scripts/infra_ownership.py manifest \
  --target "tasks/TASK-003/infra-generate-staging" \
  --write-plan "tasks/TASK-003/infra-generate-write-plan.txt" \
  --version "$(cat VERSION)" \
  --task "TASK-003" \
  --profile "tasks/TASK-003/infra-scan-project-profile.md" \
  --editions "claude,cursor" \
  --mode "full"
```

The helper hashes only explicit write-plan members and refuses missing files, absolute/traversal paths, caches, the manifest itself, and runtime state. It never walks the target's managed roots. Consequently, team files absent from the plan remain untracked and invisible to `infra-update` in both full and merge modes. `validate_generated.py` validates every tracked file's existence/hash and scans every tracked text file for placeholders, while ignoring unmanifested team files.

## Transactional Publication Recipe

Use the bundled helper; do not implement ad-hoc recursive copies:

```bash
python3 .agents/skills/bootstrap-verifier/scripts/publish_staging.py snapshot \
  --target "<path-to-target>" \
  --publication-plan "tasks/TASK-003/infra-generate-publication-plan.txt" \
  --output "tasks/TASK-003/prepublication-baseline.json"

python3 .agents/skills/bootstrap-verifier/scripts/publish_staging.py publish \
  --target "<path-to-target>" \
  --staging "tasks/TASK-003/infra-generate-staging" \
  --publication-plan "tasks/TASK-003/infra-generate-publication-plan.txt" \
  --baseline "tasks/TASK-003/prepublication-baseline.json" \
  --journal "tasks/TASK-003/publication-rollback"
```

Keep the journal until post-publication `bootstrap-verifier` succeeds. On any
failure:

```bash
python3 .agents/skills/bootstrap-verifier/scripts/publish_staging.py rollback \
  --target "<path-to-target>" \
  --journal "tasks/TASK-003/publication-rollback"
```

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
- MUST NOT write any candidate artifact into the target before the complete
  staged bundle passes evidence, semantic, routing, and bootstrap validation.
- MUST generate ONLY the selected edition(s) - never an unselected edition, never skip a selected one.
- MUST NOT invent content beyond what the profile supports - a forge needing missing data is a signal to re-scan/re-interview, not to guess.
- MUST stamp `AGENTS.md` only when this run wrote it, and write `.infra-manifest.json` from `infra-generate-write-plan.txt` before running `bootstrap-verifier`, taking the version only from this generator's root `VERSION` file.
- MUST NOT list runtime state (memory chunks, indexes, counters, `local/` dirs, Project Brain records) in the manifest - `infra-update` treats everything the manifest lists as generator-owned.
- MUST maintain an explicit generated write plan in both modes. In merge mode, snapshot `preexisting-files.txt` before any forge writes, record `"mode": "merge"`, and never write or track a pre-existing team file. In full mode, track only files actually written; never infer ownership with a target-root walk.
- MUST maintain a separate explicit publication plan for initial team-owned
  runtime seeds that intentionally stay outside manifest ownership; publication
  still never walks staging or target roots.
- MUST NOT report success while `bootstrap-verifier` has unresolved failures.
- MUST roll back publication when any write or post-publication verification
  fails; a partially published accelerator is never an acceptable result.
- MUST NOT generate a skill absent from the validated plan or retain one whose
  evidence, operational ownership, procedure, or routing value is insufficient.

## Final Output

Return what was generated, where, for which edition(s), the verification results, remaining risks, and the concrete first command the target team should run inside their project.
