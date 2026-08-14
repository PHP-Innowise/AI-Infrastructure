---
name: memory-seed
description: Bootstrap the target PHP project's full memory layer - the durable memory-bank/ (with the dependency-free context-brain runtime under memory-bank/scripts/) and the governed project-brain/ control plane - and seed the bank with initial active memory chunks drawn strictly from confirmed Project Profile findings, each cited to the real file that proves it. Use once profile-synthesizer has produced a profile. Triggers on "seed memory bank", "bootstrap memory-bank", "bootstrap project brain", "initialize memory for the target".
phase: generation
flow-next: skill-flow-composer
flow-alternatives: [policy-forge, skill-forge, hook-forge]
related: [infra-generate, policy-forge, skill-forge, agent-forge, command-forge, hook-forge, skill-flow-composer, bootstrap-verifier]
---

# Memory Seed

## Overview

`memory-seed` bootstraps the target project's complete memory layer and seeds it
with confirmed profile evidence. During generation it writes both shared roots
under the required task staging root, while resolving citations against the
real target. They are published once at the target root only after the complete
bundle passes.

- **`memory-bank/`** - the durable, indexed shared-memory layer, plus the **context-brain runtime** under `memory-bank/scripts/`: `context.py` (the CLI facade), `brain_runtime.py` (governed Project Brain runtime), `context_retrieval.py` (local SQLite/BM25 retrieval), and `validate.py` (the bank validator). The runtime is dependency-free (standard library only) and stack-agnostic.
- **`project-brain/`** - the governed control plane for active work: dynamic records (tasks, findings, bugs, incidents, decisions, events), handoffs, the append-only agent message channel that orchestrated flows write to, retrieval manifests, promotion proposals, schemas, and `PROTOCOL.md`. The runtime under `memory-bank/scripts/` operates it.

`skill-forge` separately generates the operational skills that drive this layer in every selected edition (`memory-bank`, `project-brain`, `checkpoint`, `memory` - see `skill-forge/references/php-process-skills.md`), and `hook-forge` generates the working-memory hooks (`working-memory-read.sh` / `working-memory-write.sh`) that call `memory-bank/scripts/context.py refresh` / `turn` automatically. The paths this skill creates are the contract those hooks and skills depend on - never rename them.

The profile's section 12 ("Memory Bank Preview") already lists exactly what this skill is expected to seed. It plans one chunk per cohesive durable concept, composed only from confirmed evidence, rather than one tiny chunk per factual line. This skill's job is to **fulfill that preview**, not re-derive it: same count, same concepts, same sources. If the target changed, flag drift rather than silently seeding stale content.

Unlike every other artifact this forge produces, everything under this skill's bundled `assets/` is copied **verbatim** - the runtime scripts, the chunk template, and the `project-brain/` skeleton are dependency-free and stack-agnostic, so there is nothing target-specific to adapt (the single exception is `runtime.json.template`, which takes two substitutions - see below). Only `memory-bank/README.md`, `memory-bank/INDEX.md`, and chunk content are written fresh to describe the target's own memory contract and its own real findings.

## Generated File Naming Convention (MANDATORY)

Into the required generation root, create:

**`memory-bank/` (durable memory + runtime):**
- `memory-bank/README.md`, `memory-bank/INDEX.md`, `memory-bank/.memory-counter` (written fresh)
- `memory-bank/templates/chunk.md` (copied verbatim from `assets/templates/chunk.md`)
- `memory-bank/scripts/context.py`, `memory-bank/scripts/brain_runtime.py`, `memory-bank/scripts/context_retrieval.py`, `memory-bank/scripts/validate.py` (copied verbatim from `assets/scripts/`)
- `memory-bank/local/.gitkeep` (gitignored machine-local state: the disposable SQLite index `context.db`, turn buffers, ephemeral manifests)
- `memory-bank/chunks/MEM-{NNNN}-{short-slug}.md` per seeded chunk (starting at `MEM-0001`)

