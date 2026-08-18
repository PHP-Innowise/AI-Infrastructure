---
name: profile-synthesizer
description: Compile all discovery findings into a human Project Profile plus a machine-readable evidence ledger and one complete generation contract per justified skill. Use as the final read-only infra-scan step.
phase: synthesis
flow-next: infra-generate
flow-alternatives: []
related: [infra-scan, stack-scanner, architecture-scanner, integration-scanner, infra-ops-scanner, security-compliance-scanner, conventions-scanner, domain-behavior-scanner, stack-researcher, clarifying-interview]
---

# Profile Synthesizer

## Overview

`profile-synthesizer` produces the two artifacts Phase 2 consumes: the human-reviewable `infra-scan-project-profile.md` and the machine-readable `skill-generation-plan.json`. It merges the seven scanners' findings, `stack-researcher`'s sourced notes, and `clarifying-interview`'s answers into a schema-conformant profile and one complete generation contract per proposed skill. Technical conflicts prefer higher-confidence direct evidence; behavioral conflicts preserve both source type and confidence and are surfaced rather than silently collapsed, because a test, ADR, database constraint, code path, and interview answer are not equal authorities.

Crucially, the profile is not just a dry evidence dump - it is the user's one chance to review *what will actually be generated* before committing to `infra-generate`. Section 8 shows the discovered behavioral contract; section 11 summarizes the evidence-gated inventory and points to the complete JSON contracts; section 12 previews the exact memory-bank concepts that will be seeded.

The target project path is a **required** argument. This skill reads only the current run's `tasks/TASK-{N}/` findings files (plus, if needed, the target's files to break a tie); it never writes into the target.

## Generated File Naming Convention (MANDATORY)

Write exactly two sibling files:

- `tasks/TASK-{N}/infra-scan-project-profile.md`
- `tasks/TASK-{N}/skill-generation-plan.json`

Follow `references/project-profile-schema.md` exactly. The JSON is generator runtime input, not a target artifact, and MUST NOT be copied into a generated skill tree or cited as a generated target skill's source.

## Process

