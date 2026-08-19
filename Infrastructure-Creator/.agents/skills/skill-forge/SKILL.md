---
name: skill-forge
description: Author the evidence-gated skill inventory one contract or small sibling group at a time in staging, using only the relevant target evidence slice and blocking generic, duplicated, or unsupported content.
phase: generation
flow-next: agent-forge
flow-alternatives: [policy-forge, hook-forge, memory-seed]
related: [infra-generate, policy-forge, agent-forge, command-forge, hook-forge, memory-seed, skill-flow-composer, bootstrap-verifier]
---

# Skill Forge

## Overview

`skill-forge` authors the evidence-gated inventory in `skill-generation-plan.json`. It works on one skill, or one small sibling group with an explicit shared boundary, at a time. Each pass receives only that contract and its referenced evidence slice, writes to task-scoped staging, and produces a substantive project-specific procedure rather than a renamed template. There are no fixed category counts and no minimum-line padding. Only the memory quartet is unconditional because `memory-seed` always installs its runtime; those four are `runtime-fixed` and are judged by runtime accuracy rather than project specificity (see `references/php-process-skills.md`).

Completeness means every justified contract is implemented, not that every catalog slot is filled.

Consult the bundled references for PHP-specific grounding:
- `references/php-frameworks.md` - detection signals and evidence-gated contracts for Design & Interaction, Universal PHP, and Frontend candidates.
- `references/php-architecture-patterns.md` - architecture detection and its generated-skill implications.
- `references/php-integration-catalog.md` - integration categories and what good coverage looks like per category.
- `references/php-process-skills.md` - evidence gates and distinct contracts for process/workflow candidates; only the memory quartet (`memory-bank`, `project-brain`, `checkpoint`, `memory`) is unconditional, and its "Status: runtime-fixed by decision" section defines the bar those four are held to instead.
- `references/php-specialty-skills.md` - the evidence-gated framework-specialty catalog, keyed to profile section 3.1's signals.
- `references/php-domain-behavior.md` - how section 8 enriches existing skills, when a domain skill is justified, and the always-generated operational `memory-bank` skill contract.

## Generated File Naming Convention (MANDATORY)

Write each authoring unit into the selected skill trees under
`tasks/TASK-{N}/infra-generate-staging/` (or
`infra-update-staging/`): `.claude/skills`, `.cursor/skills`, and/or
`.agents/skills`. Selected copies must be byte-identical. Staging is generator
runtime output and is never cited as evidence. Target publication is a later
orchestration step. The log maps every staged skill to exactly one plan entry.

## Process

1. **Validate inputs.** Run the plan-only gate. Require exact schema/catalog
   versions, Profile sibling identity, current fingerprints/ranges, supported
   claims, a satisfied evidence-backed `selection_gate` for every selected
   skill, rejected candidates that argue for themselves, structured ownership,
   reciprocal routing, resolved references, normalized write globs, canonical
   paths, and plan/profile inventory equality. Stop before authoring when any
   blocking contract-inventory diagnostic exists; surface nonblocking contract
   similarity/repeated-block warnings for review.
   A golden candidate is generated whether or not the target carries what its
   catalog gate asks for, and the honest form is narrow, not silent: the
   condition reports `status: "absent-golden"`, cites an absence entry whose
   search this gate resolves itself, and the skill carries `narrow_scope` - what
   it still does until the surface exists (`ABSENT_GOLDEN_NOT_PERMITTED`,
   `ABSENT_GOLDEN_UNPROVEN`, `NARROW_SCOPE_MISSING`).
   A rejection is graded like a selection, because dropping a candidate is a
   claim about the target: it names where the surface was looked for
   (`REJECTION_UNANCHORED` otherwise), it may not be one sentence stamped
   across categories (`REJECTION_TEMPLATED`), and it is tested against the real
   target through the registry's per-candidate signal
   (`REJECTION_CONTRADICTED`). A rejection may rest only on what the target is -
   never on what the current request happens to need, since the accelerator is
   generated once for all later work.
   Consolidation is not a grouping argument: the concern must be present in this
   target (`REJECTION_CONSOLIDATION_WITHOUT_SURFACE` - one that is not here is
   `absent`) and inside the absorbing skill's declared paths
   (`REJECTION_ABSORBER_OUT_OF_REACH` - widen that scope and say so).
