---
name: architecture-scanner
description: Detect a PHP target's architecture style (monolith, modular-monolith, microservices, event-driven), layering/DDD approach (layered, hexagonal/ports-and-adapters, none), module/service boundaries, communication style, framework-specialty implementation signals (ORM/data-access, async/queue, events, caching, storage, auth scaffolding, admin panel, migrations, DI container, repositories, test factories, console commands, package-vs-app), and frontend/rendering presence, from real evidence. Use as Phase 1 discovery input to profile-synthesizer. Triggers on "scan the architecture", "detect the architecture", "is this a monolith or microservices", "how is this project layered", "what ORM/queue/caching pattern does this use", "architecture-scanner".
phase: discovery
flow-next: profile-synthesizer
flow-alternatives: [stack-researcher]
related: [stack-scanner, integration-scanner, infra-ops-scanner, security-compliance-scanner, conventions-scanner, infra-scan]
---

# Architecture Scanner

## Overview

Read-only reconnaissance of a PHP target's architectural shape: whether it is a monolith, modular-monolith, microservices system, or event-driven design; how it is layered (classic layered, hexagonal/ports-and-adapters, DDD, or none); where module/service boundaries fall; how components communicate (HTTP controllers, message/event buses, RPC); which framework-specialty implementation patterns are in play (ORM/data-access, migrations, async/queue, events, caching, storage, auth scaffolding, admin panel, DI container style, repositories, test factories, console commands, package-vs-app nature); and whether a rendering/frontend layer exists at all. This scanner reads structure and dependencies only - it does not evaluate third-party integration wiring (that's `integration-scanner`'s job; overlapping signals like "which cache/queue broker" are cross-referenced, not re-detected), infra, security, or code-style conventions, which belong to their own scanners.

The target project path is a **required** argument (e.g. "scan the architecture of ../my-php-app"). Never assume the current working directory is the target. Operate strictly read-only within that path; never write to it, never read its `.env`/secrets.

## Generated File Naming Convention (MANDATORY)

Write exactly one findings file into the current run's task directory: `tasks/TASK-{N}/architecture-scanner-findings.md`. Never write into the target.

## Process

