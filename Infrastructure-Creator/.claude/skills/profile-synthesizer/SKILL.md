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

The target project path is a **required** argument. This skill reads only the current run's `tasks/TASK-{N}/` findings files (plus, if needed, the target's files to break a tie); it never writes into the target.

## Generated File Naming Convention (MANDATORY)

Write exactly one file: `tasks/TASK-{N}/infra-scan-project-profile.md`, following `references/project-profile-schema.md` exactly.

## Process

1. **Load all inputs** from `tasks/TASK-{N}/`: the six `*-findings.md`, `stack-researcher-findings.md`, and `clarifying-interview-answers.md`.
2. **Populate section 1 (AI Tool Selection)** strictly from the interview answer. If it is missing, STOP and re-run `clarifying-interview` - never assume an edition.
3. **Merge sections 2-7** from the scanners, preserving each fact's confidence tag and source path. When two scanners disagree, prefer the higher-confidence, more direct evidence and note the resolution.
4. **Fold in research notes (section 8)** from `stack-researcher`, keeping source URLs.
5. **Resolve open items (section 9)** using interview answers; anything still unresolved stays `unknown`, explicitly listed.
6. **Derive section 10 ("Skills to generate")** from sections 2-4: always an architecture skill and the universal PHP skills (coding, testing, code-review, security-review, performance, release, debugging), plus exactly one skill per `confirmed` integration in section 4. Record non-PHP neighbors as integration contracts only.
7. **Self-validate** against `references/project-profile-schema.md`: every line in sections 2-7 has a confidence tag; section 1 lists >=1 edition sourced from the interview; no skill is proposed for an absent integration; no secrets anywhere; a `confirmed` integration cites runtime wiring.
8. **Write the profile** and report.

## Output Template

```markdown
# Profile Synthesized: [target_name]

**File:** tasks/TASK-{N}/infra-scan-project-profile.md
**Editions:** [selected]
**Skills to generate:** [count] ([list])
**Confidence summary:** [X confirmed, Y inferred, Z unknown]

## Validation
[schema self-check result: pass / issues fixed]

## Review Before Generating
Read the profile and correct anything wrong, then run `infra-generate`.
```

## Guardrails

- MUST conform to `references/project-profile-schema.md` exactly.
- MUST NOT assume the AI-tool selection - it comes only from the interview.
- MUST NOT propose a skill for an integration with no evidence.
- MUST NOT include any secret or credential value.
- MUST keep every fact's confidence tag and source; never launder an `inferred` fact into a `confirmed` one.

## Final Output

Return the profile path, the selected editions, the derived skill list, the confidence summary, and the next step (user reviews, then runs `infra-generate`).
