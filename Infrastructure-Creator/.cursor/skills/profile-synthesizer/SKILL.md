---
name: profile-synthesizer
description: Merge all six scanner findings, stack-researcher's results, and clarifying-interview's answers into the single canonical, schema-conformant Project Profile that is the sole contract between Phase 1 (scanning) and Phase 2 (generation). Takes a required target-project-path argument. Use as the last step of infra-scan, after clarifying-interview. Triggers on "synthesize the profile", "build the project profile", "profile-synthesizer", "merge the findings".
phase: synthesis
flow-next: infra-generate
flow-alternatives: []
related: [infra-scan, stack-scanner, architecture-scanner, integration-scanner, infra-ops-scanner, security-compliance-scanner, conventions-scanner, stack-researcher, clarifying-interview]
---

# Profile Synthesizer

## Overview

`profile-synthesizer` produces the one artifact Phase 2 consumes: `infra-scan-project-profile.md`. It merges the six scanners' findings, `stack-researcher`'s sourced notes, and `clarifying-interview`'s answers into a single, schema-conformant document, resolving conflicts in favor of higher-confidence evidence and carrying every fact's confidence tag through.

Crucially, the profile is not just a dry evidence dump - it is the user's one chance to review *what will actually be generated* before committing to `infra-generate`. That means section 10 gives a target-specific, one-line description of every skill about to be written (not a bare name), an explicit agent/command count for the selected edition(s), and section 11 previews the exact memory-bank chunks that will be seeded - so nothing about the eventual output is a surprise at generation time.

The target project path is a **required** argument. This skill reads only the current run's `tasks/TASK-{N}/` findings files (plus, if needed, the target's files to break a tie); it never writes into the target.

## Generated File Naming Convention (MANDATORY)

Write exactly one file: `tasks/TASK-{N}/infra-scan-project-profile.md`, following `references/project-profile-schema.md` exactly.

## Process

1. **Load all inputs** from `tasks/TASK-{N}/`: the six `*-findings.md`, `stack-researcher-findings.md`, and `clarifying-interview-answers.md`.
2. **Populate section 1 (AI Tool Selection)** strictly from the interview answer. If it is missing, STOP and re-run `clarifying-interview` - never assume an edition.
3. **Merge sections 2-7** from the scanners, preserving each fact's confidence tag and source path. When two scanners disagree, prefer the higher-confidence, more direct evidence and note the resolution. Sections 3.1 (Framework-Specialty Signals) and 3.2 (Frontend Presence) come from `architecture-scanner-findings.md`'s dedicated sections - carry every signal through even when its value is "none", so 10.1's specialty list is auditable against a complete signal set, not a partial one.
4. **Fold in research notes (section 8)** from `stack-researcher`, keeping source URLs.
5. **Resolve open items (section 9)** using interview answers; anything still unresolved stays `unknown`, explicitly listed.
6. **Derive section 10.1 ("Skills To Generate")** in six groups, cross-referencing `skill-forge/references/` for what each group actually contains:
   - **Architecture** (1): from section 3, as before.
   - **Design & Interaction** (3, always): `architecture-implementer`, `api-designer`, `database-designer` - each grounded in sections 2-3's real framework/persistence evidence (see `skill-forge/references/php-frameworks.md`'s "Design & Interaction Skills").
   - **Frontend** (0 or 5): only if section 3.2's verdict is "applies" - `frontend-design`, `coder-frontend`, `wcag-accessibility`, `web-design-guidelines`, `browser-verify`; otherwise write "No UI surface detected - frontend skill group skipped" and generate none.
   - **Process & Workflow** (14, always, fixed list): the skills named in `skill-forge/references/php-process-skills.md` - list all 14 by name; one shared sentence suffices since this group's mechanic never varies by target.
   - **Universal PHP** (7): `coding`, `testing`, `code-review`, `security-review`, `performance`, `release`, `debugging`, as before.
   - **Framework-Specialty** (one per `confirmed`/`inferred` line in section 3.1, per `skill-forge/references/php-specialty-skills.md`'s mapping table): skip every `none`/`unknown` signal - never generate one speculatively.
   - **Integrations** (one per `confirmed` integration in section 4), as before.
   For EVERY non-fixed entry, write a one-line description specific to this target - name the real detected pattern/tool/package, never generic boilerplate (e.g. not "handles payment integration" but "Stripe integration wired via the StripeClient binding in config/services.php"). Record non-PHP neighbors in 10.3 as integration contracts only.
7. **Derive section 10.2 ("Agents & Commands Preview")** by first stating the group-by-group skill count breakdown (architecture + design + frontend + 14 process + 7 universal + specialty + integrations = total), then the skill total from 10.1, times the number of selected editions (section 1) that carry an agent layer (Claude, Cursor), for the agent total; the same count again for commands (Claude, Cursor only - Codex has no command layer). State Codex's "skills only" status explicitly if it was selected.
8. **Derive section 11 ("Memory Bank Preview")** using the exact same selection rule `memory-seed` applies: one planned chunk per durable `confirmed` fact worth remembering across sections 2-7 (never `inferred`/`unknown`), each with a placeholder ID starting at `MEM-0001`, a short title, its `type` (from the same enum `memory-seed` uses), and its source. This becomes the authoritative seed plan - `memory-seed` must match it exactly at generation time.
9. **Self-validate** against `references/project-profile-schema.md`: every line in sections 2-7 (including 3.1 and 3.2) has a confidence tag; section 1 lists >=1 edition sourced from the interview; no skill is proposed for an absent integration or a `none`/`unknown` specialty signal; no frontend group unless 3.2's verdict applies; every non-fixed 10.1 entry has a target-specific description; 10.2's counts (including the group breakdown) are arithmetically consistent; section 11 has no `inferred`/`unknown` entries; no secrets anywhere; a `confirmed` integration cites runtime wiring.
10. **Write the profile** and report.

## Output Template

```markdown
# Profile Synthesized: [target_name]

**File:** tasks/TASK-{N}/infra-scan-project-profile.md
**Editions:** [selected]
**Skills to generate:** [count] ([list]) - see section 10.1 for what each one will actually do
**Agents/commands preview:** [counts from section 10.2]
**Memory bank preview:** [count] chunks planned - see section 11
**Confidence summary:** [X confirmed, Y inferred, Z unknown]

## Validation
[schema self-check result: pass / issues fixed]

## Review Before Generating
Read the profile and correct anything wrong, then run `infra-generate`.
```

## Guardrails

- MUST conform to `references/project-profile-schema.md` exactly.
- MUST NOT assume the AI-tool selection - it comes only from the interview.
- MUST NOT propose a skill for an integration with no evidence, or a framework-specialty skill for a `none`/`unknown` section 3.1 signal, or the frontend group when section 3.2's verdict says it doesn't apply.
- MUST NOT write a generic, boilerplate description for any 10.1 skill entry - each one must name what was actually found in this target.
- MUST NOT include any secret or credential value.
- MUST keep every fact's confidence tag and source; never launder an `inferred` fact into a `confirmed` one.
- MUST NOT let section 11 include an `inferred`/`unknown` fact as a planned memory chunk.

## Final Output

Return the profile path, the selected editions, the derived skill list with a one-line summary of each (from 10.1), the agents/commands preview counts (from 10.2), the memory-bank chunk preview count (from 11), the confidence summary, and the next step (user reviews, then runs `infra-generate`).
