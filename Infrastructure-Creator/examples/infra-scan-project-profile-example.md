# Project Profile: acme-billing (EXAMPLE)

> Illustrative only - a fictional PHP project. Real profiles are authored from a real target's evidence.

## 0. Metadata
- Target path: ../acme-billing
- Scanned: 2026-02-10
- Task: tasks/TASK-001/
- Generator version: 1.0.0

## 1. AI Tool Selection (MANDATORY)
- Selected editions: [claude]
- Source: clarifying-interview answer ("we use Claude Code")

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

## 8. Domain & Behavioral Contract

### 8.1 Project Purpose & Domain Vocabulary
- Purpose: API service for issuing invoices, collecting payments, and processing refunds for customer accounts (confirmed; spec/ADR - README.md:L5-L12)
- Invoice: billing record with draft/issued/paid/refunded lifecycle (confirmed; domain code - app/Models/Invoice.php:L18-L44)
- Settlement: confirmed provider capture after which a payment may be refunded (confirmed; domain code - app/Modules/Billing/Domain/Payment.php:L31-L49)

### 8.2 Project Sources of Truth
- Payment-provider contract: docs/payments.md governs idempotency, webhook verification, and refund behavior (confirmed; spec/ADR - docs/payments.md:L1-L58)
- Invoice lifecycle: tests/Feature/Billing/InvoiceLifecycleTest.php is executable authority for allowed/forbidden transitions (confirmed; test - tests/Feature/Billing/InvoiceLifecycleTest.php:L12-L96)
- Database shape: database/migrations/ is authoritative for deployed schema, not model docblocks (confirmed; configuration - docs/architecture.md:L22-L25)
- Contradictions: README calls `voided` a status, but no enum, migration, workflow, or test supports it (inferred; spec/ADR + domain code - README.md:L31; app/Enums/InvoiceStatus.php)

### 8.3 Core Modules & Domain Model
- Billing: invoices, payment capture, refund workflow; app/Modules/Billing (confirmed; domain code - app/Modules/Billing/)
- Accounts: account ownership and billing access; app/Modules/Accounts (confirmed; domain code - app/Modules/Accounts/)
- Invoice: belongs to Account; has many Payments; status and total are integrity-sensitive (confirmed; domain code + database constraint - app/Models/Invoice.php:L18-L44; database/migrations/2026_01_10_create_invoices.php)

### 8.4 Business Invariants
- A paid invoice is immutable except through the refund workflow (confirmed; test - tests/Feature/Billing/InvoiceLifecycleTest.php:L55-L72)
- A refund requires a settled payment and may not exceed the remaining refundable amount (confirmed; domain code + test - app/Modules/Billing/Domain/Refund.php:L24-L51; tests/Feature/Billing/RefundTest.php:L30-L68)
- Stripe capture uses the invoice UUID as the idempotency key (confirmed; application code - app/Services/StripePaymentService.php:L42-L48)

### 8.5 Lifecycles & Transitions
- Statuses discovered: Invoice -> draft, issued, paid, refunded (confirmed; domain code - app/Enums/InvoiceStatus.php:L7-L14)
- Confirmed transition: Invoice draft -> issued; allowed when at least one line exists; forbidden when total is non-positive (confirmed; test - tests/Feature/Billing/InvoiceLifecycleTest.php:L20-L38)
- Confirmed transition: Invoice issued -> paid; allowed after confirmed provider capture (confirmed; test - tests/Feature/Billing/InvoiceLifecycleTest.php:L40-L53)
- Confirmed transition: Invoice paid -> refunded; allowed through RefundService with remaining refundable amount (confirmed; test - tests/Feature/Billing/RefundTest.php:L30-L68)

