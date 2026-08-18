---
name: integration-scanner
description: Detect a PHP target's third-party integrations from composer.json require plus runtime config wiring, categorized (payment, messaging/queue, search, cache, object storage, email/SMS, auth/identity, observability, feature flags, CDN, ML/AI, secondary database) with confidence. Use as Phase 1 discovery input to stack-researcher. Triggers on "scan integrations", "what third-party services does this use", "detect the payment/queue/search provider", "integration-scanner".
phase: discovery
flow-next: stack-researcher
flow-alternatives: [profile-synthesizer]
related: [stack-scanner, architecture-scanner, infra-ops-scanner, security-compliance-scanner, conventions-scanner, infra-scan]
---

# Integration Scanner

## Overview

Read-only reconnaissance of a PHP target's third-party integrations. Candidates come from every source in the contract's integration-source checklist - `composer.json` `require` (NOT `require-dev`, unless a dev entry is clearly a runtime dependency), runtime config wiring, the implementing package resolved in `composer.lock`, and environment/deploy declarations - because a real project routinely runs providers that `require` never names. A candidate with cited runtime wiring is `confirmed`; a package or configured subsystem with no discoverable wiring is `inferred`. Non-PHP neighbors (e.g. a Node service, a managed database) are captured only as integration contracts, not as separate stacks.

The target project path is a **required** argument; never assume the current working directory is the target. Operate strictly read-only within it.

**Secrets rule** (identical in `security-compliance-scanner`, full text in the contract): never open, read, print, or fingerprint `.env`/`.env.*` or any credential store, and never record a value. Environment-variable *names* may be cited when they come from a non-secret committed source - `config/**`, container/CI config, deploy scripts, committed `.env.example`/`*.template` files, and `env()`/`getenv()`/`$_ENV` call sites.

## Outputs (MANDATORY)

Per run: exactly one report `tasks/TASK-{NNN}/integration-scanner-findings.md` and exactly one evidence ledger `tasks/TASK-{NNN}/integration-scanner-evidence.json`, both shaped by `stack-scanner/references/scan-evidence-contract.md` - read it first. Never write into the target.

## Process

1. **Enumerate candidates from every source in the contract's integration-source checklist, not from `require` alone.** Take every non-`php`, non-`ext-*` package from `composer.json` `require` (a `require-dev` entry only when clearly runtime, with the reason), then add the sources `require` cannot show: a subsystem configured in `config/bundles.php`, `config/packages/**`, or service/provider wiring (mailer DSN, messenger transport, storage/Flysystem adapter, JWT or other auth bundle, CORS, search or cache client) is a candidate even when the package implementing it arrives transitively through a meta-package or CMS bundle - resolve that implementing package in `composer.lock`; a DSN or base URL declared in environment/container/CI config or a deploy script is a candidate contract; a `package.json` service that calls an external provider is a candidate. "No integrations" is reportable only after all of these were searched, and the report must say which were searched.
2. **Categorize each candidate** using concrete PHP package signals:
   - Payment: `stripe/stripe-php`, `srmklive/paypal`.
   - Messaging/queue: `predis/predis`, `aws/aws-sdk-php` (SQS), `enqueue/*`, `php-amqplib/php-amqplib`.
   - Search: `elasticsearch/elasticsearch`, `meilisearch/meilisearch-php`, `algolia/algoliasearch-client-php`.
   - Cache: `predis/predis`, `symfony/cache`.
   - Object storage: `league/flysystem-aws-s3-v3`.
   - Email/SMS: `symfony/mailer`, `mailgun/mailgun-php`.
   - Auth/identity: `laravel/sanctum`, `laravel/passport`, `lexik/jwt-authentication-bundle`, `firebase/php-jwt`.
   - Observability: `sentry/sentry`, `open-telemetry/*`.
   - Feature flags, CDN, ML/AI, secondary database: category by package purpose.
   These package names are examples of each class, not its definition; a transitively installed or config-only provider belongs to the same category as its named peers.
3. **Find runtime wiring and bounded call sites** for each candidate: config file (`config/services.yaml`, `config/*.php`), provider/bundle registration, DI service definition, client instantiation, and at least one real request/response or producer/consumer call path. Cite bounded line ranges, symbols, or JSON pointers. Package/config presence may confirm installation, but only call-site evidence can support owned runtime behavior.
4. **Assign confidence:** `confirmed` = implementing package + wiring both cited; `inferred` = package or configuration alone; `unknown` = ambiguous signal (e.g. a generic HTTP client used for an unnamed API).
5. **Capture provider test topology and safety.** Find provider fakes, fixtures, mock transports, sandbox configuration names, contract/integration tests, and focused test commands without reading credentials. Record whether network execution is default-deny, what explicit environment/authorization would be required, and what rollback/sanitization boundary exists. Absence stays `unknown`; do not invent a safe sandbox.
6. **Map claims and material adjacency.** Separate behavior directly supported by target evidence from catalog review questions/external provider requirements. Record the primary provider-mechanics owner and every material adjacent owner (domain outcome, async reliability, local authorization, storage/cache correctness, security, or testing), with positive, negative, ambiguous, and cross-domain routing cases.
7. **Capture non-PHP neighbors as contracts.** When config references an external service without a PHP client (e.g. a base URL, a broker DSN), record it as an integration contract with its config citation, not as a PHP dependency.
8. **Mark confidence** per finding and never present a guess as fact.

## Report Structure

Follow the `integration-scanner` report template in appendix A of `stack-scanner/references/scan-evidence-contract.md`. Every factual line carries its confidence tag and its evidence id.

## Guardrails

- MUST cite the composer package AND the runtime wiring path:line to mark an integration `confirmed`.
- MUST cite a real file path (and line where practical) for every finding, and MUST emit both artifacts with contract-shaped evidence records (target-relative path, `sha256:` fingerprint, supported claims).
- MUST operate read-only on the target and follow the shared secrets rule: key names from non-secret committed sources only; never a value, and never `.env` itself.
- MUST search every source in the integration-source checklist before reporting an empty or short integration list; `require` alone is not a complete search.
- MUST prefer `require` over `require-dev`; only include a dev entry when it is clearly a runtime dependency, and say why.
- MUST record non-PHP neighbors as integration contracts, never as PHP stacks.
- MUST NOT elevate catalog capabilities (timeouts, retries, deduplication, retention, model choice, or similar) into confirmed target behavior without call-site evidence.
- MUST capture provider-safe test topology and all material adjacent owners; a single generic sibling is insufficient.
- MUST NOT deep-dive stack identity, architecture, infra, security, or conventions - those belong to their own scanners.

## Final Output

Return both artifact paths (report and evidence ledger), the categorized integration list with per-item confidence, any integration contracts, and a one-line confidence summary. Suggest `stack-researcher` (to ground detected integrations in current provider docs) as the next step.