1. **Map the directory tree.** Record top-level and second-level directories (`src/`, `app/`, `modules/`, `packages/`, `services/`, `domain/`, `application/`, `infrastructure/`) as the primary structural evidence, citing paths.
2. **Read the PSR-4 map in `composer.json`.** Multiple namespace roots (e.g. `App\`, `Billing\`, `Catalog\`) mapped to distinct paths signal module boundaries; a single flat root signals a plain monolith. Cite `composer.json` lines.
3. **Detect monorepo / multi-service layout.** Search for more than one `composer.json` (e.g. under `packages/*` or `services/*`); multiple runnable roots suggest modular-monolith or microservices. Cite each `composer.json` path.
4. **Classify layering/DDD.** Look for `Domain/`, `Application/`, `Infrastructure/`, `Ports/`, `Adapters/` folders (hexagonal), or `Http/`, `Service/`, `Repository/` (layered). Absence of any such split marks `inferred` "none".
5. **Detect communication style.** HTTP controllers (`Controller` classes, route files), message/event buses (`symfony/messenger`, `league/tactician`, Laravel bus/events, `php-amqplib/php-amqplib`, `enqueue/*`), and RPC/gRPC packages. Cite package + wiring where present.
6. **Infer boundaries from bounded contexts.** Correlate namespace roots, directory names, and per-module `composer.json` to name candidate boundaries; keep them `inferred` unless a manifest confirms them.
7. **Detect framework-specialty implementation signals** (feeds profile section 3.1 - see `skill-forge/references/php-specialty-skills.md` for what each drives):
   - **ORM/data-access pattern:** `app/Models` + Eloquent base class, or Doctrine `#[ORM\Entity]`/`@ORM\Entity` annotations/attributes, or plain PDO/query-builder usage. Cite the base class/attribute and a real file.
   - **DB migration tooling:** `database/migrations/*.php` (Laravel), `migrations/Version*.php` (Doctrine), or another migration tool's directory; note file count as a proxy for "non-trivial history".
   - **Async/queue mechanism:** queued Job classes (`implements ShouldQueue`), Symfony Messenger message/handler classes, or scheduled-task registration. Cite real class names.
   - **Event listener/subscriber/observer pattern:** `Event`/`Listener` classes, model Observers, or `EventSubscriberInterface` implementations. Cite real class names.
   - **Multi-channel notification delivery:** `Notification`/`Mailable` classes with more than one channel (`via()` returning mail+database+broadcast, etc.), distinct from a bare mail-integration package.
   - **In-app caching strategy:** `Cache::` facade calls, `CacheInterface`/`TagAwareCacheInterface` usage, or model-level cache wrapping, beyond just a cache driver being configured.
   - **File/object storage abstraction:** `Storage::` facade usage, Flysystem adapter wiring, or signed-URL generation calls, beyond just a storage driver being configured.
   - **Auth/authorization scaffolding:** `app/Policies` + `Gate`/`can()` calls, or Symfony Voter classes (`extends Voter`) + `access_control`/`is_granted()` calls.
   - **Form/validator design:** dedicated `FormRequest`/Symfony `FormType` classes, custom `Rule`/`Constraint` classes, or validation-group usage - beyond simple inline array rules.
   - **Admin/back-office panel:** an admin-panel package in `composer.json` (e.g. an Eloquent-CRUD admin generator, EasyAdmin/Sonata-style bundle) plus its Resource/Controller registration.
   - **Declarative API resource framework:** attribute/annotation-driven API resource classes (e.g. `#[ApiResource]`) distinct from hand-written route+controller pairs.
   - **Custom console commands:** classes extending the framework's `Command` base beyond the framework's own built-ins; cite the command class and its registration.
   - **Repository/data-access layer:** `*Repository` classes wrapping ORM/PDO access, distinct from calling the ORM directly in controllers/services.
   - **DI container configuration style:** presence of a declarative container config (`services.yaml`, `services.php` with `->autowire()`, compiler passes) vs. purely code-driven bindings in Service Providers.
   - **Test data factories/fixtures:** `database/factories/*Factory.php`, Foundry factory classes, or hand-rolled fixture/object-mother classes.
   - **Package vs. application nature:** `composer.json` `type` field, absence of a deployment target (cross-check section 5), and whether the project is meant to be `require`d by a consumer vs. deployed directly.
8. **Detect frontend/rendering presence** (feeds profile section 3.2): a templating engine (`resources/views/*.blade.php`, `templates/*.twig`, or plain `.php` view files rendered by controllers) and/or a frontend asset build (`package.json` with a bundler script, `resources/js/`, `assets/` with a build config). If neither exists, record the verdict as "no UI surface - skip frontend skills" rather than guessing.
9. **Mark confidence** per finding: `confirmed` (direct evidence), `inferred` (indirect signal), or `unknown`. Never present a guess as fact.

## Output Template

```markdown
# Architecture Scanner Findings: [target_name]

**Target:** [path]  **Scanned:** [date]

## Architecture Style
- [monolith | modular-monolith | microservices | event-driven] (confirmed/inferred - evidence path:L#)

## Layering / DDD
- [layered | hexagonal/ports-and-adapters | DDD | none] (confirmed/inferred - path)

## Module / Service Boundaries
- [boundary name => namespace/path] for each (confirmed/inferred - composer.json:L# / dir)

## Monorepo Signals
- [count and paths of composer.json files, or "single root"] (confirmed - paths)

## Communication Style
- [HTTP controllers | message/event bus | RPC] (confirmed/inferred - package + wiring path:L#)

## Framework-Specialty Signals
- ORM/data-access: [...] (confidence - path) | none
- DB migration tooling: [...] (confidence - path) | none
- Async/queue mechanism: [...] (confidence - path) | none
- Event listener/subscriber/observer: [...] (confidence - path) | none
- Notification delivery: [...] (confidence - path) | none
- Caching strategy: [...] (confidence - path) | none
- File/object storage abstraction: [...] (confidence - path) | none
- Auth/authorization scaffolding: [...] (confidence - path) | none
- Form/validator design: [...] (confidence - path) | none
- Admin/back-office panel: [...] (confidence - path) | none
- Declarative API resource framework: [...] (confidence - path) | none
- Custom console commands: [...] (confidence - path) | none
- Repository/data-access layer: [...] (confidence - path) | none
- DI container configuration style: [...] (confidence - path)
- Test data factories/fixtures: [...] (confidence - path) | none
- Package vs. application: [...] (confidence - path)

## Frontend Presence
- Rendering/templating layer: [...] (confidence - path) | none
- Frontend asset build: [...] (confidence - path) | none
- Verdict: [frontend skill group applies | no UI surface - skip frontend skills]

## Confidence Summary
[X confirmed, Y inferred, Z unknown]
```

## Guardrails

- MUST cite a real file path (and line where practical) for every finding.
- MUST operate read-only on the target; MUST NOT read `.env`/secrets.
- MUST base architecture claims on PHP evidence (directory tree, PSR-4 map, composer.json count, message-bus packages, layering folders).
- MUST report absent structure as `inferred none` rather than asserting a style without evidence.
- MUST name every framework-specialty signal generically (the pattern, not a specific framework) unless the target's real framework IS the evidence.
- MUST report the frontend verdict explicitly - never silently omit it, and never assume a UI surface exists without templating/asset evidence.
- MUST NOT deep-dive third-party integration wiring, infra, security, or code-style conventions - those belong to their own scanners.

## Final Output

Return the findings file path, the detected architecture style, layering approach, candidate boundaries, communication style, the framework-specialty signal summary, and the frontend verdict, plus a one-line confidence summary. Suggest `profile-synthesizer` (to fold this into the target profile) as the next step.
