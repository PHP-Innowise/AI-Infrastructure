# PHP Integration Catalog

Integration categories, common PHP packages that signal them, and what a good generated per-integration skill should cover. Used by `integration-scanner` (to categorize) and `skill-forge` (to shape the generated skill). Only generate a skill for an integration marked `confirmed` (package present AND runtime-wired).

## Payment
- **Signals:** `stripe/stripe-php`, `srmklive/paypal`, `omnipay/*`, `mollie/mollie-api-php`, `braintree/braintree_php`.
- **Skill covers:** SDK client setup from config (never hardcoded keys), idempotency keys, webhook verification, error/retry handling, PCI-scope awareness (never log card data), test vs live mode.

## Messaging / Queue
- **Signals:** `predis/predis` (as queue), `php-amqplib/php-amqplib`, `enqueue/*`, `aws/aws-sdk-php` (SQS), Messenger transports.
- **Skill covers:** producer/consumer contracts, idempotent handlers, retry/backoff, dead-letter handling, visibility timeouts, at-least-once delivery.

## Search
- **Signals:** `elasticsearch/elasticsearch`, `meilisearch/meilisearch-php`, `algolia/algoliasearch-client-php`, `laravel/scout`.
- **Skill covers:** index mapping/settings, indexing pipeline, query building, relevance tuning, reindex strategy, keeping the index in sync with the source of truth.

## Cache
- **Signals:** `predis/predis`, `symfony/cache`, `psr/cache`/`psr/simple-cache` implementations, Memcached extension.
- **Skill covers:** cache keys/namespacing, TTLs, stampede protection, invalidation on write, what is safe to cache.

## Object Storage
- **Signals:** `league/flysystem-aws-s3-v3`, `league/flysystem`, GCS/Azure adapters.
- **Skill covers:** disk/adapter config, private vs public visibility, signed URLs, streaming large files, avoiding loading blobs into memory.

## Email / SMS
- **Signals:** `symfony/mailer`, `mailgun/mailgun-php`, `sendgrid/sendgrid`, `twilio/sdk`.
- **Skill covers:** transport config, templating, queueing sends, bounce/complaint handling, not leaking PII in logs.

## Auth / Identity
- **Signals:** `laravel/sanctum`, `laravel/passport`, `lexik/jwt-authentication-bundle`, `firebase/php-jwt`, OAuth client packages.
- **Skill covers:** token lifecycle, scope/abilities, refresh/rotation, secure storage, guard/middleware wiring, session vs stateless.

## Observability
- **Signals:** `sentry/sentry`, `open-telemetry/*`, framework debug/profiler tooling, `monolog/monolog` handlers to external sinks.
- **Skill covers:** structured logging, error capture with context (no secrets), tracing spans, sampling, correlating requests to async work.

## Feature Flags / CDN / ML-AI / Secondary DB
- **Signals:** feature-flag SDKs, CDN config, `openai-php/*` or similar AI clients, a second DB connection in config.
- **Skill covers:** the specific concern of the category, grounded in the detected package and its wiring.

## General Rules

- One skill per confirmed integration, named for the concrete provider found (e.g. a Stripe skill, not a generic "payments" skill, if Stripe is what is wired).
- Ground the skill in `stack-researcher`'s official-doc notes for the exact detected version.
- Never include secret values; always read credentials from config/env at runtime in generated guidance.
