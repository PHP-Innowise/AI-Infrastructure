# Project Profile: acme-billing (EXAMPLE)

> Illustrative only - a fictional PHP project. Real profiles are authored from a real target's evidence.

## 0. Metadata
- Target path: ../acme-billing
- Scanned: 2026-02-10
- Task: tasks/TASK-001/
- Generator version: 1.0.0

## 1. AI Tool Selection (MANDATORY)
- Selected editions: [cursor]
- Source: clarifying-interview answer ("we use Cursor")

## 2. PHP Stack
- PHP version: ^8.2, resolved 8.2.15 (confirmed - composer.json:L14, composer.lock)
- Framework: Laravel 11.x (confirmed - composer.json:L11 "laravel/framework": "^11.0", artisan)
- Package manager: composer (confirmed - composer.json, composer.lock)
- PSR-4 autoload map: App\ => app/, Database\Factories\ => database/factories/ (confirmed - composer.json:L40)
- Entry points: public/index.php, artisan (confirmed)
- Test tooling: Pest 2.x (confirmed - composer.json require-dev, tests/Pest.php)
- Lint/format: Laravel Pint (confirmed - pint.json)
- Static analysis: Larastan/PHPStan level 6 (confirmed - phpstan.neon:L3)

## 3. Architecture
- Pattern: modular-monolith (inferred - app/Modules/{Billing,Accounts,Notifications} with per-module namespaces)
- Layering/DDD: layered within modules (inferred - Domain/Application/Http subfolders per module)
- Service/module boundaries: Billing, Accounts, Notifications (confirmed - app/Modules tree)
- Communication: in-process + queued events (confirmed - events dispatched to queue)

