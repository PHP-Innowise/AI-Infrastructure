# Symfony Layered Architecture Accelerator

> **Enforceable agent policy lives in [AGENTS.md](AGENTS.md).**

A Symfony-first workflow accelerator for AI coding agents. It provides focused commands, single-skill agents, reusable engineering workflows, safety hooks, quality gates, and documentation conventions for teams building maintainable Symfony applications with a pragmatic default:

```text
Controller -> Service -> Repository
```

This is the `Symfony/` folder of the `accelerator-php` monorepo. The framework-neutral PHP base lives in the sibling `PHP Core/` folder; `Laravel/` is the equivalent Laravel specialization — see the [repository root README](../README.md) for the full comparison and usage instructions.

## What This Is

This repository is not a generated Symfony application. It is an engineering workflow layer that can be placed alongside an existing project:

- Commands route intent to a focused workflow.
- Agent wrappers execute one skill, return context, and stop.
- Skills define repeatable design, implementation, review, debugging, and delivery practices.
- Hooks enforce skill-prefixed task/spec names, zero-padded task directories, Git safety, database safety, and workflow constraints.
- `tasks/` stores temporary task artifacts; `specs/` stores durable architecture knowledge.
- `memory-bank/` stores small indexed chunks of verified reusable context shared across AI tools.

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

## Directory Structure

```text
AGENTS.md                 # Shared enforceable policy
README.md                 # Public Symfony accelerator guide
CHANGELOG.md              # Versioned accelerator changes

.claude/                  # Claude Code edition
├── agents/               # One-skill wrappers
├── commands/             # Slash-command entry points
├── hooks/                # Safety and workflow hooks
├── skills/               # Canonical authored workflows
├── DOD.md
├── GOLDEN-PRINCIPLES.md
└── STABILIZATION.md

.cursor/                  # Cursor-native mirror and adapters
.agents/skills/           # Codex-discovered skill mirror
.codex/                   # Codex config, hooks, and references
tasks/TASK-N/             # Temporary prefixed task artifacts
specs/                    # Permanent living specifications
memory-bank/              # Indexed durable cross-session project memory
examples/                 # Workflow output examples
```

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
| `memory-bank` | Retrieve, capture, audit, supersede, or archive durable project memory |
| `finishing-branch` | Present merge, PR, or cleanup alternatives |
| `release` | Prepare versioning, changelog, tag, and release notes |

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

## Memory Bank

The optional root `memory-bank/` is one canonical shared store for Claude Code, Cursor, and Codex. It contains small source-backed chunks for durable project constraints, conventions, decisions, domain knowledge, integrations, and operational lessons.

Memory is deliberately below policy, specs, code, configuration, migrations, and tests in the authority hierarchy. Agents read `memory-bank/README.md` and `memory-bank/INDEX.md`, retrieve only relevant active chunks, and verify every material claim before relying on it. Stale chunks are updated, superseded, or archived instead of silently remaining active.

Use the `memory-bank` skill directly in Codex or `/memory-bank` in Claude/Cursor to retrieve, capture, audit, supersede, archive, or initialize memory. Do not use it for transient plans, chat transcripts, generic Symfony advice, command output, or information already owned by a living spec. Secrets, `.env` contents, personal data, production identifiers, raw logs, and customer payloads are prohibited. Non-sensitive personal notes belong in ignored `memory-bank/local/`.

Each committed chunk uses `memory-bank/chunks/MEM-NNNN-short-slug.md`, is cataloged in `INDEX.md`, and cites its authoritative sources. The session-start hooks report counts only; they never inject chunk contents into logs or context automatically.

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
