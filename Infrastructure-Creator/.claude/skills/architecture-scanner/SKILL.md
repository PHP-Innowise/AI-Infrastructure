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

The target project path is a **required** argument; never assume the current working directory is the target. Operate strictly read-only within it, under the contract's secrets rule.

## Outputs (MANDATORY)

Per run: exactly one report `tasks/TASK-{NNN}/architecture-scanner-findings.md` and exactly one evidence ledger `tasks/TASK-{NNN}/architecture-scanner-evidence.json`, both shaped by `stack-scanner/references/scan-evidence-contract.md` - read it first. Never write into the target.

## Process

1. **Map the directory tree.** Record top-level and second-level directories (`src/`, `app/`, `modules/`, `packages/`, `services/`, `domain/`, `application/`, `infrastructure/`) as the primary structural evidence, citing paths.
2. **Read the PSR-4 map in `composer.json`.** Multiple namespace roots (e.g. `App\`, `Billing\`, `Catalog\`) mapped to distinct paths signal module boundaries; a single flat root signals a plain monolith. Cite `composer.json` lines.
3. **Detect monorepo / multi-service layout.** Search for more than one `composer.json` (e.g. under `packages/*` or `services/*`); multiple runnable roots suggest modular-monolith or microservices. Cite each `composer.json` path.
4. **Classify layering/DDD.** Look for `Domain/`, `Application/`, `Infrastructure/`, `Ports/`, `Adapters/` folders (hexagonal), or `Http/`, `Service/`, `Repository/` (layered). Absence of any such split marks `inferred` "none".
5. **Detect communication style.** HTTP controllers (`Controller` classes, route files), message/event buses (`symfony/messenger`, `league/tactician`, Laravel bus/events, `php-amqplib/php-amqplib`, `enqueue/*`), and RPC/gRPC packages. Cite package + wiring where present.
6. **Map boundary ownership and material adjacency.** Correlate namespace roots, directory names, per-module `composer.json`, dependency wiring, call sites, and canonical architecture documents to name candidate boundaries and the paths each authority governs. Keep ownership `inferred` unless direct evidence confirms it. For every material cross-boundary interaction, record caller, primary owner, adjacent/deferred owner, contract path, failure handoff, and representative positive/negative/ambiguous/cross-domain routing requests; do not reduce a boundary to one nearest sibling.
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
10. **Classify path authority.** For paths likely to become generated skill ownership, label them `required-existing`, `generated-runtime`, or `creatable`, cite the source that grants that classification, and leave unsupported write paths unknown. Directory shape alone proves location, not write authority.

## Report Structure

Follow the `architecture-scanner` report template in appendix A of `stack-scanner/references/scan-evidence-contract.md`. Every factual line carries its confidence tag and its evidence id.

## Guardrails

- MUST cite a real file path (and line where practical) for every finding, and MUST emit both artifacts with contract-shaped evidence records (target-relative path, `sha256:` fingerprint, supported claims).
- MUST operate read-only on the target; MUST NOT read `.env`/secrets.
- MUST base architecture claims on PHP evidence (directory tree, PSR-4 map, composer.json count, message-bus packages, layering folders).
- MUST report absent structure as `inferred none` rather than asserting a style without evidence.
- MUST name every framework-specialty signal generically (the pattern, not a specific framework) unless the target's real framework IS the evidence.
- MUST report the frontend verdict explicitly - never silently omit it, and never assume a UI surface exists without templating/asset evidence.
- MUST NOT treat a detected package, directory, or catalog capability as proof that the target owns every behavior commonly associated with it.
- MUST capture every material adjacent owner and path authority needed for routing/ownership synthesis, including ambiguity rather than forcing a single sibling.
- MUST NOT deep-dive third-party integration wiring, infra, security, or code-style conventions - those belong to their own scanners.

## Final Output

Return both artifact paths (report and evidence ledger), the detected architecture style, layering approach, candidate boundaries, communication style, the framework-specialty signal summary, and the frontend verdict, plus a one-line confidence summary. Suggest `profile-synthesizer` (to fold this into the target profile) as the next step.
