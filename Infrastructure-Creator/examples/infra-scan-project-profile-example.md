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
- Architecture: `billing-modular-monolith-architecture` - guidance for working within this modular monolith's Billing/Accounts/Notifications module boundaries and their layered Domain/Application/Http structure per module (from section 3)
- Universal PHP:
  - `coding` - Laravel 11 + PHP 8.2 conventions, formatted with Laravel Pint's default preset (pint.json)
  - `testing` - Pest 2.x via tests/Pest.php; run through `./vendor/bin/pest`
  - `code-review` - reviews against Larastan/PHPStan level 6 (phpstan.neon) and the module boundaries above
  - `security-review` - Sanctum ability-scoped token checks, `composer audit` gate already wired into CI
  - `performance` - Redis-backed queue/cache paths and outbound Guzzle-style HTTP calls as the primary hot paths
  - `release` - GitHub Actions pipeline (pint -> phpstan -> pest) as the release gate
  - `debugging` - Sentry (sentry-laravel) as the first place to check for production errors
- Integrations (one per confirmed integration in section 4):
  - `stripe-payments` - Stripe wired via a StripeClient binding in config/services.php, not just a listed dependency
  - `redis-queue` - Redis queue connection (predis/predis) per config/queue.php
  - `redis-cache` - Redis as the default cache store per config/cache.php
  - `s3-storage` - AWS S3 object storage via league/flysystem-aws-s3-v3, config/filesystems.php's s3 disk
  - `ses-mail` - Symfony Mailer routed over AWS SES per config/mail.php
  - `sanctum-auth` - Laravel Sanctum token authentication per config/sanctum.php
  - `sentry-observability` - Sentry error tracking per sentry/sentry-laravel and config/sentry.php

### 10.2 Agents & Commands Preview
- Agents: 15 skills (1 architecture + 7 universal + 7 integrations) x 1 agent-carrying selected edition (Cursor) = 15 agents.
- Commands: 15 skills x 1 command-carrying selected edition (Cursor) = 15 commands.
- Claude and Codex were not selected, so no Claude agents/commands and no Codex skills tree are generated for this target.

### 10.3 Non-PHP Neighbors (integration contracts only)
- none

## 11. Memory Bank Preview

One shared `memory-bank/` will be created at `acme-billing/`'s root. `memory-seed` will seed the following 11 chunks - one per durable `confirmed` fact - and none for the `inferred` architecture-pattern label or the `unknown` deployment target:

| Planned ID | Title | Type | Source |
| --- | --- | --- | --- |
| MEM-0001 | Framework: Laravel 11 on PHP 8.2.15 | architecture | composer.json:L11,L14 |
| MEM-0002 | Module boundaries: Billing, Accounts, Notifications | architecture | app/Modules tree |
| MEM-0003 | Payment: Stripe via StripeClient binding | integration | composer.json:L18; config/services.php:L22 |
| MEM-0004 | Queue: Redis connection (predis/predis) | integration | config/queue.php |
| MEM-0005 | Cache: Redis default store | integration | config/cache.php |
| MEM-0006 | Object storage: AWS S3 via league/flysystem-aws-s3-v3 | integration | config/filesystems.php |
| MEM-0007 | Email: Symfony Mailer over AWS SES | integration | config/mail.php |
| MEM-0008 | Auth: Laravel Sanctum token authentication | integration | config/sanctum.php |
| MEM-0009 | Observability: Sentry | integration | sentry/sentry-laravel; config/sentry.php |
| MEM-0010 | CI/CD: GitHub Actions runs Pint, PHPStan, Pest | operations | .github/workflows/ci.yml |
| MEM-0011 | Code style: Laravel Pint default preset | convention | pint.json |

**Chunks to be seeded:** 11

## Confidence Summary
21 confirmed, 3 inferred, 3 unknown