1. **Load all inputs** from `tasks/TASK-{N}/`: the seven `*-findings.md` (including `domain-behavior-scanner-findings.md`), `stack-researcher-findings.md`, and `clarifying-interview-answers.md`. Fill section 0's "Generator version" from the `VERSION` file at this generator's root - it is the single source of the version (the same value `infra-generate` later stamps into the target's `AGENTS.md` and `.infra-manifest.json`); never hardcode it or recall it from the changelog.
2. **Populate section 1 (AI Tool Selection)** strictly from the interview answer. If it is missing, STOP and re-run `clarifying-interview` - never assume an edition.
3. **Merge sections 2-7** from the six technical scanners, preserving each fact's confidence tag and source path. Compile the scanners' command definitions (including resolved Composer/npm aliases and mutation/network classification), test topology (suite/config/root, central-vs-domain placement, fixtures/fakes, and focused invocation), path-authority declarations, and material adjacency candidates without filling gaps from framework convention. When two technical scanners disagree, prefer the higher-confidence, more direct evidence and note the resolution. Sections 3.1 (Framework-Specialty Signals) and 3.2 (Frontend Presence) come from `architecture-scanner-findings.md`'s dedicated sections - carry every signal through even when its value is "none".
4. **Build section 8 ("Domain & Behavioral Contract")** from `domain-behavior-scanner-findings.md`. Preserve both confidence and source type. Assign stable invariant IDs to every confirmed invariant, mark high-priority failure invariants explicitly, and retain their bounded evidence anchors and named regression scenarios. Keep status discovery separate from confirmed transitions, authentication technology separate from product permissions, risk indicators separate from documented severity/approval, and contradictory sources visible. Carry only bounded central entities and representative critical scenarios.
5. **Fold in research notes (section 9)** from `stack-researcher`, keeping source URLs.
6. **Resolve open items (section 10)** using interview answers; anything still unresolved stays `unknown`, explicitly listed. Preserve `interview answer` as its source type rather than making it indistinguishable from repository evidence.
7. **Derive the evidence-gated skill inventory.** Load
   `skill-forge/references/candidate-registry.json` and evaluate every candidate
   against its real catalog anchor. Every registry ID must receive exactly one
   disposition: selected (families may produce multiple concrete skills) or
   rejected with reason/missing evidence. There are no category quotas. Only
   the memory quartet is runtime-fixed.
8. **Build `skill-generation-plan.json`.** Stamp schema `1.3` and the current
   reference-corpus `catalog_version`. Normalize target evidence into
   `evidence[]` with supported claims and sha256 fingerprints for repository
   files. Local evidence needs a bounded anchor (`line_range`, `symbol`, or
   `json_pointer`); a bare path is insufficient. Create exactly one complete
   contract per selected skill, including a
   `selection_gate` whose satisfied conditions cite the skill's own evidence
   and distinguish adjacent candidates, structured `ownership[]`, normalized
   path contracts (`required-existing`, `generated-runtime`, or `creatable`),
   command/test-topology references, and reciprocal routing entries for every
   material adjacent owner. Add positive, negative, ambiguous, and cross-domain
   routing cases with one primary owner or an explicitly approved ambiguity.
   Take `required_procedure_roles` from the candidate's own `roles` in
   `candidate-registry.json` - never the universal
   `load-evidence`/`execute`/`verify` trio - and wire each role to the evidence
   that grounds it and the procedure step that discharges it. Three or more
   obligations pointing at one step is a template and is rejected.
   Run every executable verification command against the unmodified target once
   and record what it did in that check's `baseline`; then state
   `expected_result` against that baseline rather than promising the command
   succeeds outright. Record what the target does **not** do as `absence`
   evidence when it matters - a configured analyser nobody invokes is a fact a
   skill needs - with a literal search and the matches it accounts for.
   Never use a
   grouped summary. Every source path is canonical and target-relative; URLs
   remain URLs. Ownership IDs use stable lowercase kebab/dotted syntax and modes
   `exclusive`, `shared`, or `composed`; enforce the single-owner,
   non-overlapping-shared-writer, and exactly-one-composer invariants before the
   checkpoint.
   Every high-priority confirmed invariant ID must map to at least one selected
   skill procedure step and one concrete verification assertion. If no
   selected skill can own both, stop and correct the inventory. A catalog
   capability (for example deduplication, retention, timeout handling, or WCAG
   conformance) is not confirmed target behavior unless the cited target
   evidence anchor proves it; otherwise encode it as a review question,
   external-standard requirement, or excluded unsupported claim.
   Load `memory-seed/assets/runtime-contract.json` for the runtime-fixed
   quartet. Compile those four contracts from its exact paths, SQLite tables,
   commands, ownership, required skeleton, and creatable artifacts; never
   synthesize runtime paths from catalog prose.
9. **Derive section 11.2 ("Agents & Commands Preview")** from `skills.length`, with a dynamic category breakdown. Multiply by selected editions carrying agent/command layers; Codex has neither. Do not embed baseline numbers in the arithmetic.
10. **Derive section 12 ("Memory Bank Preview")** using the exact same selection rule `memory-seed` applies: one planned chunk per cohesive durable concept composed only from confirmed facts across sections 2-8. Group tightly related facts rather than producing tiny per-line chunks. Link canonical sources; do not copy full specs, schemas, permission matrices, test inventories, incident narratives, or sensitive data.
11. **Self-validate both artifacts** against
    `references/project-profile-schema.md` and the plan-only semantic validator.
    Require exact top-level/schema membership, the sibling Profile path, catalog
    version, fingerprints and bounded ranges, supported claims, complete
    selection gates, auditable rejected candidates, structured ownership,
    reciprocal routing roles, normalized non-overlapping globs, unique/resolved
    references, complete skill contracts, command definitions, test topology,
    invariant procedure/assertion coverage, path authority/creatability,
    material adjacency and routing cases, dynamic count equality, canonical
    paths, no sensitive data, and runtime wiring for confirmed integrations.
    Confirm the runtime-fixed contracts match
    `memory-seed/assets/runtime-contract.json` exactly.
    The human checkpoint requires zero blocking contract-inventory diagnostics;
    report nonblocking contract-similarity warnings for review rather than
    silently discarding them.
12. **Write both files atomically for the run** and report both paths.

## Output Template

```markdown
# Profile Synthesized: [target_name]

**File:** tasks/TASK-{N}/infra-scan-project-profile.md
**Generation plan:** tasks/TASK-{N}/skill-generation-plan.json
**Editions:** [selected]
**Skills to generate:** [count] ([list]) - see section 11.1 for what each one will actually do
**Agents/commands preview:** [counts from section 11.2]
**Memory bank preview:** [count] cohesive concepts planned - see section 12
**Confidence summary:** [X confirmed, Y inferred, Z unknown]

## Validation
[schema self-check result: pass / issues fixed]

## Review Before Generating
Read the profile and correct anything wrong, then run `infra-generate`.
```

## Guardrails

- MUST conform to `references/project-profile-schema.md` exactly.
- MUST NOT assume the AI-tool selection - it comes only from the interview.
- MUST NOT propose any skill whose reference trigger and required evidence are unsatisfied. Familiarity, category symmetry, and a preferred baseline are not evidence.
- MUST generate only the memory quartet unconditionally, because its runtime is always installed.
- MUST provide a complete JSON contract for every selected skill; grouped or one-line descriptions are summaries only.
- MUST cover every `roles` entry the selected candidate declares in the registry, each wired to cited evidence and to a procedure step that discharges it.
- MUST record an observed `baseline` for every executable verification the gate cannot resolve, and phrase the expectation against it.
- MUST emit schema 1.3 operational, ownership, invariant, path, and reciprocal routing contracts; schemas 1.0/1.1/1.2 are audit/migration input only and are not publishable.
- MUST use target-relative canonical source paths in contracts and generated target skills; generator task paths are never target evidence.
- MUST NOT include any secret or credential value.
- MUST keep every fact's confidence tag and source; never launder an `inferred` fact into a `confirmed` one.
- MUST NOT let section 12 include an `inferred`/`unknown` fact, raw incident detail, customer data, or copied canonical source content.
- MUST preserve source type and contradictions for behavioral findings; confidence alone is not enough.
- MUST NOT elevate a catalog concern, inferred convention, package feature, or external standard into confirmed target behavior without a bounded target evidence anchor.
- MUST map every high-priority confirmed invariant to a selected skill procedure and concrete verification assertion.
- MUST compile the memory quartet from `memory-seed/assets/runtime-contract.json`, including `memory-bank/local/context.db`, SQLite `working_tasks`/`turn_deltas`, `project-brain/dynamic/**`, `project-brain/control/**`, required skeleton paths, creatable paths, and exact CLI commands.

## Final Output

Return both artifact paths, the selected editions, the behavioral-contract summary (including contradictions), the selected and rejected skill inventory with reasons, the dynamic agents/commands preview counts, the memory-bank concept preview count, the confidence summary, and the next step (user reviews both artifacts, then runs `infra-generate`).
