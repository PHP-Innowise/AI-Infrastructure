---
name: infra-validate
description: Run the mandatory content review-and-repair phase over a complete staged bundle (or a published target via its manifest) - parallel read-only reviewers judge every generated file on uniqueness, completeness, accuracy, and coherence, blocking findings are repaired through the owning forges within bounded rounds, and a deterministic gate refuses publication until every file is dispositioned. Triggers on "infra-validate", "validate the generated content", "review and repair the staged accelerator", "make the generated skills ready".
phase: verification
flow-next: null
flow-alternatives: []
related: [content-reviewer, bootstrap-verifier, infra-generate, infra-update, skill-forge, agent-forge, command-forge, policy-forge, hook-forge, memory-seed, skill-flow-composer]
---

# Infra Validate

## Overview

`infra-validate` is the sixth sanctioned orchestrator (see `AGENTS.md`'s
"Orchestration Exception") and the content-level counterpart of the plan's
adversarial review. The mechanical gates prove form: evidence resolves,
skeletons differ, routing parses. This phase proves substance: a reader walks
the content of **every** generated file - skills, wrappers, policy, hooks,
memory seeds, and the run's own plans and reports - and what falls short is
repaired through the forge that owns it, or the run stops. `infra-generate`
and `infra-update` MUST run it against staging before manifest stamping and
publication; it also runs standalone against a previously published target.

## Generated File Naming Convention (MANDATORY)

Writes `tasks/TASK-{N}/infra-validate-report.md` (human summary) and
`tasks/TASK-{N}/infra-validate-review.json` (the machine-readable review
record the deterministic gate consumes). Reviewer fan-out writes
`tasks/TASK-{N}/content-reviewer-{lane}-findings.md` per lane. Repairs are
written only by the owning forges, only beneath staging (or repair staging in
standalone mode) - never by this skill directly.

## Review Record Contract (schema, MANDATORY)

`infra-validate-review.json` must define exactly `task`, `reviewer`
(`"independent"`), `generation_root`, `rounds` (0..2), `files`, `blockers`,
plus `post_repair_gates` when `rounds > 0`. Each `files[]` entry carries
`path`, `lane` (`skills`, `wrappers`, `policy`, `hooks`, `memory`, `reports`,
or `runtime-verbatim`), and - unless verbatim-exempt - one
`{verdict, note}` per dimension (`uniqueness`, `completeness`, `accuracy`,
`coherence`) and a `findings[]` array. Each finding carries `dimension`,
`severity` (`blocking`/`advisory`), `statement`, and a `resolution`
(`action`: `reforged` with `forge` + `round`, `escalated`, or `accepted` -
advisory only). The `runtime-verbatim` lane is valid only for the
stack-agnostic runtime surface (`memory-bank/scripts/`,
`memory-bank/templates/`, `project-brain/PROTOCOL.md`,
`project-brain/schemas/`, `project-brain/templates/`) and requires a `basis`.
`validate_content_review.py` enforces all of this; the schema above is
documentation, not a substitute for running the gate.

## Process

1. **Locate the run.** Inside `infra-generate`/`infra-update`: the current
   task's staging root and its `infra-generate-publication-plan.txt` (or the
   update staging equivalent). Standalone: require the target path and its
   `.infra-manifest.json`; the manifest's `files` map is the review surface,
   and a missing manifest aborts - a legacy target is `infra-update`'s
   problem, not a reason to walk the tree. One sanctioned addition to that
   surface: the seeded `memory-bank/chunks/MEM-*.md` and `INDEX.md` are
   deliberately unmanifested runtime state, yet they are generator-seeded
   content - standalone mode reviews them in the `memory` lane (glob those
   two shapes only, never a broader walk), because a templated bank is
   exactly the defect a re-run of this one command must catch and fix.
2. **Partition the surface into lanes** from the explicit plan - never a tree
   walk: `skills` (generated `SKILL.md` files), `wrappers` (agents, commands,
   flows, `SKILL FLOW.md`), `policy` (`AGENTS.md`, `DOD.md`,
   `GOLDEN-PRINCIPLES.md`, `STABILIZATION.md`), `hooks` (scripts + wiring
   files), `memory` (seeded chunks, `INDEX.md`, `config/runtime.json`),
   `runtime-verbatim` (the sanctioned verbatim runtime, exempt with a basis),
   and `reports` (this run's `infra-scan-project-profile.md`,
   `skill-generation-plan.json`, and any findings documents the profile
   consumed).
3. **Fan out one `content-reviewer` per lane, in parallel.** Reviewers are
   read-only, so parallel instances are safe under the subagent gate's write
   serialization. Each gets its lane, explicit file list, the real target
   path, and the plan. Large skill inventories may split the `skills` lane
   into batches; every batch is still an explicit list.
   The `memory` lane runs its deterministic gate first and folds the result
   into the lane's findings:

   ```bash
   python3 .agents/skills/bootstrap-verifier/scripts/validate_memory_content.py \
     --bank "tasks/TASK-003/infra-generate-staging/memory-bank" \
     --target "<path-to-real-target>"
   ```

   It refuses a chunk body that states no rule, bodies collapsing to one
   shared skeleton, known boilerplate, a cited source Verification never
   explains, a range the file cannot contain, and a contradiction chunk that
   records no competing claims or presents itself as settled. Each error is a
   blocking `memory`-lane finding repaired through `memory-seed`, which
   rewrites the chunk from the run's own evidence (domain findings,
   `project-claims.json`, the plan contracts that cite the same sources).
