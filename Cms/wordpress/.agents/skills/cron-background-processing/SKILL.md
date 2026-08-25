---
name: cron-background-processing
description: Design WordPress scheduled and background work using WP-Cron, Action Scheduler, queues, or system cron with idempotency, locking, retries, observability, cleanup, and traffic-independent reliability decisions.
phase: planning
flow-next: coder
flow-alternatives: [wp-cli, performance-optimization, writing-plans]
related: [plugin-development, hooks-events, woocommerce]
---

# Cron and Background Processing

## Procedure

1. Define timeliness, volume, duration, retry, ordering, uniqueness, and
   observability requirements before selecting WP-Cron, Action Scheduler, a
   queue, WP-CLI plus system cron, or a host-specific worker.
2. Use WP-Cron only for delay-tolerant work. Document that traffic-driven
   spawning is not a delivery guarantee and recommend real cron invocation
   when the requirement needs predictable triggering.
3. Register custom schedules before scheduling, guard against duplicate
   events, schedule on controlled lifecycle, and unschedule the exact event on
   deactivation when ownership requires it.
4. Make handlers idempotent and bounded. Persist stable job identity/state,
   claim work atomically, process batches, enforce a time budget, and release
   locks safely. A timeout must not make retry unsafe.
5. Define retry/backoff, permanent-failure classification, dead-letter or
   operator recovery, structured safe logging, metrics, and cleanup/retention.
6. Do not hold database transactions or site switches across HTTP calls or
   slow work. Apply rate limits and SSRF/credential protections to outbound
   integrations.
7. Test duplicate dispatch, concurrent runners, retry after partial success,
   lock expiry, empty queue, poison item, multisite scope, and cleanup.

## Output

Return runner choice and guarantees, scheduling/unscheduling contract,
idempotency/locking/retry design, observability and recovery, tests, Context
Summary, and next step.
