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
- Every provider contract needs explicit decision points for retry vs. fail,
  authoritative vs. derived state, synchronous vs. asynchronous behavior, and
  safe degradation, limited to branches the target evidence actually supports.

The generated skill name identifies the concrete provider/mechanism. The contracts below are category shapes, not permission to generate a generic category skill. Required evidence always includes: runtime dependency, initialization/configuration path, at least one call site, exact-version official documentation URL, and a confirmed failure/runtime boundary. A package declaration alone fails the gate.

Compilation rules:

- A category “Skill covers” item is not confirmed target behavior. Require a bounded target anchor for each elevated claim; unsupported retry, timeout, deduplication, retention, model, webhook, audit, or degradation behavior remains a review question or external provider requirement.
- Bind provider verification to the scanned provider test topology: existing fake/fixture/local adapter first, then an evidenced sandbox only with explicit authorization and environment classification. Network/credential-backed execution is default-deny.
- Map each intersecting high-priority invariant ID to a procedure step and concrete safe assertion, including required and forbidden failure states.
- Carry exact command definitions, path authority, rollback/sanitization boundaries, and every material adjacent owner. Routing cases must cover positive provider mechanics, negative local/domain work, ambiguity, and cross-domain/provider requests.

## Enforceable Integration Contracts

### Payment provider
- **Trigger/scope:** confirmed client and payment/webhook call sites. Own provider API lifecycle, idempotency, webhook authenticity, error/retry and mode boundaries; exclude product pricing/refund policy.
- **Procedure:** map config/client; trace create/confirm/refund events used; assign idempotency keys; verify signed webhooks before parsing effects; map provider states to local authority; sanitize telemetry.
- **Verify/output/failure:** provider sandbox/fixtures, duplicate webhook, invalid signature, timeout and retry tests; output endpoint/event/state/error contract. Fail closed on signature/auth ambiguity.
- **Sibling/example:** domain skill owns business transition approval. Bad: copy provider quick-start and say “handle errors.”

### Messaging/queue provider
- **Trigger/scope:** confirmed producer, consumer, and transport config. Own provider delivery/ack/visibility/DLQ mechanics; exclude handler business idempotency already owned by `async-jobs`.
- **Procedure:** map topology; define serialization/version; configure visibility/ack/backoff/DLQ; correlate messages; define outage/backpressure behavior.
- **Verify/output/failure:** publish-consume, redelivery, poison, timeout and DLQ tests; output topology and runbook. Never assume exactly-once delivery.
- **Sibling/example:** `async-jobs` owns job anatomy/transaction boundaries. Bad: “retry three times” without provider semantics.

### Search provider
- **Trigger/scope:** confirmed client/index configuration plus indexing and query call sites. Own mapping, sync, relevance, reindex and provider failure; exclude source-of-truth domain writes.
- **Procedure:** identify authoritative records; define index/version/mapping; trace change propagation/deletion; construct bounded queries; plan zero-downtime reindex; define degraded behavior.
- **Verify/output/failure:** mapping, relevance fixture, drift/delete, reindex and outage tests; output index/query/sync contract. Never make the index authoritative without evidence.
- **Sibling/example:** domain/database skills own canonical data. Bad: generic search methods with no sync strategy.

### Cache provider
- **Trigger/scope:** confirmed provider adapter/config and runtime calls. Own provider connection, serialization, namespaces, cluster/outage behavior; exclude application invalidation design.
- **Procedure:** map clients/pools; define namespace/serialization/TTL limits; inspect atomic operations; handle connection failures; define observability and safe degradation.
- **Verify/output/failure:** connectivity, serialization, namespace, failover/outage tests; output provider operations contract. Do not silently convert provider failures into incorrect state.
- **Sibling/example:** `caching-strategy` owns keys/invalidation/stampede. Bad: duplicate cache-correctness guidance.

### Object-storage provider
- **Trigger/scope:** confirmed adapter/bucket/disk config and upload/read call sites. Own provider client/config, multipart/streaming, signed URL and service errors; exclude application file authorization/lifecycle.
- **Procedure:** map adapter/bucket/region/endpoint from config names only; stream/multipart; configure encryption/visibility; generate bounded URLs; classify retries and consistency.
- **Verify/output/failure:** fake/sandbox plus large stream, denied access, expired URL and outage tests; output provider boundary/runbook. Never expose credentials or default public access.
- **Sibling/example:** `file-storage` owns validation/tenant paths/retention. Bad: load every blob into memory.

