# Symfony Layered Architecture Accelerator

> **Enforceable agent policy lives in [AGENTS.md](AGENTS.md).**

A Symfony-first workflow accelerator for AI coding agents. It provides focused commands, single-skill agents, reusable engineering workflows, safety hooks, quality gates, and documentation conventions for teams building maintainable Symfony applications with a pragmatic default:

```text
Controller -> Service -> Repository
```

This is the `Symfony/` folder of the `accelerator-php` monorepo. The framework-neutral PHP base lives in the sibling `PHP Core/` folder; `Laravel/` is the equivalent Laravel specialization — see the [repository root README](https://github.com/PHP-Innowise/AI-Infrastructure/blob/main/README.md) for the full comparison and usage instructions.

## What This Is

This repository is not a generated Symfony application. It is an engineering workflow layer that can be placed alongside an existing project:

- Commands route intent to a focused workflow.
- Agent wrappers execute one skill, return context, and stop.
- Skills define repeatable design, implementation, review, debugging, and delivery practices.
- Hooks enforce skill-prefixed task/spec names, zero-padded task directories, Git safety, database safety, and workflow constraints.
- `tasks/` stores temporary task artifacts; `specs/` stores durable architecture knowledge.
- `project-brain/` stores governed shared tasks, handoffs, records, manifests, and promotion proposals.
- `memory-bank/` stores small indexed chunks of reviewed reusable context shared across AI tools.

## Supported Baseline

| Symfony | PHP | Position |
| --- | --- | --- |
| **7.4 LTS** | 8.2+ | Long-term support baseline for production projects |
| **8.1** | 8.4+ | Current stable feature line as of July 2026 |

Every workflow must inspect the consuming project's `composer.json`, lock file, installed components, and conventions before recommending APIs. The accelerator does not silently upgrade Symfony or install optional bundles.

## Multi-Tool Editions

The root `AGENTS.md` policy is shared. Each tool keeps its native integration model:

| Tool | Reads | Native integration |
| --- | --- | --- |
| **Claude Code** | `.claude/` | Skills, commands, agent wrappers, settings, hooks, and engineering references |
| **Cursor** | `.cursor/` | Self-contained skills, commands, agents, rules, hooks, and references |
| **OpenAI Codex** | `.agents/skills/` + `.codex/` | Skills from `.agents/skills`; project config, hooks, and references from `.codex` |

Do not enable Cursor's optional Claude-file loading when using the self-contained `.cursor` edition. Codex does not use duplicated `.codex/skills`, command, or wrapper trees.

## Source Edition Structure

```text
AGENTS.md                 # Shared enforceable policy
README.md                 # Public Symfony accelerator guide
CHANGELOG.md              # Versioned accelerator changes

.claude/                  # Claude Code edition
├── agents/               # One-skill wrappers
├── commands/             # Slash-command entry points
├── hooks/                # Safety and workflow hooks
├── skills/               # Claude-native skill mirror
├── DOD.md
├── GOLDEN-PRINCIPLES.md
└── STABILIZATION.md

.cursor/                  # Cursor-native mirror and adapters
.agents/skills/           # Canonical shared skills; Codex discovery
.codex/                   # Codex config, hooks, and references
Task/                     # Source-only sample/client material; not installed
tasks/                    # Installed temporary-task scaffold
specs/                    # Permanent living specifications
memory-bank/              # Indexed durable cross-session project memory
project-brain/            # Shared governed task and control records
examples/                 # Runtime reference plus source-only examples
```

## Installed Production Payload

The source edition contains maintainer and validation material that does not
belong in a consuming Symfony application. The production inventory retains
the operational accelerator: shared policy, selected native tool integrations,
runtime scripts, hooks, skills, workflow documentation, templates,
`memory-bank/`, `project-brain/`, `specs/`, and the lowercase `tasks/`
scaffold.

Source-only research, test suites, source-only worked examples, and this
repository's bundled uppercase `Task/` product/design material are not
installed. The Symfony clean-code patterns remain because active workflow
documentation references them. A client project may create and populate
uppercase `Task/` when real requirements or design inputs exist. Lowercase
`tasks/` has a separate role: it remains the temporary, skill-prefixed
`TASK-NNN/` workspace used by accelerator workflows.

The versioned inventory is responsible for resolving source exclusions and
production-specific overrides. Verify it and review an installer dry run for
the actual payload rather than maintaining an exact file-count claim in this
README.

## Workflow Model

Claude Code and Cursor expose command -> agent -> skill routing. Codex invokes the matching skill directly from `.agents/skills`; use discovered names such as `brainstorming`, `systematic-debugger`, `documentation-generator`, and `using-git-worktrees` rather than Claude/Cursor command aliases.

```text
User request
    -> command or implicit skill selection
    -> one focused skill
    -> artifact or implementation
    -> Context Summary + Next Steps
    -> stop for user control
```

Skills must not silently chain into another workflow. This keeps requirements, architecture, implementation, review, and release decisions observable.

## Architecture Policy

Default placement in a conventional application:

```text
src/
├── Controller/          # Map input, authorize, call one service, map output
├── Service/             # Use cases, decisions, transactions, side effects
├── Repository/          # Doctrine queries, persistence, pagination, locking
├── Entity/              # Local invariants only
├── DTO/                 # Request, response, command, and result contracts
├── Form/                # Server-rendered form boundaries
├── Validator/           # Reusable constraints and validators
├── Security/Voter/      # Object/action authorization
├── Message/             # Immutable Messenger payloads
├── MessageHandler/      # Thin handlers delegating to services
├── EventSubscriber/     # Framework adapters
└── Command/             # Thin console entry points
```

The layer rule is pragmatic:

- Controllers, commands, handlers, subscribers, and UX components must not hide business workflows.
- Services own application orchestration and multi-write transaction boundaries.
- Repositories own QueryBuilder, DQL, SQL, hydration, and query-performance decisions.
- Entities protect local invariants without knowing HTTP, sessions, templates, queues, or mailers.
- Forms, request DTOs, Validator constraints, and explicit validation protect external input.
- Voters, attributes, firewalls, `access_control`, and route constraints enforce authorization.
- Public APIs use response DTOs, Serializer configuration, normalizers, documented contracts, or API Platform resources.
- Interfaces are added only for multiple implementations, external/package boundaries, or a useful narrow testing contract.

### Pragmatic SOLID

The accelerator applies SOLID through outcomes rather than class-count ceremony:

- one cohesive reason to change per controller adapter, use-case service, repository/query service, and infrastructure adapter;
- dependencies point inward from Symfony/framework code toward application behavior;
- narrow interfaces exist at real substitution, vendor, storage, time, or package boundaries;
- implementations preserve contract inputs, outputs, errors, side effects, and nullability;
- typed DTOs/value objects, composition, explicit transactions, and business-readable names are preferred over array contracts, service locators, boolean mode flags, global state, and speculative inheritance.

See [Symfony clean-code patterns](examples/symfony-clean-code-patterns.md) for paired bad/good examples covering controllers, services, Doctrine, validation, voters, Messenger, console, events, API Platform, Twig, and tests. The examples are illustrative; project versions, conventions, policy, specifications, and tests remain authoritative.

## Symfony Capability Coverage

### Backend And Data

- Controllers, services, repositories, DTOs, Forms, Validator, dependency injection, decorators, tags, and compiler passes.
- Doctrine entities, mappings, relationships, indexes, constraints, repositories, transactions, locking, migrations, backfills, and safe rollout.
- Console commands, events/subscribers, Scheduler or cron integration, cache, and configuration.

### APIs And Async Work

- REST routes, request mapping, validation, stable errors, pagination, idempotency, rate limits, Serializer, and OpenAPI.
- API Platform resources, operations, state providers/processors, filters, security, and documentation.
- Messenger transports, routing, retries, failure transports, idempotency, worker lifecycle, observability, and deployment compatibility.

### Security

- Firewalls, authenticators, password hashing/upgrades, login throttling, voters, role hierarchy, `access_control`, CSRF, sessions/cookies, remember-me behavior, token lifecycle, trusted proxies/hosts, security headers, and authorization tests.
- Safe Serializer exposure, parameterized Doctrine queries, upload validation, Twig escaping, constrained outbound HTTP, secret handling, and Composer advisories.
- OWASP Top 10 review with concrete exploit scenarios and ship/block findings.

### Frontend

The default server-rendered stack is Twig + Symfony Forms + Stimulus/Turbo, using AssetMapper where it fits. Existing Encore, Vite, SPA, or separate-frontend stacks are supported rather than replaced without justification.

Frontend workflows cover semantic HTML, accessible form errors, focus management, progressive enhancement, Turbo navigation/frames/streams, Stimulus controller lifecycle, Live Components when installed, loading/empty/error states, responsive behavior, WCAG 2.2, asset builds, and browser verification.

### Quality And Operations

- PHPUnit or Pest, KernelBrowser/WebTestCase, repository integration tests, voter/constraint tests, CommandTester, Messenger tests, fixtures, Foundry, object mothers, and deterministic builders.
- Symfony Profiler, Web Debug Toolbar, Monolog, Blackfire when available, Doctrine query profiling, explain plans, cache, Messenger throughput, memory, and OPcache.
- Composer/Flex recipe review, dependency audits, deprecations, upgrades, releases, changelogs, migrations, cache warmup, worker restart/drain, rollback limitations, and living documentation.
- Indexed cross-session memory with selective retrieval, source verification, review dates, supersession, privacy controls, and deterministic validation.
- Governed shared tasks, revision-safe handoffs and records, explicit conflicts, retrieval manifests, compaction, and clearly labeled automatic or independently reviewed promotion.

## Prerequisites

Check the project-provided tools before using them:

```bash
php -v
composer --version
php bin/console about
php bin/console debug:container
```

For an existing application, install dependencies through its documented workflow, typically:

```bash
composer install
```

Do not install Symfony CLI, bundles, npm packages, or analysis tools without approval. Symfony CLI is useful but optional; `php bin/console` remains the portable application entry point.

## Quick Start

| Skill / command | Purpose |
| --- | --- |
| `requirements-analyst` | Clarify requirements and create task-ready acceptance criteria |
| `brainstorming` | Compare solution approaches before implementation |
| `researcher` | Evaluate components, bundles, packages, and unfamiliar subsystems |
| `council` | Weigh high-impact architecture, security, or operational decisions |
| `architect` | Design Symfony boundaries and layered placement |
| `api-designer` | Design routes, DTOs, validation, errors, pagination, and OpenAPI |
| `api-platform-designer` | Design API Platform resources, providers, processors, and security |
| `database-designer` | Design Doctrine entities, constraints, indexes, and queries |
| `codebase-mapper` | Map an unfamiliar Symfony codebase into source-cited, commit-stamped `codebase/` documents |
| `flow-feature` | Orchestrate a complete feature with planning, approval, implementation, review, and verification |
| `flow-review` | Run parallel code, security, and performance review and synthesize one report |
| `sdd` | Run resumable spec-driven development with durable specs, tasks, and checkpoints |
| `doctrine-migration-designer` | Plan safe schema rollout, backfills, and recovery |
| `form-validator-designer` | Design Forms, request DTOs, constraints, and error behavior |
| `security-voter-designer` | Design voters, firewalls, access rules, and authorization tests |
| `messenger-designer` | Design messages, handlers, retries, idempotency, and workers |
| `frontend-design` | Design Twig, Forms, Symfony UX, and accessible interactions |
| `writing-plans` | Produce file-specific implementation plans |
| `architecture-implementer` | Scaffold approved Symfony layers |
| `coder` | Implement behavior-changing backend work |
| `coder-frontend` | Implement Twig/Symfony UX frontend behavior |
| `console-command-coder` | Implement thin, testable console commands |
| `fixture-factory-generator` | Build deterministic fixtures and test data helpers |
| `refactorer` | Perform behavior-preserving cleanup under tests |
| `systematic-debugger` | Investigate root cause before fixing failures |
| `test-generator` | Add focused tests at the correct layer |
| `architecture-boundary-reviewer` | Review Controller/Service/Repository responsibilities |
| `repository-reviewer` | Review Doctrine queries and persistence boundaries |
| `container-reviewer` | Review autowiring, aliases, tags, decorators, and config |
| `twig-ux-reviewer` | Review Twig, Forms, UX behavior, and accessibility |
| `security-reviewer` | Audit Symfony and OWASP risks |
| `performance-optimization` | Measure and fix Symfony performance problems |
| `dependency-manager` | Audit and safely update Composer dependencies |
| `code-reviewer` | Review correctness, maintainability, security, and tests |
| `verify` | Run the active edition's Definition of Done |
| `documentation-generator` | Maintain README, ADR, API, worker, and deployment docs |
| `project-brain` | Govern shared tasks, handoffs, unified retrieval, all six record types, compaction, and promotion proposals |
| `memory-bank` | Retrieve, capture, audit, supersede/archive durable memory, or apply a governed automatic/independently reviewed promotion |
| `finishing-branch` | Present merge, PR, or cleanup alternatives |
| `release` | Prepare versioning, changelog, tag, and release notes |

See [Orchestrator Commands](https://github.com/PHP-Innowise/AI-Infrastructure/blob/main/docs/ORCHESTRATOR-COMMANDS.md) for flow examples,
approval points, parallel-review limits, and write serialization.

Example flow:

```text
/requirements-analyst Add invitation-only registration
/architect Use TASK-001 to design services, repositories, voters, and Doctrine changes
/writing-plans Create an implementation plan for TASK-001
/coder Implement TASK-001
/code-reviewer Review TASK-001
/verify Run the Symfony Definition of Done
```

## Documentation Lifecycle

Temporary artifacts live in zero-padded `tasks/TASK-N/` directories and must be prefixed with the producing skill, for example `writing-plans-registration.md`.

Permanent decisions live in `specs/` and are indexed by `specs/MANIFEST.md`. Update living specs when architecture, API contracts, schema, security, asynchronous behavior, operations, or user workflows change.

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

Use project Composer scripts first, then configured equivalents:

```bash
composer validate --strict
composer audit
composer test
composer lint
composer analyse
php bin/console lint:container
php bin/console debug:router
php bin/console doctrine:schema:validate --skip-sync
```

Frontend work also runs configured template, JavaScript, CSS, test, and production-build checks. Missing tooling is reported as `N/A - tooling not configured`; it is never installed or silently treated as passing.

## Team Usage

1. Commit the shared policy and every edition used by the team.
2. Keep personal settings, IDE state, secrets, caches, and local overrides uncommitted.
3. Agree on project-level PHPUnit/Pest, coding-standard, and PHPStan/Psalm commands.
4. Keep edition skills semantically aligned while preserving native frontmatter and hook schemas.
5. Treat task docs as temporary execution context and specs as durable knowledge.
6. Review Flex recipe changes and environment/config requirements with dependency updates.
7. Trust the project in Codex so `.codex/config.toml` and hooks load.

## Symfony Adaptation Notes

This folder specializes the universal `PHP Core/` accelerator by replacing framework-neutral persistence, routing, security, async, frontend, debugging, and verification guidance with Symfony-native practices. The sibling `Laravel/` folder is used only as a reference for workflow maturity and documentation completeness; Laravel-specific concepts are not mechanically mapped into Symfony.
