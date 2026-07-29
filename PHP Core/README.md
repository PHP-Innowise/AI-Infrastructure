# Accelerator Core PHP

> **For enforceable agent policy rules, see [AGENTS.md](AGENTS.md).**

A native-PHP accelerator framework for AI coding agents. It provides structured slash-command workflows, isolated agents, reusable skills, quality gates, and documentation conventions for PHP teams — usable from **Claude Code**, **Cursor**, and **OpenAI Codex** out of the same repository.

This is the `PHP Core/` folder of the `accelerator-php` monorepo: the universal, framework-agnostic base (Composer + PSR). Framework-specific behavior (Laravel, Symfony, etc.) belongs in the sibling `Laravel/` and `Symfony/` folders, not here — see the [repository root README](../README.md) for the full comparison and usage instructions.

## What This Is

Accelerator Core PHP is not a generated PHP application. It is a team workflow layer for AI agents:

- Commands route user intent to the right agent.
- Agents run one skill in isolated context, then stop.
- Skills define reliable workflows, examples, checklists, and output formats.
- Hooks and policy files enforce naming, safety, and verification conventions.
- `tasks/` stores temporary task documents; `specs/` stores living project specifications.
- `project-brain/` stores governed shared tasks, handoffs, records, manifests, and promotion proposals.
- `memory-bank/` stores small indexed chunks of reviewed reusable context shared across AI tools.

## Multi-Tool Editions

The same accelerator is mirrored for three agents. Each tool reads its own directory, so they coexist without conflict; the root `AGENTS.md` is the shared policy for all of them.

| Tool | Reads | Notes |
| --- | --- | --- |
| **Claude Code** | `.claude/` (agents, commands, hooks, skills, `settings.json`) | Original edition. |
| **Cursor** | `.cursor/` (agents, commands, `hooks.json`, `rules/*.mdc`, skills) | Self-contained; keep Cursor's "read `.claude`" setting **off** to avoid double-loading. See `.cursor/README.md`. |
| **Codex** | `.agents/skills/` + `.codex/` (`config.toml`, `hooks.json`) | Skills live in `.agents/skills`; no command layer (Codex deprecated custom prompts in favor of skills). See `.codex/README.md`. |

When you change a skill, mirror the edit across the editions you support (or regenerate).

## Directory Structure

```
AGENTS.md                # Shared, enforceable policy (all tools)

.claude/                 # Claude Code edition
├── agents/              # Agent wrappers that execute one skill and stop
├── commands/            # Slash commands invoked by users
├── hooks/               # Shell checks triggered by Claude Code events
├── skills/              # Skill implementations and references
├── DOD.md               # Native PHP Definition of Done
├── GOLDEN-PRINCIPLES.md # Engineering principles for reviews
├── STABILIZATION.md     # Error-to-rule process
└── settings.json        # Permissions and hook wiring

.cursor/                 # Cursor edition (skills, agents, commands, rules/*.mdc, hooks.json, docs)
.agents/skills/          # Codex skills (shared .agents convention)
.codex/                  # Codex config.toml, hooks.json, hooks/, docs

Task/                    # Product/domain planning material and design references
tasks/                   # Temporary task documentation
specs/                   # Permanent living specifications
memory-bank/             # Indexed durable cross-session project memory
project-brain/           # Shared governed task and control records
examples/                # Workflow output examples
```

## Architecture: Command -> Agent -> Skill

Each command starts a focused agent, that agent executes one skill, and then it stops with a context summary and suggested next steps. The main conversation stays clean and the user controls the workflow. (In Codex there is no command layer — you invoke a skill by name and Codex can also trigger it implicitly.)

```
User runs: /requirements-analyst [prompt]
              |
              v
      Command selects agent
              |
              v
      Agent executes one skill
              |
              v
      Output: result + context summary + next steps
```

## Native PHP Stack

The guidance assumes plain, framework-agnostic PHP. Recommended baseline:

- PHP 8.2+
- Composer 2+ with PSR-4 autoloading
- PSR-1 / PSR-12 / PER coding style; `declare(strict_types=1)`
- PHPUnit or Pest for tests
- PHP-CS-Fixer or PHP_CodeSniffer for formatting
- PHPStan or Psalm for static analysis
- Rector (optional) for automated refactors/upgrades
- PDO (or a documented data layer) with prepared statements for persistence

The accelerator does not force a heavy layered architecture. It encourages the simplest structure that fits — front controller, handlers, use-case/service classes, domain objects, data-access gateways — and adds interfaces, DTOs, or repositories only when they reduce real complexity.

## Prerequisites

```bash
php -v
composer --version
```

Install PHP and Composer via your OS package manager, Docker, or your team-standard PHP runtime. For an existing project:

```bash
composer install
```

## Quick Start

Use slash commands (Claude Code / Cursor) to move through the workflow:

