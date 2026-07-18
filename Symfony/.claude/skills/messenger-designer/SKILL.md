---
name: messenger-designer
description: Design Symfony Messenger messages, handlers, transports, retries, failure handling, idempotency, and tests.
phase: planning
flow-next: coder
flow-alternatives: [performance-optimization, writing-plans]
---

# Symfony Messenger Designer

Use for async/retryable workflows.

Specify:

- Message class and immutable payload.
- Handler that delegates to a service.
- Transport/routing choice.
- Retry strategy, failure transport, idempotency, and deduplication.
- Transaction boundary and dispatch timing.
- Observability and operational commands.
- Tests for handler behavior and failure paths.

## Design Workflow

1. Confirm why the work is asynchronous: latency, retries, fan-out, scheduling, isolation, or throughput. Keep synchronous work synchronous when a queue adds no operational value.
2. Use a small immutable message containing stable identifiers and primitive/value-object data, not managed Doctrine entities or sensitive payloads without a documented need.
3. Keep the handler thin and delegate one idempotent application workflow to a service.
4. Define routing, transport, serializer, retry strategy, delay/backoff, retryable versus unrecoverable exceptions, failure transport, and redelivery limits.
5. Define idempotency/deduplication storage and concurrency behavior. Assume at-least-once delivery unless the transport proves otherwise.
6. Avoid dispatch-before-commit races. Use the project's transaction middleware, post-commit pattern, or outbox when consistency requires it.
7. Specify worker time/memory limits, signals, deployment restart/drain, monitoring, alerting, failed-message inspection, replay, and retention.

Compare message/handler/service responsibilities with [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md). Keep transport concerns outside the use case and make at-least-once delivery behavior explicit.

## Verification

Test message serialization compatibility, handler delegation, duplicate delivery, retry classification, terminal failure, and transaction timing. Document payload backward compatibility for rolling deployments.