2. **Choose one authoring unit.** Default to one skill. A group may contain only a small set of nearest siblings whose contracts explicitly define their ownership boundary (for example `debugging`/`systematic-debugger`). Never batch a whole category.
3. **Load the minimum slice.** Provide the authoring pass only the selected contract(s), referenced `evidence[]`, cited target files/excerpts, and applicable reference contract. Do not feed unrelated profile prose or prior generated skill bodies.
4. **Author into staging.** Implement every contract field as operational
   content: bounded purpose and positive/negative triggers; canonical inputs;
   owned/excluded scope; an evidence-specific procedure; safe verification;
   explicit outputs; and failure handling. Render each evidence item with its
   exact evidence ID, target-relative path, bounded line range (or an exact
   stable symbol/config-key anchor when a line range is unavailable), source
   type, confidence, and only the supported claim. A path without an anchor or
   a decorative evidence list is not sufficient.
   - Structure every procedure step as **Anchor**, **Inspect/Change**,
     **Decision**, and **Expected result**. Name the real target method,
     invariant, field, configuration key, fixture, suite, or package supported
     by that anchor. Include both the evidenced success branch and consequential
     failure branch; do not emit a generic open/trace/apply/exercise loop.
   - For a read-only contract, use only inspect/evaluate/compare/report wording,
     prescribe no edit or write, and state that implementation requires routing
     to a generated write-capable owner. For a write-capable contract, identify
     the exact owned write scope before any change instruction.
   - Structure each verification as **Check**, **Applies when**, **Command or
     manual assertion**, **Safety**, **Expected result**, **Pass**, **Fail**, and
     **Unavailable/skip report**. Commands must be copied from target evidence,
     resolved through target script aliases, and non-mutating. Manual assertions
     must identify exact observable behavior and evidence anchors.
   - If a dependency, fixture, binary, or local service is unavailable, do not
     silently pass or install/start it. Report `SKIPPED - <check>: <unavailable
     dependency>; impact: <unverified behavior>; evidence: <anchor>` and keep a
     required check blocking when its contract cannot otherwise be proved.
   Procedure text must materially reflect the evidence, not merely mention it
   in a generic preface. Render every planned `required_procedure_roles` entry
   as the step it is wired to, so each catalog obligation is visible as an
   operational instruction; do not collapse them into one inspection step.
   Where a check carries a `baseline`, carry its recorded observation into the
   skill so the reader knows what the command already reports on untouched
   code.
5. **Preserve boundaries.** If a nearest sibling is planned, state what routes to each and cross-reference it without copying its procedure. If it is not planned, do not create a dangling reference or silently absorb unsupported scope.
6. **Write valid frontmatter:** `name`, one-line trigger-aware `description`, `phase`, `flow-next`, `flow-alternatives`, and `related`. Every reference resolves within the plan.
7. **Review substance, not length.** Reject a skill that could serve an unrelated PHP repository after renaming nouns; reject generic five-step loops, unsupported commands, decorative evidence lists, grouped procedures, and line padding. There is no minimum line count.
   Shared safety/authority text is allowed only through a plan
   `fixed_blocks` entry with an explicit ID, version, and exact content; keep it
   confined to guardrails/failure handling.
8. **Analyze every proposed command without executing it.** Use
   `bootstrap-verifier/scripts/analyze_commands.py` against the real target.
   First inventory aliases without enforcement, then pass each command proposed
   for a generated verification separately with `--no-scripts --verification
   --command '<command>'`. The analyzer reads `composer.json` and `package.json`,
   resolves aliases transitively, and blocks shell composition, unknown aliases,
   cycles, shell/sudo/xargs/variable indirection, executables outside its
   curated read-only allow-list, workspace-writing format/fix modes,
   database/deploy/destructive operations, and external/provider/network
   actions. A mutating formatter may
   be documented only as an explicitly selected change step, never as
   verification. Replace unsafe verification with a real check/dry-run script
   evidenced in the target or an exact manual assertion; never weaken the
   classification.
