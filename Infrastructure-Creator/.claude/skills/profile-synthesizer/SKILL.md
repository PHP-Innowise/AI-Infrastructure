---
name: profile-synthesizer
description: Merge all seven scanner findings (including domain behavior), stack-researcher's results, and clarifying-interview's answers into the single canonical, schema-conformant Project Profile that is the sole contract between Phase 1 (scanning) and Phase 2 (generation). Takes a required target-project-path argument. Use as the last step of infra-scan, after clarifying-interview. Triggers on "synthesize the profile", "build the project profile", "profile-synthesizer", "merge the findings".
phase: synthesis
flow-next: infra-generate
flow-alternatives: []
related: [infra-scan, stack-scanner, architecture-scanner, integration-scanner, infra-ops-scanner, security-compliance-scanner, conventions-scanner, domain-behavior-scanner, stack-researcher, clarifying-interview]
---

# Profile Synthesizer

## Overview

`profile-synthesizer` produces the one artifact Phase 2 consumes: `infra-scan-project-profile.md`. It merges the seven scanners' findings, `stack-researcher`'s sourced notes, and `clarifying-interview`'s answers into a single, schema-conformant document. Technical conflicts prefer higher-confidence direct evidence; behavioral conflicts preserve both source type and confidence and are surfaced rather than silently collapsed, because a test, ADR, database constraint, code path, and interview answer are not equal authorities.

Crucially, the profile is not just a dry evidence dump - it is the user's one chance to review *what will actually be generated* before committing to `infra-generate`. Section 8 shows the discovered behavioral contract; section 11 gives a target-specific, one-line description of every skill about to be written (including any evidence-gated domain skills), an explicit agent/command count for the selected edition(s); and section 12 previews the exact memory-bank concepts that will be seeded.

The target project path is a **required** argument. This skill reads only the current run's `tasks/TASK-{N}/` findings files (plus, if needed, the target's files to break a tie); it never writes into the target.

## Generated File Naming Convention (MANDATORY)

Write exactly one file: `tasks/TASK-{N}/infra-scan-project-profile.md`, following `references/project-profile-schema.md` exactly.

## Process

