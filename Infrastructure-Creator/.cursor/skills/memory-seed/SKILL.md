---
name: memory-seed
description: Bootstrap a memory-bank/ directory at the target PHP project's root and seed it with initial active memory chunks drawn strictly from confirmed Project Profile findings, each cited to the real file that proves it. Use once profile-synthesizer has produced a profile. Triggers on "seed memory bank", "bootstrap memory-bank", "initialize memory for the target".
phase: generation
flow-next: skill-flow-composer
flow-alternatives: [policy-forge, skill-forge, hook-forge]
related: [infra-generate, policy-forge, skill-forge, agent-forge, command-forge, hook-forge, skill-flow-composer, bootstrap-verifier]
---

# Memory Seed

## Overview

`memory-seed` bootstraps the target project's own `memory-bank/` - the durable, indexed shared-memory layer the generated accelerator relies on - and seeds it with an initial set of `active` chunks drawn strictly from the profile's `confirmed` findings. One shared `memory-bank/` is created at the target root (not per edition), so it survives when the team prunes editions.

Unlike every other artifact this forge produces, `scripts/validate.py` and `templates/chunk.md` are copied verbatim from this skill's bundled `assets/` - they are dependency-free and stack-agnostic, so there is nothing target-specific to adapt. Every other file (`README.md`, `INDEX.md`, chunk content) is written fresh to describe the target's own memory contract and its own real findings.

## Generated File Naming Convention (MANDATORY)

Into the target root, create:
- `memory-bank/README.md`, `memory-bank/INDEX.md`, `memory-bank/.memory-counter`
- `memory-bank/templates/chunk.md` (copied verbatim from `assets/templates/chunk.md`)
- `memory-bank/scripts/validate.py` (copied verbatim from `assets/scripts/validate.py`)
- `memory-bank/local/.gitkeep` (gitignored personal notes area)
- `memory-bank/chunks/MEM-{NNNN}-{short-slug}.md` per seeded chunk (starting at `MEM-0001`)

Append a log to `tasks/TASK-{N}/memory-seed-log.md`.

## Process

1. **Read the profile.** Collect every `confirmed` fact worth remembering across sections 2-7 (framework/version, architecture pattern, each confirmed integration, auth pattern, conventions).
2. **Bootstrap the skeleton:** write `README.md` (fresh prose naming the target and its memory contract: authority hierarchy, layout, what belongs here, retrieval, creating/updating a chunk, lifecycle, security), copy `templates/chunk.md` and `scripts/validate.py` verbatim, create empty gitignored `local/`.
3. **Seed one chunk per durable fact**, starting at `MEM-0001`. Fill the JSON frontmatter exactly per `templates/chunk.md`'s required keys. `type` from the allowed set (architecture/constraint/convention/decision/domain/integration/operations); `status: active`; `sources` MUST cite the real target file that proves the fact.
4. **Write `INDEX.md`** with the exact 8-column table: `ID | Title | Type | Scope | Tags | Status | Last Verified | File`.
5. **Set `.memory-counter`** to one past the highest allocated ID.
6. **Run `validate.py`** (`python3 memory-bank/scripts/validate.py`) and fix any structural error before declaring success.

## Output Template

```markdown
# Memory Seed Complete: [target_name]

**Seeded chunks:** [count]
- [MEM-0001: title (source)]
- ...

**Counter:** [value]
**validate.py:** [pass/fail]

## Next
skill-flow-composer, once all forges have finished.
```

## Guardrails

- MUST copy `assets/scripts/validate.py` and `assets/templates/chunk.md` byte-for-byte - do not "improve" them.
- MUST seed only `confirmed` facts, each with a real source path; never seed an `inferred`/`unknown` item as fact.
- MUST NOT include any secret or credential value in any chunk.
- MUST run `validate.py` and fix all reported errors before reporting success.
- MUST create ONE shared `memory-bank/` at the target root, not per edition.

## Final Output

Return the seeded chunk list (ID, title, source), the final counter value, the `validate.py` result, the log path, and the next step (`skill-flow-composer`).