9. **Verify the unit.** Run authored validation with
   `--allow-partial-skills` while batches remain. This still validates the
   complete plan inventory and every staged skill that exists; it suppresses
   only missing planned `SKILL.md` diagnostics. Check frontmatter,
   contract-field coverage, evidence/source resolution, sibling boundaries,
   write scope, commands against real config, and absence of
   secrets/placeholders/task paths. Record pass/fail before selecting the next
   unit.
10. **Log exactly one mapping per skill.** Record plan name/category, `kind`, evidence IDs, staged path, verification result, and publication eligibility. Compute category and total counts from successful log entries, and keep the two classes separate: evidence-derived project skills and `runtime-fixed` runtime guides are counted and reported apart, never merged into one "skills for your project" number.
11. **Run the complete final gate.** After all batches are authored, run normal
    full validation without `--allow-partial-skills`. Missing planned files and
    every other authored or contract diagnostic remain publication blockers.

## Output Template

```markdown
# Skill Forge Complete: [target_name]

**Editions:** [selected]
**Project skills staged:** [count of non-runtime-fixed entries] ([names])
**Runtime guides staged:** [count of runtime-fixed entries] ([names]) - guides to the memory runtime `memory-seed` installs, not derived from this target
**Dynamic breakdown:** [category=count, including zero-count categories]
- [category]: [names, or none]

## Log
tasks/TASK-{N}/skill-forge-log.md

## Next
agent-forge (wrap these skills), then command-forge; policy-forge/hook-forge/memory-seed if not already run.
```

## Guardrails

- MUST author only plan entries whose evidence gates passed; the memory quartet is the only unconditional set.
- MUST author every `runtime-fixed` skill exclusively from `memory-seed/assets/runtime-contract.json`: each `memory-bank/`/`project-brain/` path and each `python3 memory-bank/scripts/*.py` form it names must appear there, and it MUST NOT name a target path it declares no evidence for. Project specificity is not required of these four and MUST NOT be faked by borrowing an unrelated target file as evidence.
- MUST report project skills and runtime guides as two counts, never as one combined total.
- MUST process one skill or a small explicit sibling group at a time from a minimal contract/evidence slice.
- MUST stage before publication and MUST NOT cite generator task/staging paths in generated target skills.
- MUST produce substantive project-specific procedures; evidence-name substitution and minimum-line padding are invalid.
- MUST reflect the target's real framework/version and real tooling, not assumed defaults.
- MUST write only the selected edition(s).
- MUST ensure every cross-reference resolves to a skill generated in this run.
- MUST NOT use partial mode as the publication gate; the final complete inventory gate is mandatory.
- MUST NOT generate deep skills for non-PHP neighbors - those are integration contracts only.
- MUST NOT let a scope-split pair duplicate content: `debugging`/`systematic-debugger`, `database-designer`/`orm-patterns`, `performance`/`caching-strategy`, and `api-designer`/`api-platform-design` each have one explicit owner per concern and MUST cross-reference, not restate, their counterpart's half.
- MUST enrich existing skills from confirmed section 8 behavior before creating a domain skill; MUST NOT generate a domain skill without multiple coherent confirmed rules and a distinct operational purpose.
- MUST preserve source type, unknowns, and contradictions; MUST NOT turn statuses into transitions, observed enforcement into a complete permission matrix, or risk indicators into invented severity/approval.
- MUST default provider and integration procedures to static inspection,
  existing unit/contract tests, dependency-injected fakes, local fixtures, or
  local adapters. Network and credential-backed execution is denied by default,
  including provider sandboxes. A live action may be described only as a
  separately authorized operational branch that records the authorizer,
  classifies the non-production environment, bounds data and rollback, and
  sanitizes output; it MUST NOT be a generated verification check.
- MUST reject generated verification that writes the workspace, changes a
  database/deployment, invokes an external/provider/network boundary, contains
  shell composition, or relies on an unresolved/cyclic script alias.
- MUST include exact evidence anchors, explicit pass/fail outcomes, and
  unavailable-dependency/skip impact reporting in every generated skill.

## Final Output

Return the staged skill list by dynamic category, each skill's plan/evidence mapping and verification result, failures that block publication, the log path, and the next validation/publication step. Do not claim target editions were written while output remains in staging.
