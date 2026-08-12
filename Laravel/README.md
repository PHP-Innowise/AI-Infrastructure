# Laravel AI Accelerator

> **For enforceable agent policy rules, see [AGENTS.md](AGENTS.md).**

A Laravel-first accelerator framework for AI coding agents. It provides structured slash-command workflows, isolated agents, reusable skills, quality gates, and documentation conventions for PHP teams building Laravel applications — usable from **Claude Code**, **Cursor**, and **OpenAI Codex** out of the same repository.

This is the `Laravel/` folder of the `accelerator-php` monorepo: it specializes the accelerator for Laravel. The framework-agnostic native-PHP base lives in the sibling `PHP Core/` folder; other frameworks (Symfony, etc.) get their own sibling folder — see the [repository root README](https://github.com/PHP-Innowise/AI-Infrastructure/blob/main/README.md) for the full comparison and usage instructions.

## What This Is

Accelerator Core PHP is not a generated Laravel application. It is a team workflow layer for AI agents:

- Commands route user intent to the right agent.
- Agents run one skill in isolated context, then stop.
- Skills define reliable workflows, examples, checklists, and output formats.
- Hooks and policy files enforce naming, safety, and verification conventions.
- `tasks/` stores temporary task documents; `specs/` stores living project specifications.
- `project-brain/` stores governed shared tasks, handoffs, records, manifests, and promotion proposals.
- `memory-bank/` stores small indexed chunks of verified reusable context shared across AI tools.

## Multi-Tool Editions

The same accelerator is mirrored for three agents. Each tool reads its own directory, so they coexist without conflict; the root `AGENTS.md` is the shared policy for all of them.

| Tool | Reads | Notes |
| --- | --- | --- |
| **Claude Code** | `.claude/` (agents, commands, hooks, skills, `settings.json`) | Original edition. |
| **Cursor** | `.cursor/` (agents, commands, `hooks.json`, `rules/*.mdc`, skills) | Self-contained; keep Cursor's "read `.claude`" setting **off** to avoid double-loading. See `.cursor/README.md`. |
| **Codex** | `.agents/skills/` + `.codex/` (`config.toml`, `hooks.json`) | Skills live in `.agents/skills`; no command layer (Codex deprecated custom prompts in favor of skills). See `.codex/README.md`. |

When you change a skill, mirror the edit across the editions you support (or regenerate).

## Source Edition Structure

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

Task/                    # Source-only sample/client material; not installed
tasks/                   # Installed temporary-task scaffold
specs/                   # Permanent living specifications
memory-bank/             # Indexed durable cross-session project memory
project-brain/           # Shared governed task and control records
examples/                # Source-only worked examples
```

## Installed Production Payload

The source edition is intentionally broader than the ready-made installation.
Production inventories keep the runtime needed by a consuming Laravel project:
`AGENTS.md`, selected native tool integrations, policies, hooks, skills,
workflow documentation, templates, `memory-bank/`, `project-brain/`, `specs/`,
and the lowercase `tasks/` operational scaffold.

Source-only research, tests, worked examples, and this repository's bundled
uppercase `Task/` product/design material remain available to maintainers but
are not copied into client projects. Uppercase `Task/` is optional client-input
space: create and populate it only when the consuming project actually has
requirements or design assets to place there. It is distinct from lowercase
`tasks/`, which remains available for temporary, skill-prefixed `TASK-NNN/`
artifacts.

The versioned inventory resolves these exclusions and any production
overrides. Use the inventory verifier and installer dry run to inspect the
actual payload; do not derive or freeze an exact file count from this source
tree.

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
| `/flow-feature` | Orchestrate a complete feature with planning, approval, implementation, review, and verification |
| `/flow-review` | Run parallel code, security, and performance review and synthesize one report |
| `/sdd` | Run resumable spec-driven development with durable specs, tasks, and checkpoints |
| `/codebase-mapper` | Map an unfamiliar Laravel codebase into source-cited, commit-stamped `codebase/` documents |
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
| `/project-brain` | Govern shared tasks, handoffs, unified retrieval, all six record types, compaction, and promotion proposals |
| `/memory-bank` | Retrieve, capture, audit, supersede/archive durable memory, or apply a governed automatic/independently reviewed promotion |
| `/verify` | Run the Laravel Definition of Done |
| `/review-pr` | Review a GitHub pull request |
| `/finishing-branch` | Prepare branch completion or PR |
| `/release` | Prepare release notes and changelog |

See [Orchestrator Commands](https://github.com/PHP-Innowise/AI-Infrastructure/blob/main/docs/ORCHESTRATOR-COMMANDS.md) for flow examples,
approval points, parallel-review limits, and write serialization.

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

## Project Brain And Memory Bank

Use `memory` in Codex or `/memory` in Claude/Cursor for an argument-free, authority-aware refresh: governed mode validates Project Brain and rebuilds the disposable source index without creating a second task record. Use `checkpoint` or `/checkpoint` for progress capture; it defers to revision-checked Project Brain updates in governed mode and writes local Working Memory only when lightweight mode is explicitly configured. Neither command completes a task or applies a promotion.

Governed Project Brain mode is the default for non-trivial work. `project-brain/` is the shared authority for active tasks, handoffs, findings, bugs, incidents, decisions, events, retrieval manifests, conflicts, and promotion proposals. The ignored SQLite database is only a disposable index plus local binding/cache in this mode.

Use the one public task-aware retrieval command:

```bash
python3 memory-bank/scripts/context.py retrieve QUERY --task-id ID
```

Canonical policy, specs, current code, configuration, migrations, and tests always outrank Project Brain, memory, retrieval packets, and local indexes. Automatic delivery is bounded: Claude Code and Codex retrieve a fresh Task Capsule at prompt time, while Cursor uses an `alwaysApply` rule rendered at the previous turn boundary. Explicit `context.py retrieve` remains available for every tool. Use `--mode lightweight` only explicitly for machine-local work that does not need shared continuity or governed records.

### Task Capsule

At the start of a complex request and before a complex phase handoff, the agent
derives a concise sanitized retrieval query and builds a Task Capsule from
optional Working Memory and `context` retrieval. The raw request is not copied
into the packet. The complete packet is capped at 8,000 Unicode characters and
contains at most two Procedural, three Semantic, and one Episodic result.
Retrieved entries are short snippets with source paths; the next agent reads a
full source only when its current step requires it.

Simple tasks stay in the current context. Fresh contexts are reserved for
research-to-planning, planning-to-implementation,
implementation-to-independent-verification, and recovery after compaction.
`memory`, `checkpoint`, and explicit `complete` keep their existing roles.

New chunks use conflict-free names such as `memory-bank/chunks/MEM-YYYYMMDD-xxxxxxxx-short-slug.md`; legacy `MEM-NNNN` chunks keep their IDs. Rebuild the derived `INDEX.md` with `python3 memory-bank/scripts/context.py reindex-bank` instead of editing a shared counter. Automatic promotions are tagged `auto-promoted` and are explicitly unreviewed; disable `automatic_promotion` for the independent-review workflow. Session-start banners remain metadata-only even though separate prompt/turn hooks deliver bounded working context.

## Optional MCP Integrations

MCP servers are optional external integrations. This accelerator requires
none. Enable only the servers that correspond to systems this project actually
uses: a small, relevant toolset consumes less context and creates a smaller
security boundary than installing every available server.

Commonly useful for a PHP project:

- [Context7](https://github.com/upstash/context7) — current, version-specific
  framework and package documentation. Most valuable when the installed
  framework or library version differs from the model's built-in knowledge,
  which is the one gap no repository-local index can close.
- [GitHub MCP Server](https://github.com/github/github-mcp-server) —
  repository, pull-request, issue, and workflow context. Prefer the official
  server with repository-scoped, least-privilege access.
- [Sentry](https://mcp.sentry.dev/) or
  [Datadog](https://docs.datadoghq.com/mcp_server/) — production errors,
  traces, and incident evidence. Pick the platform this project already uses;
  do not connect both without a concrete need.
- [Playwright MCP](https://github.com/microsoft/playwright-mcp) — browser
  interaction and UI-flow verification. Only for a browser-facing surface.
- [Linear](https://linear.app/docs/mcp) or
  [Atlassian Rovo](https://github.com/atlassian/atlassian-mcp-server) —
  requirements and issue context. Prefer read-only access.
- A database-specific MCP server — schema inspection and query diagnostics
  when repository evidence is insufficient. Use a dedicated read-only account
  and never point one at an unrestricted production database by default.

### Security rules

1. Prefer first-party servers and official documentation.
2. Start with read-only scopes, the smallest toolset, and one project or
   organization boundary.
3. Keep tokens, connection strings, and credentials outside the repository.
4. Require human confirmation for writes, deployments, issue transitions, and
   other consequential actions.
5. Treat MCP output as external evidence: verify important claims against
   canonical project sources before changing code.
6. Do not add generic filesystem or memory MCP servers merely to duplicate the
   repository access, Memory Bank, Project Brain, or Local Context Engine this
   accelerator already supplies.

### Context rules

Tool definitions are not free and are not paid once. They sit at the front of
the model's context and are re-read on every turn of the session, so a server
you never call still costs you on every prompt.

7. Scope every server to the smallest toolset it needs. `github-mcp-server`
   defaults to five toolsets (`context, repos, issues, pull_requests, users`)
   and `--toolsets all` is substantially larger; do not use `all`. Run with
   `--read-only` (`GITHUB_READ_ONLY=1`) unless a workflow needs writes — which
   also satisfies rule 4. Run `playwright-mcp` without `--caps` unless a
   capability is genuinely required. A server left at its widest setting can
   occupy several times the context of this accelerator's own `AGENTS.md` and
   all of its skill descriptions combined.
8. Decide the server set before starting a session. Tool definitions are the
   first tier of the prompt cache, ahead of the system prompt and the
   conversation, so adding or removing a server mid-session invalidates that
   cache and the accumulated context is re-established at full price.
9. What a server returns usually costs more than what it declares, because a
   large result stays in context and is re-read for the rest of the session.
   Prefer bounded, structured output, and constrain at the call site anything
   that can return a page, a log stream, or a query result set. Where an API
   exposes no size parameter, the only lever is a narrower request.

Verify rather than estimate: `/context` reports the MCP tools row for the
current session.

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

This folder specializes the universal `PHP Core/` base for Laravel:

- Top-level onboarding and policy docs rewritten for Laravel accuracy.
- Definition of Done, Golden Principles, and Stabilization examples rewritten around Laravel conventions (Form Requests, Policies, Eloquent, Artisan).
- Claude/Cursor/Codex settings and allowed shell commands updated for `artisan`, `pint`, and the Laravel package ecosystem.
- Backend, API, database, testing, security, performance, frontend (Blade/Livewire/Inertia), and documentation skills rewritten for Laravel.
- Command and agent descriptions updated to assume a Laravel backend.
