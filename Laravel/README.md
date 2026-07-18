# Accelerator Core PHP

> **For enforceable agent policy rules, see [AGENTS.md](AGENTS.md).**

A Laravel-first accelerator framework for AI coding agents. It provides structured slash-command workflows, isolated agents, reusable skills, quality gates, and documentation conventions for PHP teams building Laravel applications — usable from **Claude Code**, **Cursor**, and **OpenAI Codex** out of the same repository.

This is the `feature/laravel-accelerator` branch: it specializes the accelerator for Laravel. The framework-agnostic native-PHP base lives on `main`; other frameworks get their own dedicated branches.

## What This Is

Accelerator Core PHP is not a generated Laravel application. It is a team workflow layer for AI agents:

- Commands route user intent to the right agent.
- Agents run one skill in isolated context, then stop.
- Skills define reliable workflows, examples, checklists, and output formats.
- Hooks and policy files enforce naming, safety, and verification conventions.
- `tasks/` stores temporary task documents; `specs/` stores living project specifications.

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
├── DOD.md               # Laravel Definition of Done
├── GOLDEN-PRINCIPLES.md # Engineering principles for reviews
├── STABILIZATION.md     # Error-to-rule process
└── settings.json        # Permissions and hook wiring

.cursor/                 # Cursor edition (skills, agents, commands, rules/*.mdc, hooks.json, docs)
.agents/skills/          # Codex skills (shared .agents convention)
.codex/                  # Codex config.toml, hooks.json, hooks/, docs

Task/                    # Product/domain planning material and design references
tasks/                   # Temporary task documentation
specs/                   # Permanent living specifications
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

## Laravel Default Stack

The PHP guidance in this accelerator assumes Laravel as the default backend framework.

Recommended baseline:

- PHP 8.2+ (8.3+ required for Laravel 13)
- Composer 2+
- Laravel 12 or 13
- PHPUnit or Pest (Pest 4+ for built-in Playwright browser testing)
- Laravel Pint
- PHPStan with Larastan, or Psalm if the project already uses Psalm
- Laravel Sanctum for first-party API/session auth when appropriate; Passport for third-party OAuth2 clients
- Eloquent, migrations, factories, seeders, Policies, Jobs, Events, queues, and API Resources
- `laravel/boost` (dev dependency) so AI coding agents get live access to routes, schema, config, and version-pinned docs

This accelerator does not force every project into a heavy layered architecture. It encourages Laravel-native structure first, then adds Actions, Services, DTOs, or domain modules only when they reduce real complexity.

## Prerequisites

### PHP and Composer

```bash
php -v
composer --version
```

Install PHP and Composer using your operating system package manager, Laravel Herd, Docker, or your team-standard PHP runtime.

### Laravel Tooling

For an existing Laravel app, install dependencies from the Laravel project directory:

```bash
composer install
```

Common verification tools:

```bash
php artisan test
vendor/bin/pint --test
vendor/bin/phpstan analyse
composer validate --strict
```

If the project uses Pest:

```bash
vendor/bin/pest
```

### Frontend Tooling

Laravel projects typically use Blade, Livewire, or Inertia (Vue/React/Svelte) — see `frontend-design`/`coder-frontend` for choosing between them. Vite is the standard asset bundler:

```bash
npm install
npm run dev     # local
npm run build   # production
```

## Quick Start

Use slash commands to move through the workflow:

