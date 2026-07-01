# Accelerator Core PHP

> **For enforceable agent policy rules, see [AGENTS.md](AGENTS.md).**

A Laravel-first accelerator framework for Claude Code. It provides structured slash-command workflows, isolated agents, reusable skills, quality gates, and documentation conventions for PHP teams building Laravel applications.

This repository is a PHP adaptation. Universal workflow assets were copied where they are framework-neutral; Laravel-specific guidance was written intentionally rather than converted by word replacement.

## What This Is

Accelerator Core PHP is not a generated Laravel application. It is a team workflow layer for Claude Code:

- Commands route user intent to the right agent.
- Agents run one skill in isolated context.
- Skills define reliable workflows, examples, checklists, and output formats.
- Hooks and policy files enforce naming, safety, and verification conventions.
- `tasks/` stores temporary task documents.
- `specs/` stores living project specifications.

## Directory Structure

```
.claude/
├── agents/              # Agent wrappers that execute one skill and stop
├── commands/            # Slash commands invoked by users
├── hooks/               # Shell checks triggered by Claude Code events
├── skills/              # Skill implementations and references
├── DOD.md               # Laravel/PHP Definition of Done
├── GOLDEN-PRINCIPLES.md # Engineering principles for reviews
└── settings.json        # Team configuration for permissions and hooks

Task/
├── Epics/               # Product/domain planning material
└── designs/             # Shared visual design references

tasks/                   # Temporary task documentation
specs/                   # Permanent living specifications
examples/                # Workflow output examples
```

## Architecture: Command -> Agent -> Skill

The accelerator uses a manual flow. Each command starts a focused agent, that agent executes one skill, and then it stops with a context summary and suggested next steps. The main conversation stays clean and the user controls the workflow.

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

- PHP 8.2+
- Composer 2+
- Laravel 11 or 12
- PHPUnit or Pest
- Laravel Pint
- PHPStan with Larastan, or Psalm if the project already uses Psalm
- Laravel Sanctum for first-party API auth when appropriate
- Eloquent, migrations, factories, seeders, policies, jobs, events, queues, and API resources

This accelerator does not force every project into a heavy layered architecture. It encourages Laravel-native structure first, then adds services, actions, repositories, DTOs, or domain modules only when they reduce real complexity.

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
composer validate
```

If the project uses Pest:

```bash
vendor/bin/pest
```

### Optional Frontend Tooling

Laravel projects may use Blade, Livewire, Inertia, or an API-only backend with a separate frontend. If a project includes Vite or other JavaScript tooling, run frontend commands only in the project that owns that tooling and only when the team has explicitly configured those commands.

## Quick Start

Use slash commands to move through the workflow:

| Command | Purpose |
| --- | --- |
| `/requirements-analyst` | Clarify and decompose requirements |
| `/brainstorm` | Explore solution options |
| `/architect` | Make Laravel architecture decisions |
| `/api-designer` | Design REST APIs, requests, resources, and OpenAPI docs |
| `/writing-plans` | Create implementation plans |
| `/coder` | Implement Laravel backend features |
| `/coder-frontend` | Implement frontend work when the project has a frontend |
| `/test-generator` | Add PHPUnit/Pest tests |
| `/code-reviewer` | Review code for correctness, maintainability, and risk |
| `/verify` | Run the Laravel/PHP Definition of Done |
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

- HTTP boundary: routes, controllers, form requests, middleware.
- Validation: form requests or explicit validators at input boundaries.
- Authorization: policies and gates.
- Persistence: Eloquent models, migrations, factories, seeders.
- Output: API resources for stable response shapes.
- Business logic: services or actions when controller/model code becomes unclear.
- Async work: jobs, queues, events, listeners, notifications, and mailables.
- Integration boundaries: typed clients or dedicated services for external APIs.

Avoid copying patterns from other ecosystems unless they solve a clear Laravel problem.

## Verification

Before claiming completion, agents must run applicable checks from `.claude/DOD.md`. Missing tooling is reported as `N/A - tooling not configured`; it is not installed silently.

Typical Laravel verification:

```bash
composer validate
php artisan test
vendor/bin/pint --test
vendor/bin/phpstan analyse
php artisan route:list
```

## Sharing With a Team

Recommended team setup:

1. Keep `.claude/settings.json`, `.claude/commands`, `.claude/agents`, `.claude/skills`, and `.claude/hooks` committed.
2. Keep `.claude/settings.local.json` uncommitted for personal overrides.
3. Agree on project-level PHP tooling: PHPUnit or Pest, Pint, PHPStan/Larastan or Psalm.
4. Keep specs current as features evolve.
5. Treat generated task docs as temporary working material, not permanent architecture records.

## Adaptation Notes

The following were intentionally rewritten for PHP/Laravel accuracy:

- Top-level onboarding and policy docs.
- Definition of Done.
- Claude settings and allowed shell commands.
- Backend implementation, architecture, API design, testing, verification, and documentation skills.
- Command and agent descriptions that previously assumed a non-PHP backend.

