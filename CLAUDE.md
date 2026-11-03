# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Repository Is

This is **not a PHP application**. It is a monorepo of AI-agent accelerators for PHP
projects: ready-made policy/skill/agent/hook bundles for Laravel, Symfony,
framework-neutral PHP Core, and WordPress, plus `Infrastructure-Creator/`, a generator that scans
a *target* PHP project and builds a bespoke accelerator for it. There is no
application code, no Composer install, and no PHP runtime to execute here — the
repository's production runtime and tooling are Rust, built from
`runtime-rust/`. Retained Python tests and fixtures document the retired
implementation; they are not executable production dependencies. QA workbook
processing and Harness orchestration run in the same native engine.

```
Laravel/, Symfony/, PHP Core/    # framework ready-to-use editions
Cms/wordpress/                   # WordPress ready-to-use edition
Infrastructure-Creator/          # generator that builds a bespoke edition for a target project
install/                         # installer docs + versioned inventories
scripts/                         # versioned tooling data and historical source references
harness/                         # browser assets and historical fixtures; native orchestration in runtime-rust/
tests/                           # repository-level tests (installation, mirrors, framework semantics)
docs/                            # shared documentation (CI, adoption, tool integration, operations)
```

Each ready-made accelerator (`Laravel/`, `Symfony/`, `PHP Core/`,
`Cms/wordpress/`) and `Infrastructure-Creator/` is a self-contained unit with its own
`AGENTS.md` (enforceable policy), `README.md`, `CHANGELOG.md`, `VERSION`,
`memory-bank/`, `project-brain/` (Infrastructure-Creator excepted), `specs/`,
`tasks/`, and mirrored tool integrations `.claude/`, `.cursor/`, `.agents/` +
`.codex/`. Changes to one edition are not automatically synchronized to the
others — verify a change is meaningful for the target stack before porting it.

## Commands

The production runtime and repository tools are native Rust. Run the platform's
`infra-engine` executable from a verified release or from
`runtime-rust/target/release/`; installed projects use
`memory-bank/bin/infra-engine` (`.exe` on Windows). Git is required for
source custody and repository operations. PHP is needed only for PHP snippet
validation; provider CLIs and optional ONNX/vector libraries are selected
explicitly for the operations that use them.

**Source-first migration gate:** complete the production Rust implementation
before dependency fetches, builds, formatting, tests, checks, or provider calls.
Keep those phases marked deferred until the implementation gate is closed.
The commands below describe the later validation and maintenance phase; their
presence is not evidence that they have run.

### Native validation after the implementation gate

```bash
cargo build --manifest-path runtime-rust/Cargo.toml --locked --bin infra-engine
infra-engine check --list
infra-engine check --strict --base origin/main
infra-engine check --group mirrors --group parity --strict
```

The declarative catalog is `install/check-catalog.json`. The same native
catalog runner backs local checks and generated GitHub workflows. Retained
Python tests and fixtures are historical references; they are not a production
runtime or a working differential oracle after their imported implementation
has been retired.

### Canonical resources and mirrors

```bash
infra-engine repo mirrors --check
infra-engine repo mirrors --write --edition Laravel --edition "PHP Core"
infra-engine repo core-sync --check
infra-engine repo core-sync --write
infra-engine repo asset-parity --check
infra-engine repo policy-lock --check
infra-engine repo budget --check
infra-engine repo links
infra-engine repo routes
infra-engine repo php-snippets --require-php
infra-engine repo stabilization
infra-engine repo changelog --base origin/main
```

Edit canonical resources only. Native mirror generation carries those edits to
generated tool trees. Runtime logic belongs in `runtime-rust/`; shared resource
changes start in `PHP Core/` and are propagated through the native tools.
Rebuilding policy locks or inventories is a deliberate payload change.

### Installation and attached mode

```bash
infra-engine install --verify-inventories
infra-engine install --write-inventories
infra-engine install --edition "PHP Core" --target /path/to/project --dry-run --merge-existing
infra-engine install --edition "PHP Core" --target /path/to/project --merge-existing
infra-engine attach run --project /path/to/project --tool codex
```

Review a dry-run's collisions before installation. Source hook and MCP
configuration files are templates: the native installer or generator binds the
selected executable and final target root before they are used. Do not replace
that materialization with shell ancestor searches or a Python bootstrap.

### Installed memory and native applications

```bash
memory-bank/bin/infra-engine memory start --task-id TASK-ID --goal "Goal"
memory-bank/bin/infra-engine memory index
memory-bank/bin/infra-engine memory retrieve "Query" --task-id TASK-ID
memory-bank/bin/infra-engine memory validate
infra-engine collect skills --edition Laravel --dry-run
infra-engine serve start --project /path/to/project
infra-engine app install
infra-engine harness run --project /path/to/project --worker dry-run
```

Memory global options (`--root`, `--project`, `--db`, `--mode`, `--owner`)
precede the memory operation. Ordinary memory and hook commands use the real
calendar. Historical evaluation clocks are accepted only through explicit
controlled evaluation routes; proof and deadline clocks remain real.
Collection bundles stay in ignored `.c2p/` and are never installed in clients.

## Architecture

### Shared workflow model: Command → Agent → Skill

Every ready-made edition follows the same pipeline, adapted per stack:

```
User request → Command routes intent → Agent executes exactly one Skill in an isolated context → stop
```

- A **Command** (Claude Code/Cursor only) routes intent to a workflow.
- An **Agent** stays thin: executes one skill, then stops — no automatic chaining.
- A **Skill** holds the actual procedure: checks, examples, decision criteria, output format.
- **Hooks** and `AGENTS.md` policy enforce security, naming, and verification conventions.