4. **Merge lane findings into the review record.** Every advisory finding is
   recorded with `action: accepted` and a reason, or repaired alongside the
   blockers. Every blocking finding must end `reforged` or `escalated`.
5. **Repair through the owning forge, bounded.** For each blocking finding,
   re-invoke the lane's forge (`skill-forge`, `agent-forge`, `command-forge`,
   `skill-flow-composer`, `policy-forge`, `hook-forge`, `memory-seed`;
   `reports` findings go back to `profile-synthesizer` or stop the run) with
   the finding attached as a contract amendment. A forge that would need
   evidence the target does not contain MUST escalate (`action: escalated`)
   for re-scan/re-interview - a repair never invents content. At most **two**
   repair rounds; a finding still blocking after round two is an escalation.
6. **Re-run the mechanical gates after any repair round** - a repair can
   regress what the validators already passed:

   ```bash
   python3 .agents/skills/bootstrap-verifier/scripts/validate_skill_quality.py \
     --plan "tasks/TASK-003/skill-generation-plan.json" \
     --target "<path-to-real-target>" \
     --skills-dir "tasks/TASK-003/infra-generate-staging/.claude/skills" \
     --registry ".agents/skills/skill-forge/references/candidate-registry.json"
   ```

   plus `validate_generated.py` with the run's usual arguments, and
   `validate_memory_content.py` whenever a repair touched the `memory` lane.
   Re-review each repaired file (its reviewer entry reflects the repaired
   content, with the finding's `resolution.round` recording when it was
   fixed). Record the re-run gates in `post_repair_gates`.
7. **Run the deterministic gate** and require exit 0:

   ```bash
   python3 .agents/skills/bootstrap-verifier/scripts/validate_content_review.py \
     --publication-plan "tasks/TASK-003/infra-generate-publication-plan.txt" \
     --review "tasks/TASK-003/infra-validate-review.json"
   ```

   Standalone mode substitutes a plan file written from the manifest's
   `files` map. A nonzero exit is this phase failing, not a formality.
8. **Standalone repairs publish transactionally.** Against a published
   target, forges write into `tasks/TASK-{N}/infra-validate-staging/`, the
   repaired subset publishes via `publish_staging.py` snapshot/publish with a
   rollback journal, the manifest hashes are refreshed with
   `bootstrap-verifier`'s recipe, and `bootstrap-verifier` runs afterwards.
   Never edit a manifest-owned file in place.
   **Memory chunks are the exception that needs consent, not a walk-in.**
   They are team-owned live data from the moment they are seeded: the team
   may have edited or added chunks since generation. Standalone repair of a
   chunk therefore requires an explicit per-chunk decision - show the current
   body next to the `memory-seed` rewrite (built from the task's evidence:
   the profile, `project-claims.json`, and the plan contracts citing the
   same sources) and apply only approved rewrites, appending new knowledge
   as a superseding chunk rather than overwriting a team edit. A chunk the
   evidence can no longer support is flagged `needs-review`, never silently
   deleted. Re-run `validate_memory_content.py` and the bank's own
   `validate.py` against the published bank afterwards.
9. **Report** per-lane counts, findings, repairs, escalations, and the gate
   result in `infra-validate-report.md`.

## Output Template

```markdown
# Infra Validate Complete: [target_name]

**Mode:** [staged (TASK-{N}) / standalone against published target]
**Files reviewed:** [count] across [lane list]
**Blocking findings:** [count] ([reforged] repaired, [escalated] escalated)
**Advisory findings:** [count]
**Repair rounds:** [0-2]
**Deterministic gate:** [pass/fail]

## Repairs
- [path] - [dimension]: [statement] -> reforged by [forge] (round [n])

## Escalations (need a human)
- [path] - [statement] and why evidence cannot settle it

## Lane Reports
- [lane]: tasks/TASK-{N}/content-reviewer-[lane]-findings.md
```

## Guardrails

- MUST review every file the publication plan (or manifest) lists - a file
  too boring to review is still dispositioned, and the verbatim exemption
  covers only the sanctioned runtime surface.
- MUST run `validate_memory_content.py` for the `memory` lane in every mode -
  a bank whose chunks share one body template has preserved nothing, however
  clean its structure - and in standalone mode MUST cover the unmanifested
  seeded chunks (`memory-bank/chunks/MEM-*.md` + `INDEX.md` only) while
  treating them as team-owned: per-chunk approval before any rewrite, team
  edits superseded rather than overwritten, unsupported chunks flagged
  `needs-review` rather than deleted.
- MUST keep reviewers read-only and repairs inside the owning forges; this
  skill never edits generated content itself.
- MUST bound repairs at two rounds and escalate what survives them; MUST NOT
  loop a forge against a reviewer indefinitely.
- MUST NOT let a forge invent content to satisfy a finding - missing evidence
  escalates to re-scan/re-interview.
- MUST re-run `validate_skill_quality.py` and `validate_generated.py` after
  any repair round, and MUST NOT report success while
  `validate_content_review.py` exits nonzero.
- MUST NOT walk staging or the target to enumerate the surface - the explicit
  plan or manifest is the only authority.
- In standalone mode, MUST abort without `.infra-manifest.json` and MUST
  publish repairs transactionally with a manifest hash refresh, never in
  place.

## Final Output

Return the mode, files reviewed per lane, blocking/advisory counts, repairs
with their forges and rounds, open escalations, and the deterministic gate
result. On pass, `infra-generate`/`infra-update` may proceed to manifest
stamping and publication.
