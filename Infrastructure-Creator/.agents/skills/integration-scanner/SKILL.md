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

Read-only reconnaissance of a PHP target's third-party integrations. Evidence comes from `composer.json` `require` (NOT `require-dev`, unless a dev entry is clearly a runtime dependency) cross-referenced with runtime config wiring (config files, service registration, provider/bundle registration, env-var references). A package with matching runtime wiring is `confirmed`; a package present with no discoverable wiring is `inferred`. Non-PHP neighbors (e.g. a Node service, a managed database) are captured only as integration contracts, not as separate stacks.

The target project path is a **required** argument (e.g. "scan integrations of ../my-php-app"). Never assume the current working directory is the target. Operate strictly read-only within that path; never write to it, never read its `.env`/secrets (env-var *names* may be cited from config, but never their values).

## Generated File Naming Convention (MANDATORY)

Write exactly one findings file into the current run's task directory: `tasks/TASK-{N}/integration-scanner-findings.md`. Never write into the target.

## Process

1. **Read `composer.json` `require`.** Enumerate every non-`php`, non-`ext-*` runtime package as an integration candidate; exclude `require-dev` unless the package is clearly used at runtime.
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
3. **Find runtime wiring and bounded call sites** for each candidate: config file (`config/services.yaml`, `config/*.php`), provider/bundle registration, DI service definition, client instantiation, and at least one real request/response or producer/consumer call path. Cite bounded line ranges, symbols, or JSON pointers. Package/config presence may confirm installation, but only call-site evidence can support owned runtime behavior.
4. **Assign confidence:** `confirmed` = package + wiring both cited; `inferred` = package only; `unknown` = ambiguous signal (e.g. a generic HTTP client used for an unnamed API).
5. **Capture provider test topology and safety.** Find provider fakes, fixtures, mock transports, sandbox configuration names, contract/integration tests, and focused test commands without reading credentials. Record whether network execution is default-deny, what explicit environment/authorization would be required, and what rollback/sanitization boundary exists. Absence stays `unknown`; do not invent a safe sandbox.
6. **Map claims and material adjacency.** Separate behavior directly supported by target evidence from catalog review questions/external provider requirements. Record the primary provider-mechanics owner and every material adjacent owner (domain outcome, async reliability, local authorization, storage/cache correctness, security, or testing), with positive, negative, ambiguous, and cross-domain routing cases.
7. **Capture non-PHP neighbors as contracts.** When config references an external service without a PHP client (e.g. a base URL, a broker DSN), record it as an integration contract with its config citation, not as a PHP dependency.
8. **Mark confidence** per finding and never present a guess as fact.

## Output Template

```markdown
# Integration Scanner Findings: [target_name]

**Target:** [path]  **Scanned:** [date]

## Integrations by Category
### [Category, e.g. Payment]
- [package] - wiring/call sites: [bounded anchors] - supported claims: [...] - review questions/external requirements: [...] (confirmed/inferred)
- Test/safety: [fake/fixture/sandbox, focused command, network policy, authorization, rollback, sanitization]
- Adjacency/routing: [primary provider owner; material defer/fallback owners; positive/negative/ambiguous/cross-domain cases]

### [Next category ...]
- ...

## Integration Contracts (non-PHP neighbors)
- [service] - referenced at [config path:L#] (inferred/unknown)

## Uncategorized / Ambiguous
- [package or reference] (unknown - path:L#)

## Confidence Summary
[X confirmed, Y inferred, Z unknown]
```

## Guardrails

- MUST cite the composer package AND the runtime wiring path:line to mark an integration `confirmed`.
- MUST cite a real file path (and line where practical) for every finding.
- MUST operate read-only on the target; MUST NOT read `.env`/secrets (env-var names only, never values).
- MUST prefer `require` over `require-dev`; only include a dev entry when it is clearly a runtime dependency, and say why.
- MUST record non-PHP neighbors as integration contracts, never as PHP stacks.
- MUST NOT elevate catalog capabilities (timeouts, retries, deduplication, retention, model choice, or similar) into confirmed target behavior without call-site evidence.
- MUST capture provider-safe test topology and all material adjacent owners; a single generic sibling is insufficient.
- MUST NOT deep-dive stack identity, architecture, infra, security, or conventions - those belong to their own scanners.

## Final Output

Return the findings file path, the categorized integration list with per-item confidence, any integration contracts, and a one-line confidence summary. Suggest `stack-researcher` (to ground detected integrations in current provider docs) as the next step.