**`project-brain/` (governed control plane, copied verbatim from `assets/project-brain/`):**
- `PROTOCOL.md`, `README.md`, `.gitignore`
- `schemas/` (all six `*.schema.json` files, including `message.schema.json` for the agent channel), `templates/` (all nine record/control templates), `scripts/validate.py`
- `config/providers.json`, `config/telemetry.json`, and `config/runtime.json` (from `runtime.json.template`, after the one substitution below)
- `indexes/active.json`, `indexes/archive.json` (both `[]`)
- empty, `.gitkeep`-held directories: `archive/`, `local/`, `control/handoffs/`, `control/messages/`, `control/promotions/`, `control/retrieval-manifests/`, `dynamic/bugs/`, `dynamic/decisions/`, `dynamic/events/`, `dynamic/findings/`, `dynamic/incidents/`, `dynamic/tasks/`

**Target `.gitignore`:** ensure it excludes `memory-bank/local/` and `__pycache__/` (append if missing; `project-brain/` ships its own `.gitignore` for `local/`).

Append a log to `tasks/TASK-{N}/memory-seed-log.md`.

## Placeholder Substitution (`runtime.json.template`)

`assets/project-brain/config/runtime.json.template` becomes `project-brain/config/runtime.json` with exactly two substitutions:

1. **`{{TARGET_FRAMEWORK}}`** -> the target's framework slug - lowercase, kebab-case, from the profile's confirmed section 2 framework (`laravel`, `symfony`, `php-core`, `laminas`, ...). When section 2 has no confirmed framework, or the target is plain PHP with no framework, use `generic`. This is safe by design: the runtime treats `framework` as an opaque configuration value. The only framework-sensitive behavior is `MIRROR_RULES.skip_by_framework` inside `context_retrieval.py`, a lookup keyed by this slug - a slug with no entry (any non-`symfony` value today) simply contributes no framework-specific mirror exemptions, and `brain_runtime.py` itself defaults to `generic` when the key is absent. So an unrecognized slug degrades to "no framework exceptions", never to an error. Do NOT invent a known-framework slug (`laravel`/`symfony`) that the profile does not confirm just to look specific.
2. **`{{CANONICAL_EDITION}}`** -> the skills root `context.py parity` treats as the source of truth, derived from the profile's selected editions (section 1): `.agents` when Codex is selected, else `.claude` when Claude is selected, else `.cursor`. The value MUST name an edition root that will actually exist in the target: `skill_mirror_drift` in `context_retrieval.py` classifies every mirror file as drift when the canonical edition's skills tree is missing, so a claude-only target seeded with a hardcoded `.agents` fails `context.py parity` with a false total-drift report even though status/validate/refresh/turn are clean.

All other `runtime.json` values are shipped defaults (`mode: governed`, `automatic_promotion/completion/compaction: true`, `provider: sqlite-fts5`, telemetry off). Leave them as-is; the target team tunes them post-handoff.

## Process

