---
name: event-subscriber-designer
description: Design Symfony event subscribers/listeners as thin adapters that delegate business workflows to services.
phase: planning
flow-next: coder
flow-alternatives: [architecture-boundary-reviewer, writing-plans]
---

# Symfony Event Subscriber Designer

Use subscribers for framework integration, not hidden business workflows.

Specify:

- Event name/class.
- Priority and ordering assumptions.
- Service method delegated to.
- Idempotency and side effects.
- Error behavior and observability.
- Tests proving the subscriber wires the event to the service.

## Decision Rules

- Use a subscriber for framework lifecycle adaptation or cross-cutting integration, not to conceal a primary use case.
- Prefer explicit service calls when ordering is business-critical or the caller requires a result.
- Distinguish Symfony framework events, Doctrine lifecycle events, and domain/application events; they have different transaction and failure semantics.
- Avoid network calls or long-running work inside request-critical events. Dispatch a Messenger message after durable state is available when asynchronous handling is appropriate.
- Do not mutate entities in Doctrine post-flush hooks or trigger recursive flush cycles.

Compare explicit service calls and thin subscriber adapters with [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md). An event must reduce coupling without making business ordering, failure, or transaction semantics invisible.

## Output

Specify event type, subscriber method, priority, input mapping, delegated service, transaction timing, idempotency, failure policy, logging/metrics, registration/autoconfiguration, and unit/integration tests. Call out ordering dependencies as risks rather than relying on undocumented priorities.