### 8.6 Roles & Permissions
- Account owner -> view invoices for owned account; cross-account access denied (confirmed; authorization rule + test - app/Policies/InvoicePolicy.php:L18-L31; tests/Feature/Billing/InvoiceAuthorizationTest.php:L15-L44)
- Finance admin -> initiate refund; standard account user denied (confirmed; authorization rule + test - app/Policies/InvoicePolicy.php:L42-L55; tests/Feature/Billing/RefundAuthorizationTest.php:L12-L37)
- Completeness: complete for invoice view and refund endpoints only; other Billing actions remain unknown (confirmed; authorization rule - routes/api.php:L30-L58)

### 8.7 Audit Obligations
- Refund requested/completed/failed events record actor ID, invoice UUID, amount, provider reference, and timestamp (confirmed; test - tests/Feature/Billing/RefundAuditTest.php:L18-L61)

### 8.8 High-Risk / Forbidden Workflows
- Refund creation: money + irreversible external contract; requires Finance Admin policy check and refund regression suite; no additional approval threshold documented (confirmed; authorization rule + test - app/Policies/InvoicePolicy.php:L42-L55; tests/Feature/Billing/RefundTest.php)
- Direct invoice status assignment outside InvoiceLifecycleService is forbidden (confirmed; spec/ADR - docs/payments.md:L44-L48)

### 8.9 Critical QA / Regression Scenarios
- Paid invoice update is rejected and original values remain unchanged (confirmed; test - tests/Feature/Billing/InvoiceLifecycleTest.php:L55-L72)
- Retried Stripe capture reuses the same idempotency key and does not create a second payment (confirmed; test - tests/Feature/Billing/StripeIdempotencyTest.php:L17-L49)
- Non-finance user cannot refund an invoice (confirmed; test - tests/Feature/Billing/RefundAuthorizationTest.php:L12-L37)

### 8.10 Known Risks & Incident Lessons
- A historical webhook retry created duplicate payments when idempotency lookup occurred after insert; prevention rule: resolve idempotency before persistence and keep the regression test (confirmed; spec/ADR + test - docs/incidents/2026-01-duplicate-payment.md:L20-L34; tests/Feature/Billing/StripeIdempotencyTest.php)

### 8.11 Domain Skill Candidates
- `billing-rules-review` - Billing bounded context; protects invoice lifecycle, refund limits/authorization/audit, Stripe idempotency, and related regression scenarios (confirmed; sources in sections 8.4-8.10)

## 9. Research Notes (from stack-researcher)
- stripe/stripe-php ^13: use idempotency keys on charge creation; verify webhook signatures with the endpoint secret (source: stripe.com/docs/api, stripe.com/docs/webhooks)
- laravel/sanctum ^4: prefer ability-scoped tokens; rotate on privilege change (source: laravel.com/docs/11.x/sanctum)

## 10. Open Items
- Deployment target unknown (user unsure at interview).
- Whether non-refund Billing actions have a complete authorization matrix remains unknown.
- README's unsupported `voided` status requires correction or an authoritative workflow update.

## 11. Generation Notes
- Pre-existing accelerator in target: no

### 11.1 Skills To Generate (with what each will do)

**Architecture (1):**
- `billing-modular-monolith-architecture` - preserves Billing/Accounts/Notifications boundaries and assigns confirmed Billing invariants to the owning layer

**Design & Interaction (3):**
- `architecture-implementer` - scaffolds Laravel module skeletons while keeping invoice transitions and refund side effects inside their confirmed services
- `api-designer` - designs Form Request/API Resource endpoints while enforcing confirmed InvoicePolicy and lifecycle guards
- `database-designer` - designs tables/constraints/migrations and maps confirmed integrity invariants without duplicating Eloquent usage guidance

**Frontend:** skipped - no UI surface detected.

**Process & Workflow (15):**
`requirements-analyst`, `researcher`, `brainstorming`, `council`, `writing-plans`, `using-git-worktrees`, `systematic-debugger`, `refactorer`, `dependency-manager`, `review-pr`, `finishing-branch`, `documentation-generator`, `skill-creator`, `reflect`, `memory-bank`. `memory-bank` operates the shared seeded bank through retrieve/capture/supersede/audit modes.

