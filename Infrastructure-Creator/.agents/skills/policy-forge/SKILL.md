---
name: policy-forge
description: Generate the target PHP project's governing policy documents from an approved Project Profile - one shared AGENTS.md at the target root, plus DOD.md, GOLDEN-PRINCIPLES.md, and STABILIZATION.md duplicated into each selected edition folder. Content is tailored to the target's real stack, architecture, security posture, and conventions - never templated. Use once profile-synthesizer has produced a profile. Triggers on "generate policy", "forge AGENTS.md", "write the target's DOD/principles".
phase: generation
flow-next: skill-forge
flow-alternatives: [hook-forge, memory-seed]
related: [infra-generate, skill-forge, agent-forge, command-forge, hook-forge, memory-seed, skill-flow-composer, bootstrap-verifier]
---

# Policy Forge

## Overview

`policy-forge` writes the target project's governance layer: the operational rules any AI edition must obey when working in that repository. It produces one shared `AGENTS.md` at the target root - the single source of policy truth regardless of which editions are installed - and duplicates the three enforcement companions (`DOD.md`, `GOLDEN-PRINCIPLES.md`, `STABILIZATION.md`) inside each selected edition folder so each edition ships self-contained. Every rule is authored from the profile's confirmed evidence: the real PHP version and framework (section 2), the detected architecture and layering (section 3), the actual auth/secrets/security posture (section 6), and the real code-style/git-hook/docs conventions (section 7). It never emits a rule for tooling the target does not have.

Consumes profile sections **1** (which editions), **2** (stack + real command lines), **3** (architecture boundaries), **6** (security/secrets), and **7** (conventions).

## Generated File Naming Convention (MANDATORY)

Into the target, write:
- `AGENTS.md` at the target ROOT - a SINGLE shared file (never per edition).
- For each selected edition folder in `{.claude, .cursor, .codex}`: `<edition>/DOD.md`, `<edition>/GOLDEN-PRINCIPLES.md`, `<edition>/STABILIZATION.md` (identical copies duplicated into each selected edition).

Append a generation log to `tasks/TASK-{N}/policy-forge-log.md` listing every file written and the profile lines each rule is grounded in.

## Process

1. **Read the profile.** Confirm selected editions (section 1). Extract the real toolchain from section 2 (test runner + config, lint/format tool, static-analysis tool + config, entry points, package manager), architecture facts (section 3), security facts (section 6), and conventions (section 7).
2. **Author `AGENTS.md`** at the target root as the shared policy. Encode: the target's real tooling command lines (only commands whose tools were detected), safety rules derived from section 6 (never read/print/commit secrets, parameterized queries, upload/authorization constraints as applicable), the target's architecture boundaries from section 3, and the target's own file-naming + verification policy. Each command line MUST cite the config/file that proves it exists.
3. **Author `DOD.md`** as the Definition of Done: the exact checks to run before claiming completion (tests, format/lint, static analysis) using only the tools found; report absent tooling as `N/A - not configured`.
4. **Author `GOLDEN-PRINCIPLES.md`**: the durable, stack-specific non-negotiables (SOLID/DRY/KISS applied to the detected framework, the source-of-truth hierarchy, secrets discipline).
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

## Log
tasks/TASK-{N}/policy-forge-log.md

## Next
skill-forge; hook-forge/memory-seed if not already run.
```

## Guardrails

- MUST write `AGENTS.md` as a SINGLE shared file at the target root - never per edition.
- MUST duplicate `DOD.md`, `GOLDEN-PRINCIPLES.md`, `STABILIZATION.md` into EACH selected edition folder, and only selected ones.
- MUST author every command/rule from confirmed profile evidence with a source citation; MUST NOT emit a check for a tool the target lacks.
- MUST NOT include any secret or credential value in any generated document.
- MUST keep the three companions byte-identical across editions in a single run.

## Final Output

Return the root `AGENTS.md` path, the per-edition companion paths, the profile sections consumed, the log path, and the next step (`skill-forge`).
