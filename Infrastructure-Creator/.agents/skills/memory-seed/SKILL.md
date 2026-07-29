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

`memory-seed` bootstraps the target project's own `memory-bank/` - the durable, indexed shared-memory layer the generated accelerator relies on - and seeds it with an initial set of `active` chunks drawn strictly from confirmed profile evidence. One shared `memory-bank/` is created at the target root (not per edition), so it survives when the team prunes editions. `skill-forge` separately generates the operational `memory-bank` skill in every selected edition so users can retrieve, capture, supersede, and audit this shared bank after generation.

The profile's section 12 ("Memory Bank Preview") already lists exactly what this skill is expected to produce. It plans one chunk per cohesive durable concept, composed only from confirmed evidence, rather than one tiny chunk per factual line. This skill's job is to **fulfill that preview**, not re-derive it: same count, same concepts, same sources. If the target changed, flag drift rather than silently seeding stale content.

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

1. **Read profile section 12 first** - it is the authoritative seed plan, already reviewed by the user. Cross-check it against confirmed facts in sections 2-8, including canonical source authority, durable invariants, lifecycles, permissions, audit obligations, integration contracts, and sanitized incident-prevention rules.
2. **Bootstrap the skeleton:** write `README.md` (fresh prose naming the target and its memory contract: authority hierarchy, layout, what belongs here, retrieval, creating/updating a chunk, lifecycle, security), copy `templates/chunk.md` and `scripts/validate.py` verbatim, create empty gitignored `local/`.
3. **Seed one chunk per row in section 12's preview table**, starting at `MEM-0001`, in the same order. Fill frontmatter exactly per `templates/chunk.md`. Each chunk represents one cohesive concept and links all canonical sources that prove it. It may group tightly related facts (for example, a lifecycle's statuses, confirmed transitions, guards, permission, and audit consequence) but MUST NOT copy full specs, schemas, permission matrices, test inventories, incident narratives, or logs. If revalidation surfaces a new confirmed concept or invalidates a previewed one, report drift rather than silently reconciling it.
4. **Write `INDEX.md`** with the exact 8-column table: `ID | Title | Type | Scope | Tags | Status | Last Verified | File`.
5. **Set `.memory-counter`** to one past the highest allocated ID.
6. **Run `validate.py`** (`python3 memory-bank/scripts/validate.py`) and fix any structural error before declaring success.

## Output Template

```markdown
# Memory Seed Complete: [target_name]

**Seeded chunks:** [count] (profile's section 12 previewed: [count])
- [MEM-0001: title (source)]
- ...

**Counter:** [value]
**validate.py:** [pass/fail]
**Drift from preview:** none | [what changed and why]

## Next
skill-flow-composer, once all forges have finished.
```

## Guardrails

- MUST copy `assets/scripts/validate.py` and `assets/templates/chunk.md` byte-for-byte - do not "improve" them.
- MUST seed only cohesive concepts composed from `confirmed` facts, each with canonical source paths; never seed an `inferred`/`unknown` item as fact.
- MUST seed the same set of chunks section 12 previewed - same count, same concepts, same sources - unless the target genuinely changed, in which case report drift explicitly.
- MUST NOT duplicate a canonical spec/schema/test inventory/permission matrix or include raw incident logs, customer data, payloads, or sensitive operational detail.
- MUST preserve authority: current policy, canonical specs, code, migrations, constraints, and tests outrank memory.
- MUST NOT include any secret or credential value in any chunk.
- MUST run `validate.py` and fix all reported errors before reporting success.
- MUST create ONE shared `memory-bank/` at the target root, not per edition.

## Final Output

Return the seeded chunk list (ID, cohesive concept title, canonical sources), how it compares to section 12's preview, the final counter value, the validator result, the log path, and the next step (`skill-flow-composer`).