**Universal PHP (7):**
- `coding` - Laravel 11/PHP 8.2 implementation respecting Billing invariants and Pint
- `testing` - Pest critical scenarios for lifecycle, refund authorization/audit, and Stripe idempotency
- `code-review` - maps diffs to affected invariants, permissions, audit obligations, and regression scenarios
- `security-review` - checks Sanctum plus object-level/account ownership and Finance Admin refund enforcement
- `performance` - measures Redis queue and outbound HTTP hot paths, deferring cache correctness to `caching-strategy`
- `release` - GitHub Actions Pint -> PHPStan -> Pest gate plus affected critical scenarios
- `debugging` - Sentry/runbook locations plus sanitized duplicate-payment prevention rule; methodology stays in `systematic-debugger`

**Framework-Specialty (11):**
`eloquent-patterns`, `migration-safety`, `async-jobs`, `event-boundary-review`, `notification-delivery`, `caching-strategy`, `file-storage`, `auth-scaffolding`, `form-validation-design`, `console-commands`, `test-data-factories`.

**Integrations (7):**
`stripe-payments`, `redis-queue`, `redis-cache`, `s3-storage`, `ses-mail`, `sanctum-auth`, `sentry-observability`.

**Domain (1):**
- `billing-rules-review` - reviews changes against invoice transitions, refund amount/role/audit rules, Stripe idempotency, and the named regression scenarios, while surfacing the unresolved `voided` contradiction

### 11.2 Agents & Commands Preview
- Skill count: 1 architecture + 3 design + 0 frontend + 15 process + 7 universal + 11 specialty + 7 integrations + 1 domain = **45 skills**.
- Agents: 45 skills x 1 Claude edition = 45 agents.
- Commands: 45 skills x 1 Claude edition = 45 commands.

### 11.3 Non-PHP Neighbors
- none

## 12. Memory Bank Preview

One shared bank will seed 10 cohesive concepts. It links canonical sources rather than copying specs, matrices, tests, or incident narratives.

| Planned ID | Title | Type | Source |
| --- | --- | --- | --- |
| MEM-0001 | Laravel 11 / PHP 8.2 platform and verification tooling | architecture | composer.json; pint.json; phpstan.neon; tests/Pest.php |
| MEM-0002 | Billing, Accounts, Notifications module boundaries | architecture | app/Modules/; docs/architecture.md |
| MEM-0003 | Invoice lifecycle and immutability invariants | domain | app/Enums/InvoiceStatus.php; tests/Feature/Billing/InvoiceLifecycleTest.php |
| MEM-0004 | Refund amount, authorization, and audit contract | domain | app/Modules/Billing/Domain/Refund.php; app/Policies/InvoicePolicy.php; tests/Feature/Billing/RefundAuditTest.php |
| MEM-0005 | Stripe idempotency and webhook contract | integration | docs/payments.md; app/Services/StripePaymentService.php; tests/Feature/Billing/StripeIdempotencyTest.php |
| MEM-0006 | Redis queue/cache and async processing contract | integration | config/queue.php; config/cache.php; app/Jobs/ProcessRefund.php |
| MEM-0007 | S3 invoice storage and SES delivery contract | integration | config/filesystems.php; app/Services/InvoicePdfService.php; config/mail.php |
| MEM-0008 | Sanctum authentication and account ownership boundary | constraint | config/sanctum.php; app/Policies/InvoicePolicy.php |
| MEM-0009 | Sentry observability and duplicate-payment prevention lesson | operations | config/sentry.php; docs/incidents/2026-01-duplicate-payment.md |
| MEM-0010 | GitHub Actions release gate and Pint convention | operations | .github/workflows/ci.yml; pint.json |

**Chunks to be seeded:** 10

## Confidence Summary
54 confirmed, 5 inferred, 5 unknown