Codex has no command layer: skills are invoked by name or auto-selected from `.agents/skills/`.

`tasks/` holds temporary, skill-prefixed `TASK-NNN/` work docs. `specs/` holds
durable living specifications registered in `specs/MANIFEST.md`. Uppercase
`Task/` in the ready-made editions is optional client-owned input material —
distinct from lowercase `tasks/` — and is excluded from the installed payload.

### Three mirrored tool integrations per edition

| Tool | Reads | Notes |
| --- | --- | --- |
| Claude Code | `.claude/` | Canonical source for agents, commands, hooks, skills, settings. |
| Cursor | `.cursor/` | Self-contained mirror; skills/commands/agents/rules/hooks. |
| Codex | `.agents/skills/` + `.codex/` | No command/agent-wrapper layer; skills only. |

`.claude/skills` (and `.agents/skills`, the canonical source) generate the
`.cursor/skills` mirror; `infra-engine repo mirrors` enforces this per the
transformation rules in `scripts/README.md`. **Never hand-edit a generated
mirror file** — edit the canonical source and regenerate with `--write`, or
`infra-engine repo mirrors --check` (and CI's `mirrors`/`parity` jobs) will fail.

### Project Brain / Local Context Engine / Memory Bank

Three distinct persistence layers exist inside each edition (and inside a
generated target project):

- **Project Brain** (`project-brain/`) — Git-tracked shared authority for
  active tasks, handoffs, findings, decisions. Governed mode is default.
- **Local Context Engine** — indexes policy/skills/docs/specs/Project Brain
  records into ignored SQLite (`memory-bank/local/context.db`), retrieval via
  FTS5/BM25. Disposable: deleting it deletes no authoritative data, though it
  drops the turns buffered since the last checkpoint and the recent files.
- **Memory Bank** (`memory-bank/`) — Git-tracked durable knowledge chunks
  (`MEM-YYYYMMDD-xxxxxxxx-{slug}.md`), auto-promoted by default from resolved
  findings/incidents/decisions (tagged `auto-promoted`, unreviewed) unless
  `automatic_promotion` is set to `false` in `project-brain/config/runtime.json`.

Authority order (highest first): enforcement/policy/hooks/CI → canonical
project sources (code, config, migrations, tests, specs) → Project Brain →
Memory Bank → skills/operations → examples → README docs. Retrieved snippets
are discovery aids only, always verified against the cited canonical source.

The public CLI is `memory-bank/bin/infra-engine memory {start,update,retrieve,complete,index,parity,status,validate,compact,export}`,
run from an edition or a consuming project's root. `retrieve` is the current
name; `context` is a compatibility alias. `memory`/`checkpoint` (Codex) or
`/memory`/`/checkpoint` (Claude/Cursor) are the argument-free refresh/capture
entry points — see `AGENTS.md` in any edition for the exact MUST/MUST NOT rules.

### Infrastructure-Creator's own pipeline

`Infrastructure-Creator/` is itself a generator, not an edition to install.
Its workflow (`infra-scan → review → infra-generate`, or `infra-build` to
chain both) is invoked via AI-assistant commands/skills, not terminal
executables:

```
infra-scan <target>     # read-only; 7 parallel PHP scanners + stack-researcher +
                         # clarifying-interview -> tasks/TASK-{N}/infra-scan-project-profile.md
                         # (human review checkpoint — read the full report, not just the summary)
infra-generate <target> # revalidates the approved profile against current target state,
                         # stages policy/hooks/memory + evidence-scoped skill batches,
                         # runs a semantic gate, then agents/commands/flows, then bootstrap-verifier
infra-update <target>   # requires .infra-manifest.json; per-file decision for anything team-edited
```

Never point the generator at itself, and never copy `Infrastructure-Creator/`
into the target project. It supports non-PHP targets by offering to build an
independent sibling generator (`Infrastructure-Creator-[Stack]/`) rather than
silently failing — see `Infrastructure-Creator/README.md#non-php-targets`.

### Native installer (`infra-engine install`)

Installs one of the four ready-made editions into a target project from
versioned inventories (`install/inventories/*.json`), refusing unsupported
collisions before writing anything. Always dry-run first
(`--dry-run --merge-existing`), resolve every `COLLISION`, then repeat without
`--dry-run`. Use Infrastructure-Creator instead for projects with substantial
custom architecture — see `install/README.md`.

## Contributing Conventions

- Only change a skill in the edition(s) where it actually applies — Laravel,
  Symfony, WordPress and PHP Core semantics must remain distinct.
- Evaluate a universal policy change in `PHP Core/` first, then adapt it to
  other frameworks' boundaries rather than copying it blindly.
- Within one edition, mirror supported skill/agent/command changes across
  `.claude/`, `.cursor/`, `.agents/`, and `.codex/` (via `infra-engine repo mirrors --write`,
  not by hand).
- Record edition-local changes in that edition's own `CHANGELOG.md`. Changes
  to shared core (memory/context core, Project Brain, hooks, mirror machinery,
  root `scripts/`) go in the root `CHANGELOG.md` instead — CI's `changelog`
  job enforces this on PRs via `infra-engine repo changelog`.
- Run the affected edition's `DOD.md` verification after a change.
- Memory retrieval takes fixes. A ranking, admission, budget or capsule
  heuristic is merged only on its own task-outcome measurement, sealed
  before it runs on held-out tasks, never on stand numbers; see
  `docs/MEMORY-EVAL.md` ("Accepting a retrieval change").