1. **Read profile section 12 first** - it is the authoritative seed plan, already reviewed by the user. Cross-check it against confirmed facts in sections 2-8, including canonical source authority, durable invariants, lifecycles, permissions, audit obligations, integration contracts, and sanitized incident-prevention rules.
2. **Bootstrap `memory-bank/`:** write `README.md` (fresh prose naming the target and its memory contract: authority hierarchy, layout, what belongs here, retrieval, creating/updating a chunk, lifecycle, security, and the runtime CLI - `python3 memory-bank/scripts/context.py --help`), copy `templates/chunk.md` and all four `assets/scripts/*.py` verbatim, create empty gitignored `local/`.
3. **Bootstrap `project-brain/`:** copy the whole `assets/project-brain/` skeleton verbatim (including `.gitkeep` placeholders and both empty `[]` indexes), then materialize `config/runtime.json` from the template with the two substitutions above (framework slug + canonical edition).
4. **Seed one chunk per row in section 12's preview table**, starting at `MEM-0001`, in the same order. Fill frontmatter exactly per `templates/chunk.md` (JSON frontmatter; `valid_from`/`valid_to` are optional temporal-validity keys - seed chunks normally set `valid_from` to the seed date and leave `valid_to` null). Each chunk represents one cohesive concept and links all canonical sources that prove it. It may group tightly related facts (for example, a lifecycle's statuses, confirmed transitions, guards, permission, and audit consequence) but MUST NOT copy full specs, schemas, permission matrices, test inventories, incident narratives, or logs. If revalidation surfaces a new confirmed concept or invalidates a previewed one, report drift rather than silently reconciling it.
5. **Write `INDEX.md`** with the exact 8-column table: `ID | Title | Type | Scope | Tags | Status | Last Verified | File`.
6. **Set `.memory-counter`** to one past the highest allocated ID.
7. **Run the validators and smoke checks** from the staged generation root; fix any structural error before declaring success:
   - `python3 memory-bank/scripts/validate.py` (bank structure)
   - `python3 memory-bank/scripts/context.py validate` (Project Brain records - passes on the empty skeleton)
   - `python3 memory-bank/scripts/context.py status` (runtime imports and index health; exit 0 proves the four scripts and the skeleton are wired correctly)

## Output Template

```markdown
# Memory Seed Complete: [target_name]

**Seeded chunks:** [count] (profile's section 12 previewed: [count])
- [MEM-0001: title (source)]
- ...

**Runtime:** memory-bank/scripts/ (context.py, brain_runtime.py, context_retrieval.py, validate.py - verbatim)
**Project Brain:** project-brain/ skeleton (framework slug: [slug], canonical edition: [.agents/.claude/.cursor])
**Counter:** [value]
**validate.py:** [pass/fail] | **context.py validate:** [pass/fail] | **context.py status:** [pass/fail]
**Drift from preview:** none | [what changed and why]

## Next
skill-flow-composer, once all forges have finished.
```

## Guardrails

- MUST copy every file under `assets/` byte-for-byte - runtime scripts, chunk template, and the `project-brain/` skeleton included; do not "improve", trim, or re-generate them. The only sanctioned edits are the `{{TARGET_FRAMEWORK}}` and `{{CANONICAL_EDITION}}` substitutions in `runtime.json.template` (which is copied under the name `config/runtime.json`).
- MUST leave no `{{TARGET_FRAMEWORK}}` or `{{CANONICAL_EDITION}}` placeholder in the written `project-brain/config/runtime.json`; MUST use `generic` rather than an unconfirmed framework slug; and MUST set `canonical_edition` to an edition root that is actually generated (`.agents` for Codex, else `.claude`, else `.cursor`) - a canonical edition absent from the tree makes `context.py parity` report false total drift.
- MUST seed only cohesive concepts composed from `confirmed` facts, each with canonical source paths; never seed an `inferred`/`unknown` item as fact.
- MUST seed the same set of chunks section 12 previewed - same count, same concepts, same sources - unless the target genuinely changed, in which case report drift explicitly.
- MUST NOT duplicate a canonical spec/schema/test inventory/permission matrix or include raw incident logs, customer data, payloads, or sensitive operational detail.
- MUST preserve authority: current policy, canonical specs, code, migrations, constraints, and tests outrank memory; `project-brain/` outranks retrieval caches; `PROTOCOL.md` governs every Brain mutation.
- MUST NOT include any secret or credential value in any chunk.
- MUST run all three checks in step 7 and fix every reported error before reporting success.
- MUST create ONE shared `memory-bank/` and ONE shared `project-brain/` at the target root, not per edition, and MUST NOT rename any runtime path (`hook-forge`'s working-memory hooks call `memory-bank/scripts/context.py` at exactly that path).

## Final Output

Return the seeded chunk list (ID, cohesive concept title, canonical sources), how it compares to section 12's preview, the framework slug and canonical edition written to `runtime.json`, the final counter value, the three validator/smoke results, the log path, and the next step (`skill-flow-composer`).