| Command | Purpose |
| --- | --- |
| `/requirements-analyst` | Clarify and decompose requirements |
| `/brainstorm` | Explore solution options |
| `/researcher` | Evaluate packages and compare approaches |
| `/council` | Weigh high-stakes decisions from multiple expert views |
| `/architect` | Make Laravel architecture decisions |
| `/api-designer` | Design REST APIs, Form Requests, API Resources, and OpenAPI docs |
| `/database-designer` | Design schemas, Eloquent relationships, and migrations |
| `/writing-plans` | Create implementation plans |
| `/architecture-implementer` | Scaffold an approved architecture via Artisan generators |
| `/coder` | Implement Laravel backend features |
| `/coder-frontend` | Implement Blade/Livewire/Inertia frontend work |
| `/filament` | Build Filament admin panels: Resources, Schemas, Tables, Relation Managers, Widgets |
| `/eloquent` | Deep Eloquent model-layer patterns: polymorphic relations, casts, scopes, Observers |
| `/queues-jobs` | Design and implement queued Jobs, job middleware, batching, chaining, Horizon |
| `/events-notifications` | Implement Events, Listeners, Observers, Notifications, and Mailables |
| `/auth-scaffolding` | Set up web/session auth starter kits, multi-guard config, Policy/Gate patterns |
| `/caching` | Design and implement a caching strategy: stampede prevention, tagging, invalidation |
| `/console-scheduler` | Build custom Artisan commands and schedule recurring tasks |
| `/file-storage` | Implement file storage/uploads: disk config, secure uploads, signed URLs |
| `/package-developer` | Build and maintain a reusable Composer/Laravel package |
| `/refactorer` | Behavior-preserving refactors and Laravel version upgrades |
| `/test-generator` | Add PHPUnit/Pest tests |
| `/code-reviewer` | Review code for correctness, maintainability, and risk |
| `/security-reviewer` | Audit changes against the OWASP Top 10 |
| `/performance-optimization` | Diagnose and fix performance problems (N+1 queries, caching) |
| `/dependency-manager` | Audit and manage Composer/Laravel packages |
| `/debugger` | Find root cause before fixing bugs |
| `/verify` | Run the Laravel Definition of Done |
| `/review-pr` | Review a GitHub pull request |
| `/finishing-branch` | Prepare branch completion or PR |
| `/release` | Prepare release notes and changelog |

Example:

```text
/requirements-analyst Add invitation-only registration for trainers and players

[Agent returns requirements and context summary]

/architect Based on TASK-001, design the Laravel module boundaries and authorization model
```

## Documentation System

### Temporary Task Docs

Temporary task documents live in `tasks/TASK-N/`.

- Created by: requirements analysis, brainstorming, and implementation planning.
- Naming: files must be prefixed with the skill name, for example `requirements-analyst-requirements.md`.
- Lifecycle: delete or archive after the implementation is complete.

### Living Specifications

Living specifications live in `specs/`.

- Entry point: `specs/MANIFEST.md`.
- Files: architecture, API, frontend, and implementation specifications.
- Updates: append new sections with `[TASK-N]` prefixes.
- Lifecycle: permanent project knowledge.

## Laravel Implementation Philosophy

Use Laravel conventions before inventing abstractions:

- HTTP boundary: routes, controllers, Form Requests, middleware.
- Validation: Form Requests or explicit validators at input boundaries.
- Authorization: Policies and Gates.
- Persistence: Eloquent models, migrations, factories, seeders.
- Output: API Resources for stable response shapes.
- Business logic: Actions or Services when controller/model code becomes unclear.
- Async work: Jobs, queues, Events, Listeners, Notifications, and Mailables.
- Integration boundaries: typed clients or dedicated services for external APIs.

Avoid copying patterns from other ecosystems unless they solve a clear Laravel problem.

## Verification

Before claiming completion, agents must run applicable checks from the active edition's `DOD.md` (`.claude/DOD.md`, `.cursor/DOD.md`, or `.codex/DOD.md`). Missing tooling is reported as `N/A - tooling not configured`; it is not installed silently.

Typical Laravel verification:

```bash
composer validate --strict
php artisan test
vendor/bin/pint --test
vendor/bin/phpstan analyse
php artisan route:list
```

## Sharing With a Team

1. Commit the edition(s) your team uses: `.claude/`, `.cursor/`, and/or `.agents/` + `.codex/`, plus the shared root `AGENTS.md`.
2. Keep personal overrides uncommitted (e.g. `.claude/settings.local.json`).
3. Agree on project-level PHP tooling: PHPUnit or Pest, Pint, PHPStan/Larastan or Psalm.
4. Keep specs current as features evolve.
5. Treat generated task docs as temporary working material, not permanent architecture records.
6. Cursor: keep the "read `.claude` files" setting off. Codex: trust the project so `.codex/` config and hooks load.

## Adaptation Notes

This branch specializes the universal `main` base for Laravel:

- Top-level onboarding and policy docs rewritten for Laravel accuracy.
- Definition of Done, Golden Principles, and Stabilization examples rewritten around Laravel conventions (Form Requests, Policies, Eloquent, Artisan).
- Claude/Cursor/Codex settings and allowed shell commands updated for `artisan`, `pint`, and the Laravel package ecosystem.
- Backend, API, database, testing, security, performance, frontend (Blade/Livewire/Inertia), and documentation skills rewritten for Laravel.
- Command and agent descriptions updated to assume a Laravel backend.