| Command | Purpose |
| --- | --- |
| `/requirements-analyst` | Clarify and decompose requirements |
| `/brainstorm` | Explore solution options |
| `/researcher` | Evaluate libraries and compare approaches |
| `/council` | Weigh high-stakes decisions from multiple expert views |
| `/architect` | Make native PHP architecture decisions |
| `/api-designer` | Design REST APIs, DTOs, serializers, and OpenAPI docs |
| `/database-designer` | Design schemas, keys, indexing, and migrations |
| `/writing-plans` | Create implementation plans |
| `/architecture-implementer` | Scaffold an approved architecture (skeletons, DI, PSR-4) |
| `/coder` | Implement native PHP backend features |
| `/coder-frontend` | Implement server-rendered frontend work |
| `/refactorer` | Behavior-preserving refactors and PHP upgrades |
| `/test-generator` | Add PHPUnit/Pest tests |
| `/code-reviewer` | Review code for correctness, maintainability, and risk |
| `/security-reviewer` | Audit changes against the OWASP Top 10 |
| `/performance-optimization` | Diagnose and fix performance problems |
| `/dependency-manager` | Audit and manage Composer dependencies |
| `/systematic-debugger` (`/debugger`) | Find root cause before fixing bugs |
| `/project-brain` | Govern shared tasks, handoffs, unified retrieval, all six record types, compaction, and promotion proposals |
| `/memory-bank` | Retrieve, capture, audit, supersede/archive durable memory, or apply an approved promotion |
| `/verify` | Run the native PHP Definition of Done |
| `/review-pr` | Review a GitHub pull request |
| `/finishing-branch` | Prepare branch completion or PR |
| `/release` | Prepare release notes and changelog |

Example:

```text
/requirements-analyst Add invitation-only registration for trainers and players

[Agent returns requirements and context summary]

/architect Based on TASK-001, design the module boundaries and authorization model
```

## Documentation System

### Temporary Task Docs

Temporary task documents live in `tasks/TASK-N/`.

- Created by: requirements analysis, brainstorming, and implementation planning.
- Naming: files must be prefixed with the skill name, e.g. `requirements-analyst-requirements.md`.
- Lifecycle: delete or archive after implementation is complete.

### Living Specifications

Living specifications live in `specs/`.

- Entry point: `specs/MANIFEST.md`.
- Files: architecture, API, frontend, and implementation specifications.
- Updates: append new sections with `[TASK-N]` prefixes.
- Lifecycle: permanent project knowledge.

## Native PHP Implementation Philosophy

Prefer explicit, minimal structure over framework imitation:

- HTTP boundary: front controller, routing, handlers/controllers, PSR-15 middleware.
- Validation: typed DTOs / value objects / explicit validators at input boundaries.
- Authorization: an explicit access-control layer, never hidden UI.
- Persistence: PDO with prepared statements; repositories/gateways in the infrastructure layer.
- Output: serializers/DTOs for stable API response shapes.
- Business logic: use-case/service classes when handler code becomes unclear.
- Async work: explicit workers/queues with typed payloads and retry behavior.
- Integration boundaries: typed clients with timeouts, retries, and error mapping for external APIs.

## Project Brain And Memory Bank

Use `memory` in Codex or `/memory` in Claude/Cursor for an argument-free, authority-aware refresh: governed mode validates Project Brain and rebuilds the disposable source index without creating a second task record. Use `checkpoint` or `/checkpoint` for progress capture; it defers to revision-checked Project Brain updates in governed mode and writes local Working Memory only when lightweight mode is explicitly configured. Neither command completes a task or applies a promotion.

Governed Project Brain mode is the default for non-trivial work. `project-brain/` is the shared authority for active tasks, handoffs, findings, bugs, incidents, decisions, events, retrieval manifests, conflicts, and promotion proposals. The ignored SQLite database is only a disposable index plus local binding/cache in this mode.

Use the one public task-aware retrieval command:

```bash
python3 memory-bank/scripts/context.py retrieve QUERY --task-id ID
```

Canonical policy, specs, current code, configuration, migrations, and tests always outrank Project Brain, memory, retrieval packets, and local indexes. Retrieval and indexing are explicit; the accelerator does not inject context into prompts automatically. Use `--mode lightweight` only explicitly for machine-local work that does not need shared continuity or governed records.

The root `memory-bank/` remains the canonical durable store for reviewed reusable constraints, conventions, decisions, domain knowledge, integrations, and operational lessons. Use `memory-bank` directly in Codex or `/memory-bank` in Claude/Cursor for durable retrieval/capture/audit/supersession and approved promotion application. Use `project-brain` directly in Codex or `/project-brain` in Claude/Cursor for active work and proposals. Codex intentionally has no fake command/agent wrapper.

Agents may propose promotion but cannot self-approve it. Application requires explicit human review and records source and destination revisions. Secrets, transcripts, raw logs, personal/customer data, and unredacted incident payloads are prohibited. Session hooks report only mode, index health/staleness, active binding count, and validation status; they never index, retrieve, print, or inject records.

See the repository-level [user task workflow example](../USER-TASK-WORKFLOW-EXAMPLE.md) for the complete prompt-to-archive flow without duplicating it here.

## Verification

Before claiming completion, agents run applicable checks from `DOD.md` (per edition: `.claude/DOD.md`, `.cursor/DOD.md`, `.codex/DOD.md`). Missing tooling is reported as `N/A - tooling not configured`; it is not installed silently. Prefer the project's Composer scripts so local and CI use the same entry points:

```bash
composer validate --strict
composer test        # or vendor/bin/phpunit / vendor/bin/pest
composer lint        # or vendor/bin/php-cs-fixer fix --dry-run --diff / vendor/bin/phpcs
composer analyse     # or vendor/bin/phpstan analyse / vendor/bin/psalm
php -l path/to/File.php
```

## Sharing With a Team

1. Commit the edition(s) your team uses: `.claude/`, `.cursor/`, and/or `.agents/` + `.codex/`, plus the shared root `AGENTS.md`.
2. Keep personal overrides uncommitted (e.g. `.claude/settings.local.json`).
3. Agree on project-level PHP tooling: PHPUnit or Pest, PHP-CS-Fixer or PHP_CodeSniffer, PHPStan or Psalm.
4. Keep specs current as features evolve.
5. Treat generated task docs as temporary working material, not permanent architecture records.
6. Cursor: keep the "read `.claude` files" setting off. Codex: trust the project so `.codex/` config and hooks load.