### 3.1 Framework-Specialty Signals
- ORM / data-access pattern: Eloquent ORM (confirmed - app/Models/Invoice.php:L1 extends Model, database/migrations/)
- DB migration tooling: Laravel migrations, 18 files (confirmed - database/migrations/)
- Async/queue mechanism: queued jobs + queued events/listeners dispatched to the Redis connection in section 4 (confirmed - app/Jobs/ProcessRefund.php implements ShouldQueue, app/Listeners/SendInvoiceReceipt.php)
- Event listener/subscriber/observer pattern: confirmed - app/Listeners/, InvoiceObserver registered in app/Providers/EventServiceProvider.php:L22
- Multi-channel notification delivery: confirmed - app/Notifications/PaymentFailed.php via(): ['mail', 'database']
- In-app caching strategy (beyond the Redis driver in section 4): confirmed - Cache::remember() in app/Services/PricingService.php:L47, tagged by plan ID
- File/object storage abstraction (beyond the S3 disk in section 4): confirmed - Storage::disk('s3')->temporaryUrl() for invoice PDFs in app/Services/InvoicePdfService.php:L31
- Auth/authorization scaffolding (beyond Sanctum tokens in section 4/6): confirmed - app/Policies/InvoicePolicy.php + Gate::authorize() calls in InvoiceController
- Form/validator design: confirmed - app/Http/Requests/StoreInvoiceRequest.php with a custom ValidCurrency rule
- Admin/back-office panel: none
- Declarative API resource framework: none - hand-rolled routes/controllers (confirmed - routes/api.php)
- Custom console commands: confirmed - app/Console/Commands/ReconcileStripePayouts.php
- Repository/data-access layer: inferred none - Eloquent models called directly from services, no *Repository classes found
- DI container configuration style: code-driven bindings only (confirmed - bindings in app/Providers/*ServiceProvider.php, no services.yaml-style config)
- Test data factories/fixtures: confirmed - database/factories/ (InvoiceFactory, AccountFactory)
- Package vs. application: deployed application (confirmed - Dockerfile + docker-compose.yml + CI deploy step, no library-style composer.json type)

### 3.2 Frontend Presence
- Rendering/templating layer: none detected (confirmed - no resources/views, API-only routes in routes/api.php)
- Frontend asset build: none (confirmed - no package.json bundler script)
- Verdict: no UI surface - skip frontend skills (confirmed)

## 4. Integrations
- Payment: stripe/stripe-php, wired via config/services.php + a StripeClient binding (confirmed - composer.json:L18, config/services.php:L22)
- Messaging/Queue: Redis queue connection (confirmed - config/queue.php redis connection, predis/predis in require)
- Cache: Redis (confirmed - config/cache.php default=redis)
- Object storage: AWS S3 via league/flysystem-aws-s3-v3 (confirmed - config/filesystems.php s3 disk)
- Email/SMS: Symfony Mailer over SES (confirmed - config/mail.php)
- Auth/Identity: Laravel Sanctum tokens (confirmed - composer.json, config/sanctum.php)
- Observability: Sentry (confirmed - sentry/sentry-laravel, config/sentry.php)
- Search / feature flags / ML-AI / secondary DB: none

## 5. Infrastructure & Ops
- Containers: Dockerfile (php:8.2-fpm) + docker-compose.yml (app, redis, mysql) (confirmed)
- CI/CD: GitHub Actions (.github/workflows/ci.yml runs pint, phpstan, pest) (confirmed)
- IaC: none detected (unknown)
- Deployment target: (unknown - not determinable from repo; asked in interview, user unsure)

## 6. Security & Compliance
- Auth pattern: token (Sanctum) (confirmed)
- Secrets handling: .env + config; no values read (confirmed - .env.example present)
- Security tooling: composer audit step in CI (confirmed - ci.yml:L31)
- Compliance mentions: PCI referenced in docs/payments.md (textual mention only; not a compliance assertion)

## 7. Conventions
- Code style: Laravel Pint default preset (confirmed - pint.json)
- Git hooks: none detected (unknown)
- Docs/ADRs: docs/ with 4 markdown files; no formal ADRs (confirmed - docs/)

## 8. Research Notes (from stack-researcher)
- stripe/stripe-php ^13: use idempotency keys on charge creation; verify webhook signatures with the endpoint secret (source: stripe.com/docs/api, stripe.com/docs/webhooks)
- laravel/sanctum ^4: prefer ability-scoped tokens; rotate on privilege change (source: laravel.com/docs/11.x/sanctum)

## 9. Open Items
- Deployment target unknown (user unsure at interview).

## 10. Generation Notes
- Pre-existing accelerator in target: no

### 10.1 Skills To Generate (with what each will do)

**Architecture (1):**
- `billing-modular-monolith-architecture` - guidance for working within this modular monolith's Billing/Accounts/Notifications module boundaries and their layered Domain/Application/Http structure per module (from section 3)

**Design & Interaction (3, always generated):**
- `architecture-implementer` - scaffolds new feature skeletons using `artisan make:*` matching the Billing/Accounts/Notifications module layering above
- `api-designer` - designs routes/api.php endpoints using Form Requests + API Resources, Sanctum-scoped where relevant (section 3.1 shows no declarative API resource framework, so `api-platform-design` is not generated and `api-designer` covers the full API surface here, not a narrowed subset)
- `database-designer` - designs the schema itself (tables/columns/keys/indexes) and which migrations to author, consistent with the existing database/migrations structure; defers ORM usage patterns (relationships-as-used-in-code, casts, scopes) to `eloquent-patterns` below rather than duplicating them

**Frontend: skipped - no UI surface detected (section 3.2 verdict: API-only backend, no views/asset build)**

**Process & Workflow (14, always generated, framework-agnostic):**
`requirements-analyst`, `researcher`, `brainstorming`, `council`, `writing-plans`, `using-git-worktrees`, `systematic-debugger`, `refactorer`, `dependency-manager`, `review-pr`, `finishing-branch`, `documentation-generator`, `skill-creator`, `reflect` - same fixed mechanic as every target, authored against acme-billing's real git remote and docs/ layout where a project-specific convention exists. `systematic-debugger` here is the tool-agnostic root-cause methodology only (no Sentry/tool names) - it cross-references the universal `debugging` skill below rather than duplicating it.

**Universal PHP (7):**
- `coding` - Laravel 11 + PHP 8.2 conventions, formatted with Laravel Pint's default preset (pint.json)
- `testing` - Pest 2.x via tests/Pest.php; run through `./vendor/bin/pest`
- `code-review` - reviews against Larastan/PHPStan level 6 (phpstan.neon) and the module boundaries above
- `security-review` - Sanctum ability-scoped token checks, `composer audit` gate already wired into CI
- `performance` - Redis-backed queue path and outbound Guzzle-style HTTP calls as the primary hot paths; names the Redis cache only as a lever and defers to `caching-strategy` below for invalidation-correctness depth
- `release` - GitHub Actions pipeline (pint -> phpstan -> pest) as the release gate
- `debugging` - Sentry (sentry-laravel) as the first place to check for production errors; defers to `systematic-debugger` (below) for the root-cause investigative discipline itself

**Framework-Specialty (11, one per confirmed/inferred section 3.1 signal):**
- `eloquent-patterns` - advanced Eloquent usage (relationships-as-used-in-code, casts, scopes) for app/Models/Invoice.php and related models, once `database-designer` has settled the schema itself
- `migration-safety` - safe rollout order for the 18 migrations under database/migrations, given the live `invoices`/`accounts` tables
- `async-jobs` - queued Job/Event/Listener design (app/Jobs/ProcessRefund.php, app/Listeners/) dispatched to the Redis queue connection
- `event-boundary-review` - keeps app/Listeners/ and InvoiceObserver thin, delegating real work to services
- `notification-delivery` - multi-channel design for app/Notifications/PaymentFailed.php (mail + database channels)
- `caching-strategy` - cache-aside correctness for PricingService's Cache::remember() usage, tag-scoped by plan ID; owns invalidation depth that `performance` above only references
- `file-storage` - signed-URL generation pattern for InvoicePdfService's Storage::disk('s3') usage
- `auth-scaffolding` - Policy/Gate design for app/Policies/InvoicePolicy.php, distinct from Sanctum token auth
- `form-validation-design` - custom rule (ValidCurrency) and FormRequest conventions for app/Http/Requests/
- `console-commands` - CLI command design for app/Console/Commands/ReconcileStripePayouts.php
- `test-data-factories` - InvoiceFactory/AccountFactory conventions under database/factories/

**Integrations (7, one per confirmed integration in section 4):**
- `stripe-payments` - Stripe wired via a StripeClient binding in config/services.php, not just a listed dependency
- `redis-queue` - Redis queue connection (predis/predis) per config/queue.php
- `redis-cache` - Redis as the default cache store per config/cache.php
- `s3-storage` - AWS S3 object storage via league/flysystem-aws-s3-v3, config/filesystems.php's s3 disk
- `ses-mail` - Symfony Mailer routed over AWS SES per config/mail.php
- `sanctum-auth` - Laravel Sanctum token authentication per config/sanctum.php
- `sentry-observability` - Sentry error tracking per sentry/sentry-laravel and config/sentry.php

### 10.2 Agents & Commands Preview
- Skill count breakdown: 1 architecture + 3 design + 0 frontend + 14 process + 7 universal + 11 specialty + 7 integrations = **43 skills**.
- Agents: 43 skills x 1 agent-carrying selected edition (Cursor) = 43 agents.
- Commands: 43 skills x 1 command-carrying selected edition (Cursor) = 43 commands.
- Claude and Codex were not selected, so no Claude agents/commands and no Codex skills tree are generated for this target.

### 10.3 Non-PHP Neighbors (integration contracts only)
- none

## 11. Memory Bank Preview

One shared `memory-bank/` will be created at `acme-billing/`'s root. `memory-seed` will seed the following 15 chunks - one per durable `confirmed` fact - and none for the `inferred` architecture-pattern label, the `inferred none` repository-layer finding, or the `unknown` deployment target:

| Planned ID | Title | Type | Source |
| --- | --- | --- | --- |
| MEM-0001 | Framework: Laravel 11 on PHP 8.2.15 | architecture | composer.json:L11,L14 |
| MEM-0002 | Module boundaries: Billing, Accounts, Notifications | architecture | app/Modules tree |
| MEM-0003 | ORM: Eloquent, migrations under database/migrations (18 files) | architecture | app/Models/Invoice.php:L1; database/migrations/ |
| MEM-0004 | Async: queued Jobs/Events/Listeners over the Redis queue connection | architecture | app/Jobs/ProcessRefund.php; app/Listeners/ |
| MEM-0005 | Authorization: Policy/Gate layer (InvoicePolicy) distinct from Sanctum auth | architecture | app/Policies/InvoicePolicy.php |
| MEM-0006 | Payment: Stripe via StripeClient binding | integration | composer.json:L18; config/services.php:L22 |
| MEM-0007 | Queue: Redis connection (predis/predis) | integration | config/queue.php |
| MEM-0008 | Cache: Redis default store | integration | config/cache.php |
| MEM-0009 | Object storage: AWS S3 via league/flysystem-aws-s3-v3 | integration | config/filesystems.php |
| MEM-0010 | Email: Symfony Mailer over AWS SES | integration | config/mail.php |
| MEM-0011 | Auth: Laravel Sanctum token authentication | integration | config/sanctum.php |
| MEM-0012 | Observability: Sentry | integration | sentry/sentry-laravel; config/sentry.php |
| MEM-0013 | Ops: custom console command ReconcileStripePayouts | operations | app/Console/Commands/ReconcileStripePayouts.php |
| MEM-0014 | CI/CD: GitHub Actions runs Pint, PHPStan, Pest | operations | .github/workflows/ci.yml |
| MEM-0015 | Code style: Laravel Pint default preset | convention | pint.json |

**Chunks to be seeded:** 15

## Confidence Summary
41 confirmed, 3 inferred, 3 unknown