### Email/SMS provider
- **Trigger/scope:** confirmed transport/client and send/webhook call sites. Own provider submission, templates/encoding boundary, delivery callbacks, suppression/rate failures; exclude multi-channel product routing.
- **Procedure:** map sender/config; sanitize recipient/content logging; submit idempotently where supported; process delivery/bounce/complaint callbacks; handle throttling/outage.
- **Verify/output/failure:** provider fixtures/sandbox for rendering, bounce, complaint, throttle and timeout; output event/error contract. Stop sends on suppression/privacy uncertainty.
- **Sibling/example:** `notification-delivery` owns channel selection/deduplication. Bad: log full message bodies and phone numbers.

### Auth/identity provider
- **Trigger/scope:** confirmed middleware/guard/client and token/callback paths. Own protocol/token validation, scopes, rotation/revocation and provider outage; exclude local object authorization.
- **Procedure:** map issuer/audience/redirect config; validate signature/claims/state/nonce; define token/session storage and refresh; map external identity to local principal; handle key rotation/logout.
- **Verify/output/failure:** invalid/expired/wrong-audience/replay/rotation/outage tests; output trust-boundary contract. Deny when validation or mapping is ambiguous.
- **Sibling/example:** `auth-scaffolding` owns local policies/voters. Bad: decode JWT without verification.

### Observability provider
- **Trigger/scope:** confirmed SDK/exporter/handler and emitted traces/errors/logs. Own instrumentation, correlation, sampling, redaction and provider failure; exclude root-cause methodology.
- **Procedure:** map initialization/environments; define trace/request/job correlation; capture actionable context with redaction; configure sampling; avoid recursive exporter failure; document retention authority if known.
- **Verify/output/failure:** emitted test event/span/log, async correlation, redaction and provider outage; output instrumentation/query map. Never send secrets/customer payloads.
- **Sibling/example:** `debugging` tells where to query; `systematic-debugger` tells how to investigate. Bad: “add more logs.”

### Feature-flag provider
- **Trigger/scope:** confirmed flag SDK/config and evaluation call sites. Owns provider evaluation, context, defaults, exposure and outage; excludes product rollout approval.
- **Procedure:** inventory actual flags/call sites; define stable typed defaults; minimize evaluation context; distinguish release/experiment/permission uses; handle stale/offline behavior; record ownership/retirement evidence.
- **Verify/output/failure:** on/off/default, targeting, stale-cache and outage tests; output flag contract and retirement checks. Never use a flag as authorization unless policy explicitly says so.
- **Sibling/example:** domain/security skills own business permission. Bad: “wrap code in a boolean flag.”

### CDN provider
- **Trigger/scope:** confirmed CDN/origin configuration and runtime asset/content routing. Owns provider cache keys, TTL, purge, origin and failure behavior; excludes application-level data caching.
- **Procedure:** map origins/routes; define cache key/vary rules; classify public/private content; configure TTL/revalidation; design purge/versioning; protect origin and define bypass.
- **Verify/output/failure:** hit/miss/vary/purge/private-content/origin-outage tests; output delivery/cache contract. Fail closed for private content when cache separation is uncertain.
- **Sibling/example:** `caching-strategy` owns in-app caches. Bad: cache authenticated HTML globally.

### ML/AI provider
- **Trigger/scope:** confirmed client/model config and prompt/inference call sites. Owns provider request/response, data minimization, model/version, nondeterminism, rate/error and safety boundaries; excludes product decisions made from output.
- **Procedure:** classify submitted data; minimize/redact; define structured input/output validation; pin/record model behavior where supported; bound time/cost/retries; handle refusal/malformed/unsafe output; require human review where evidenced.
- **Verify/output/failure:** fixture/contract, malformed, refusal, timeout, rate-limit and sensitive-data tests; output inference contract and fallback. Never trust model output as executable/authorized fact.
- **Sibling/example:** security/domain skills own authorization and business consequences. Bad: send raw customer records and parse free text blindly.

### Secondary database
- **Trigger/scope:** confirmed second connection/client and real read/write call sites. Owns connection, source authority, consistency, transaction and outage boundary; excludes schema design.
- **Procedure:** identify which store is authoritative for each datum; map synchronization/lag; delimit transactions and unsupported cross-store atomicity; define idempotent reconciliation; configure safe retries/pooling.
- **Verify/output/failure:** consistency/lag/reconciliation/partial-write/outage tests; output authority and failure contract. Stop when ownership between stores is ambiguous.
- **Sibling/example:** `database-designer` owns schema; domain skill owns business invariant. Bad: assume a distributed transaction exists.
