---
name: memory-seed
description: Bootstrap the target PHP project's full memory layer - the durable memory-bank/ (with its bundled native Rust engine under memory-bank/bin/) and the governed project-brain/ control plane - and seed the bank with initial active memory chunks drawn strictly from confirmed Project Profile findings, each cited to the real file that proves it. Use once profile-synthesizer has produced a profile. Triggers on "seed memory bank", "bootstrap memory-bank", "bootstrap project brain", "initialize memory for the target".
phase: generation
flow-next: skill-flow-composer
flow-alternatives: [policy-forge, skill-forge, hook-forge]
related: [infra-generate, policy-forge, skill-forge, agent-forge, command-forge, hook-forge, skill-flow-composer, bootstrap-verifier]
---

# Memory Seed

Native helper commands use the generator's explicit bundled executable. Set `GENERATOR_NATIVE` to the absolute path `<generator>/memory-bank/bin/infra-engine` (`infra-engine.exe` on Windows). Use the selected release artifact; it is never located on PATH, downloaded or compiled during generation.

## Overview

`memory-seed` writes the shared native memory layer into the required task staging root and seeds it from the approved Project Profile. Citations are checked against the real target. Publication happens once after the complete staged accelerator passes its gates.

- `memory-bank/` holds durable chunks, their index and templates, the trusted native runtime contract and mirror rules, and the selected Rust executable at `memory-bank/bin/infra-engine[.exe]`. The native engine implements retrieval, checkpoints, governed results, handoffs, chat continuity, MCP and validation with bundled SQLite/FTS5.
- `project-brain/` holds governed tasks, findings, bugs, incidents, decisions, events, handoffs, messages, retrieval manifests, promotions, schemas and protocol. It is operated by the same engine.
- `memory-bank/local/context.db` is an ignored disposable index. Current project sources and shared Brain/Bank records remain authoritative.

`skill-forge` separately creates the six runtime-fixed guides: `memory-bank`, `project-brain`, `checkpoint`, `memory`, `context-save` and `context-load`. `hook-forge` emits the native host launch fields and compatibility shims for the supported Rust hook roles. None of those shims parses JSON, discovers task identity, retrieves memory or implements publication.

The bundled `assets/runtime-contract.json` is the source for paths, operations, SQLite checkpoint tables, ownership and chat-snapshot boundaries. Its native contract must match the trusted contract compiled into the selected engine. The artifact metadata at `assets/native-artifacts.json` pins the exact engine bytes, platform target and required capabilities. Preserve both contracts.

Profile section 12 is the approved seed plan: one cohesive durable concept per row, the same count, concepts and sources. Revalidate it against confirmed evidence; report drift instead of silently replacing the plan.

## Generated paths

Create one shared memory layer at the generation root:

- `memory-bank/README.md`, `INDEX.md` and `.memory-counter` are authored for the target.
- `memory-bank/bin/infra-engine` or `infra-engine.exe` is the selected native artifact, copied by the scoped native seed operation.
- `memory-bank/runtime-contract.json`, `memory-bank/MCP.md`, `memory-bank/config/mirror-rules.json` and `memory-bank/templates/chunk.md` are the bundled static resources.
- `memory-bank/local/` is empty ignored machine state. Seed durable chunks under `memory-bank/chunks/` with IDs allocated from the preview plan.
- `project-brain/` receives its protocol, README, ignore rules, schemas, templates, provider/telemetry/runtime config, empty active/archive indexes and empty record/control directories from the trusted assets.
- No executable Python file, interpreter launcher or runtime fallback is part of the payload.

Keep runtime state out of manifest ownership: chunks/index/counter, local state, Brain indexes and record/control directories belong to the target team after seeding. Static resources and the native artifact may be owned by the explicit write plan.

## Runtime configuration

Materialize `assets/project-brain/config/runtime.json.template` with exactly the supported profile substitutions:

1. The confirmed framework slug from section 2; use `generic` when no framework is confirmed. Never invent a supported framework to make the output look specific.
2. The actual canonical skills root from selected clients: `.agents` for Codex, otherwise `.claude`, otherwise `.cursor`. It must exist in the generated tree.

Keep the other shipped runtime defaults. Approved declarative hook policy is emitted separately by `hook-forge` through its native evidence-bound service; it does not become shell code.

## Process

1. Read profile section 12 and cross-check its concepts against confirmed sections 2–8: canonical authority, invariants, lifecycle, permissions, audit obligations, integration contracts and sanitized incident-prevention rules.
2. Seed only the trusted memory/Brain assets into staging using the native scoped operation:

   ```sh
   "$GENERATOR_NATIVE" creator seed-assets \
     --assets "<generator>/.agents/skills/memory-seed/assets" \
     --staging "<generation-root>" \
     --framework "<confirmed-slug-or-generic>" \
     --editions "<selected-clients>"
   ```

   The operation selects the explicit current-platform artifact from the bundled metadata, verifies its digest/capabilities, copies only the runtime/static skeleton, and applies the two config substitutions. It must not overlay a ready edition's policy, skills, commands or agents. The separate `seed-runtime` ready-edition overlay is not this forge's operation.