1. **Load all inputs** from `tasks/TASK-{N}/`: the seven `*-findings.md` (including `domain-behavior-scanner-findings.md`), `stack-researcher-findings.md`, and `clarifying-interview-answers.md`. Fill section 0's "Generator version" from the `VERSION` file at this generator's root - it is the single source of the version (the same value `infra-generate` later stamps into the target's `AGENTS.md` and `.infra-manifest.json`); never hardcode it or recall it from the changelog.
2. **Populate section 1 (AI Tool Selection)** strictly from the interview answer. If it is missing, STOP and re-run `clarifying-interview` - never assume an edition.
3. **Merge sections 2-7** from the six technical scanners, preserving each fact's confidence tag and source path. When two technical scanners disagree, prefer the higher-confidence, more direct evidence and note the resolution. Sections 3.1 (Framework-Specialty Signals) and 3.2 (Frontend Presence) come from `architecture-scanner-findings.md`'s dedicated sections - carry every signal through even when its value is "none".
4. **Build section 8 ("Domain & Behavioral Contract")** from `domain-behavior-scanner-findings.md`. Preserve both confidence and source type. Keep status discovery separate from confirmed transitions, authentication technology separate from product permissions, risk indicators separate from documented severity/approval, and contradictory sources visible. Carry only bounded central entities and representative critical scenarios.
5. **Fold in research notes (section 9)** from `stack-researcher`, keeping source URLs.
6. **Resolve open items (section 10)** using interview answers; anything still unresolved stays `unknown`, explicitly listed. Preserve `interview answer` as its source type rather than making it indistinguishable from repository evidence.
7. **Derive section 11.1 ("Skills To Generate")** in eight groups, cross-referencing `skill-forge/references/` for what each group actually contains:
   - **Architecture** (1): from section 3, as before.
   - **Design & Interaction** (3, always): `architecture-implementer`, `api-designer`, `database-designer` - each grounded in sections 2-3's real framework/persistence evidence (see `skill-forge/references/php-frameworks.md`'s "Design & Interaction Skills").
   - **Frontend** (0 or 5): only if section 3.2's verdict is "applies" - `frontend-design`, `coder-frontend`, `wcag-accessibility`, `web-design-guidelines`, `browser-verify`; otherwise write "No UI surface detected - frontend skill group skipped" and generate none.
   - **Process & Workflow** (18, always, fixed list): the skills named in `skill-forge/references/php-process-skills.md`, including the memory quartet (`memory-bank`, `project-brain`, `checkpoint`, `memory`) that operates the shared memory layer - list all 18 by name; one shared sentence suffices since this group's mechanic never varies by target.
   - **Universal PHP** (7): `coding`, `testing`, `code-review`, `security-review`, `performance`, `release`, `debugging`, as before.
   - **Framework-Specialty** (one per `confirmed`/`inferred` line in section 3.1, per `skill-forge/references/php-specialty-skills.md`'s mapping table): skip every `none`/`unknown` signal - never generate one speculatively.
   - **Integrations** (one per `confirmed` integration in section 4), as before.
   - **Domain** (0 or more): one skill per cohesive candidate in section 8.11 only when multiple confirmed rules form a bounded context and the skill has a distinct review purpose. Never generate one skill per rule, status, entity, role, risk, or test.
   For EVERY non-fixed entry, write a one-line description specific to this target - name the real detected pattern/tool/package/domain rules, never generic boilerplate. Record non-PHP neighbors in 11.3 as integration contracts only.
8. **Derive section 11.2 ("Agents & Commands Preview")** by first stating the group-by-group skill count breakdown (architecture + design + frontend + 18 process + 7 universal + specialty + integrations + domain = total), then the skill total from 11.1, times the number of selected editions (section 1) that carry an agent layer (Claude, Cursor), for the agent total; the same count again for commands (Claude and Cursor carry command layers; Codex has no command layer). State Codex's direct-skill invocation model explicitly if it was selected.
9. **Derive section 12 ("Memory Bank Preview")** using the exact same selection rule `memory-seed` applies: one planned chunk per cohesive durable concept composed only from confirmed facts across sections 2-8. Group tightly related facts (such as one lifecycle's statuses, transitions, guards, permissions, and audit consequence) rather than producing tiny per-line chunks. Link canonical sources; do not copy full specs, schemas, permission matrices, test inventories, incident narratives, or sensitive data. This becomes the authoritative seed plan.
10. **Self-validate** against `references/project-profile-schema.md`: every line in sections 2-8 has confidence; every section 8 finding also has source type; statuses are not presented as transitions without evidence; permission completeness is stated; risk indicators do not invent severity/approval; section 1 lists >=1 edition sourced from the interview; no skill is proposed for absent evidence; every non-fixed 11.1 entry has a target-specific description; 11.2 counts are arithmetically consistent; section 12 contains only confirmed, source-linked cohesive concepts; no secrets/customer data anywhere; a `confirmed` integration cites runtime wiring.
11. **Write the profile** and report.

## Output Template

```markdown
# Profile Synthesized: [target_name]

**File:** tasks/TASK-{N}/infra-scan-project-profile.md
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
- MUST NOT propose a skill for an integration with no evidence, a framework-specialty skill for a `none`/`unknown` section 3.1 signal, the frontend group when section 3.2's verdict says it doesn't apply, or a domain skill without a cohesive section 8.11 candidate.
- MUST NOT write a generic, boilerplate description for any 11.1 skill entry - each one must name what was actually found in this target.
- MUST NOT include any secret or credential value.
- MUST keep every fact's confidence tag and source; never launder an `inferred` fact into a `confirmed` one.
- MUST NOT let section 12 include an `inferred`/`unknown` fact, raw incident detail, customer data, or copied canonical source content.
- MUST preserve source type and contradictions for behavioral findings; confidence alone is not enough.

## Final Output

Return the profile path, the selected editions, the behavioral-contract summary (including contradictions), the derived skill list with a one-line summary of each (from 11.1), the agents/commands preview counts (from 11.2), the memory-bank concept preview count (from 12), the confidence summary, and the next step (user reviews, then runs `infra-generate`).
