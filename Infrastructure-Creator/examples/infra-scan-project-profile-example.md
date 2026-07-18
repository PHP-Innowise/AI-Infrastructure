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
- Skills to generate: architecture (modular-monolith); universal (coding, testing, code-review, security-review, performance, release, debugging); integrations (stripe-payments, redis-queue, redis-cache, s3-storage, ses-mail, sanctum-auth, sentry-observability)
- Non-PHP neighbors (integration contracts only): none

## Confidence Summary
21 confirmed, 3 inferred, 3 unknown