3. Write the target-specific Bank README/index/counter and register project Memory MCP for only the selected hosts:

   ```sh
   "$GENERATOR_NATIVE" repo mcp-config \
     --root "<generation-root>" --delivery-root "<real-target>" \
     --tool "<selected-client>"
   ```

   Repeat `--tool` for each selected client. The helper writes only staged configurations but binds their executable and runtime root to the final target. Preserve external servers and existing Codex configuration; registration grants no trust. Include these host MCP files in the explicit write plan.

4. **Seed one chunk per row in section 12's preview table**, starting at `MEM-0001`, in the same order. Fill frontmatter exactly per `templates/chunk.md` (JSON frontmatter; `valid_from`/`valid_to` are optional temporal-validity keys - seed chunks normally set `valid_from` to the seed date and leave `valid_to` null). Each chunk represents one cohesive concept and links all canonical sources that prove it. It may group tightly related facts (for example, a lifecycle's statuses, confirmed transitions, guards, permission, and audit consequence) but MUST NOT copy full specs, schemas, permission matrices, test inventories, incident narratives, or logs. If revalidation surfaces a new confirmed concept or invalidates a previewed one, report drift rather than silently reconciling it.

   **The body serializes the rule itself - a chunk whose body could be
   regenerated from its own title has preserved nothing.** A measured
   publication seeded fifteen chunks that all carried the same sentence with
   the title and paths substituted; `creator validate-memory-content` now refuses
   that, and this is what each section must actually hold, drawn from the
   run's own evidence (the domain findings, `project-claims.json`, and the
   plan contracts that cite these sources):
   - **Durable Context** - the smallest concrete rule, stated as behavior:
     the condition, the required outcome, the forbidden outcome, and the
     consequence, with real symbols, statuses, values, and `path:Lrange`
     citations inline. *"When the integration export throws, the persisted
     status must remain FAILED and no FINISHED completion effects may run"* -
     not *"X is a confirmed authority boundary; use only the cited ranges."*
   - **Consequences** - what future code and tests must preserve, named
     concretely, including at least one forbidden outcome a regression test
     should assert. Never process instructions like "re-open the sources."
   - **Verification** - per cited source: what that exact range proves, and
     which file or behavior change triggers re-review of this chunk.
   - **A contradiction chunk records the conflict, not a verdict**: claim A
     with its cited range, claim B with its cited range, the current safe
     operating rule, and the owner or condition for resolution - and its
     status is `needs-review`, because an unresolved conflict presented as
     `active` settled knowledge is a lie with a citation. The same applies
     to any finding the profile carries as contradictory.
   - Bodies are written one concept at a time from that concept's own
     evidence; no two chunks may share sentence scaffolding once their
     nouns are erased.
5. Write `INDEX.md` with exactly `ID | Title | Type | Scope | Tags | Status | Last Verified | File`.
6. Set `.memory-counter` to one past the highest allocated seed ID.
7. Write the task-scoped root-ignore requirements contract. Keep `.context-handoff/` and `memory-bank/local/` ignored; preserve the approved shared-file composition contract. Never read, append or replace the target root ignore file directly.
8. From the staged generation root, run native structure, substance and health checks:

   ```sh
   "<generation-root>/memory-bank/bin/infra-engine" bank validate \
     --root "<generation-root>" --source-root "<real-target>"
   "$GENERATOR_NATIVE" creator validate-memory-content \
     --bank "<generation-root>/memory-bank" --target "<real-target>"
   "<generation-root>/memory-bank/bin/infra-engine" memory \
     --root "<generation-root>" validate
   "<generation-root>/memory-bank/bin/infra-engine" memory \
     --root "<generation-root>" status
   ```

   Use `infra-engine.exe` on Windows. Verify the copied runtime contract matches the bundled trusted contract and its required skeleton exists. Creatable paths are allowed to remain absent. An error blocks completion.

## Output

Return the seeded chunk IDs, concrete concept titles and canonical sources; agreement or drift against section 12; framework and canonical edition; counter; native artifact identity; structure/substance/health results; log path; and `skill-flow-composer` as the next step after all forges finish.

## Guardrails

- Preserve the exact trusted static resources and native artifact. Only supported runtime-template substitutions and approved declarative hook policy are target-specific.
- Create one shared Bank and Brain at the target root, not one per client.
- Seed only confirmed cohesive concepts and preserve the reviewed preview. Do not promote inferred or unknown claims to facts.
- Serialize the actual rule: condition, required and forbidden behavior, consequence, concrete symbols and cited ranges. Do not reuse a boilerplate body after merely changing names.
- A contradiction records both cited claims, the current safe rule and the resolution owner/condition under `needs-review`.
- Explain what each cited source proves and which change triggers review. Avoid copied specs, schemas, permission matrices, raw logs, incident transcripts and secrets.
- Preserve authority: policy and canonical sources outrank Brain; Brain outranks Bank; the local index is disposable.
- Compile the six runtime-fixed guides from the trusted native runtime contract. Do not invent operations or paths.
- Chat snapshots and prepared merges remain ignored local visible-text convenience state. They never admit private account stores, hidden reasoning or system/tool envelopes into Brain, Bank or SQLite.
- Native result capture uses the existing structured result guards. It never treats arbitrary chat text as an attested save or completes a task automatically.
