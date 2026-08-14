---
name: policy-forge
description: Generate the target PHP project's governing policy documents from an approved Project Profile - one shared AGENTS.md at the target root, plus DOD.md, GOLDEN-PRINCIPLES.md, and STABILIZATION.md duplicated into each selected edition folder. Content is tailored to the target's real stack, architecture, security posture, conventions, sources of truth, and confirmed behavioral contract - never templated. Use once profile-synthesizer has produced a profile. Triggers on "generate policy", "forge AGENTS.md", "write the target's DOD/principles".
phase: generation
flow-next: skill-forge
flow-alternatives: [hook-forge, memory-seed]
related: [infra-generate, skill-forge, agent-forge, command-forge, hook-forge, memory-seed, skill-flow-composer, bootstrap-verifier]
---

# Policy Forge

## Overview

`policy-forge` writes the target project's governance layer: the operational rules any AI edition must obey when working in that repository. It produces one shared `AGENTS.md` at the target root - the single source of policy truth regardless of which editions are installed - and duplicates the three enforcement companions (`DOD.md`, `GOLDEN-PRINCIPLES.md`, `STABILIZATION.md`) inside each selected edition folder so each edition ships self-contained. Every rule is authored from confirmed profile evidence: the real PHP stack (section 2), architecture (section 3), security posture (section 6), conventions (section 7), and only the high-value behavioral contract in section 8 (project-specific authority, critical invariants, authorization boundaries, forbidden/high-risk behavior with documented governance, audit obligations, and required regression scenarios). It never emits a rule for absent tooling or turns inferred behavior into policy.

Consumes profile sections **1** (which editions), **2** (stack + real command
lines), **3** (architecture boundaries), **6** (security/secrets), **7**
(conventions), and confirmed high-value rules from **8** (behavioral contract),
plus the validated generation plan's skill inventory and routing boundaries.

## Generated File Naming Convention (MANDATORY)

Into the required **generation root** (task staging during
`infra-generate`/`infra-update`; never the evidence target directly), write:
- `AGENTS.md` at the target ROOT - a SINGLE shared file (never per edition).
- For each selected edition folder in `{.claude, .cursor, .codex}`: `<edition>/DOD.md`, `<edition>/GOLDEN-PRINCIPLES.md`, `<edition>/STABILIZATION.md` (identical copies duplicated into each selected edition).

Append a generation log to `tasks/TASK-{N}/policy-forge-log.md` listing every file written and the profile lines each rule is grounded in.

## Process

1. **Read the profile.** Confirm selected editions (section 1). Extract the real toolchain from section 2, architecture facts from section 3, security facts from section 6, conventions from section 7, and confirmed high-value behavioral rules from section 8. Preserve source type and any contradiction; an interview answer or implementation path does not silently outrank an explicit spec/ADR or database constraint.
2. **Author `AGENTS.md`** in staging as the shared policy. Encode real tooling,
   safety, architecture, sources of truth, and only globally applicable
   high-value behavior. Project invariants that belong to one bounded context
   stay in their owning skill/rule instead of being copied into every workflow.
   The **Subagents** section permits only generated roster agents. The scoped
   **Orchestration** section requires contract-based specialist selection,
   read-only parallelism, serialized writers, bounded capsules, checkpoints,
   and stop-on-failure behavior.
3. **Author `DOD.md`** as the Definition of Done: exact tests/format/static-analysis commands plus affected confirmed critical scenarios, denied paths, transitions, and audit checks when a change touches their scope. Report absent tooling as `N/A - not configured`.
4. **Author `GOLDEN-PRINCIPLES.md`**: durable stack-specific non-negotiables, project-specific source authority, critical behavioral invariants, and secrets discipline.
5. **Author `STABILIZATION.md`**: the error-to-rule loop the target uses to convert recurring mistakes into permanent rules.
6. **Duplicate** `DOD.md`, `GOLDEN-PRINCIPLES.md`, `STABILIZATION.md` into every selected edition folder (byte-identical copies). Do NOT write into unselected editions.
7. **Log** every written path and the profile line backing each command/rule.

## Output Template

```markdown
# Policy Forge Complete: [target_name]

**Editions:** [selected]
**Root policy:** AGENTS.md (shared, 1 file)
**Per-edition companions:** DOD.md, GOLDEN-PRINCIPLES.md, STABILIZATION.md x [edition count]

## Grounding
- Stack commands: [profile section 2 lines cited]
- Architecture rules: [section 3]
- Security rules: [section 6]
- Conventions: [section 7]
- Behavioral rules: [confirmed section 8 lines and canonical sources]

## Log
tasks/TASK-{N}/policy-forge-log.md

## Next
skill-forge; hook-forge/memory-seed if not already run.
```

## Guardrails

- MUST write `AGENTS.md` as a SINGLE shared file at the target root - never per edition.
- MUST duplicate `DOD.md`, `GOLDEN-PRINCIPLES.md`, `STABILIZATION.md` into EACH selected edition folder, and only selected ones.
- MUST author every command/rule from confirmed profile evidence with a source citation; MUST NOT emit a check for a tool the target lacks.
- MUST preserve section 8 source type and contradictions; MUST NOT turn implementation behavior or an interview answer into stronger policy than its evidence supports.
- MUST include only high-value behavioral rules in policy and link canonical sources; MUST NOT copy the full domain profile into every policy file.
- MUST NOT invent severity, ownership, approval, legal obligations, or a complete permission/transition matrix.
- MUST NOT include any secret or credential value in any generated document.
- MUST keep the three companions byte-identical across editions in a single run.
- MUST derive the generated roster/orchestration policy from the validated
  generation plan and MUST NOT mention pruned or unvalidated skills.

## Final Output

Return the root `AGENTS.md` path, the per-edition companion paths, the profile sections consumed, the log path, and the next step (`skill-forge`).
