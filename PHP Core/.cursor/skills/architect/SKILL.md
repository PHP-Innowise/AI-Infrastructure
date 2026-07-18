---
name: architect
description: Make native PHP architecture decisions. Use when designing features, choosing module boundaries, layering, dependency direction, persistence, concurrency/async, scalability, or security trade-offs.
phase: planning
flow-next: api-designer
flow-alternatives: [architecture-implementer, writing-plans, coder]
related: [brainstorming, api-designer, architecture-implementer, coder]
---

# Architect

## Overview

Design features in plain PHP with the simplest structure that stays testable and changeable. Add layers only when they reduce real complexity.

This is the native-PHP base. Do not assume a framework. If the project uses one, prefer the matching sibling accelerator folder (`Laravel/`, `Symfony/`) for framework-native placement.

## Core Rule

Prefer standard, explicit PHP building blocks over ceremony:

- Front controller + router for HTTP entry points.
- Request DTOs / validators for input at the boundary.
- Use-case (application) classes for workflows that do not belong in controllers or entities.
- Domain entities and value objects for business rules and invariants.
- Repository/gateway interfaces with PDO implementations for persistence.
- PSR-15 middleware for cross-cutting HTTP concerns (auth, logging, CORS).
- PSR-11 container (or explicit manual wiring) for dependency injection.
- Interfaces at boundaries so infrastructure can be swapped and mocked.
- Message/queue workers for slow or side-effect-heavy operations.

## Layering And Dependency Direction

```
HTTP / CLI (entry)   ->  Application (use cases)  ->  Domain (entities, rules)
        |                        |                          ^
        v                        v                          |
   Infrastructure  <----  interfaces defined by Domain/Application
   (PDO, HTTP clients, queues, cache, filesystem)
```

Dependencies point inward. Domain knows nothing about the database or HTTP; infrastructure implements interfaces the inner layers declare.

## Decision Tree

```
New behavior
        |
        |-- HTTP input?
        |     |-- Route -> Controller/Handler
        |     |-- Request DTO / validator
        |
        |-- Authorization rule?
        |     |-- Access-control service or middleware
        |
        |-- Database-backed entity?
        |     |-- Migration (schema)
        |     |-- Entity + Repository interface
        |     |-- PDO repository implementation
        |
        |-- Multi-step business workflow?
        |     |-- Application/use-case class (+ transaction)
        |
        |-- Slow/external/side-effect operation?
              |-- Queue job / worker / event dispatch
```

## Pattern Guidance

| Situation | Preferred native PHP approach |
| --- | --- |
| Simple CRUD | Controller + Request DTO + Repository + response serializer |
| Complex write workflow | Controller + Request DTO + use-case class + PDO transaction |
| User-specific permissions | Access-control service checked in the use case or middleware |
| Reusable query logic | Repository methods or query objects |
| External API integration | Dedicated client behind an interface, with timeout and error mapping |
| Expensive side effect | Queue job dispatched after the transaction commits |
| Public API response | Serializer/DTO with a documented, stable JSON shape |
| Admin screens | Server-rendered templates or a project-standard admin approach |

## Transaction Boundaries

Use PDO transactions when multiple writes must succeed or fail together.

```php
$pdo->beginTransaction();

try {
    // multiple related writes
    $pdo->commit();
} catch (\Throwable $e) {
    $pdo->rollBack();

    throw $e;
}
```

Avoid holding a transaction open around slow external API calls. Persist intent first, then dispatch a worker after commit.

## Security Checklist

- Is the entry point authenticated?
- Is the action authorized in an explicit access-control layer?
- Is input validated and normalized at the boundary?
- Are queries parameterized (PDO prepared statements)?
- Is output escaped in templates (XSS)?
- Are state-changing web requests CSRF-protected?
- Are database constraints backing important invariants?
- Are file uploads validated and stored safely?
- Are secrets kept out of logs and source code?
- Are rate limits needed?
- Does the feature expose personal, payment, or minor data?

## Scalability Checklist

- Are indexes needed for new query patterns?
- Could N+1 queries appear? Batch or join instead.
- Should slow work be queued?
- Is cache invalidation clear (PSR-6/PSR-16)?
- Are external calls retried safely with timeouts?
- Does the design behave correctly under concurrent requests?

## When NOT To Add A Layer

Layers have a cost. Add one only when it earns its keep:

- Do not add a repository interface if there is exactly one implementation and no test seam benefit; use the query directly until a second reason appears.
- Do not add a service/use-case class for a pure passthrough; let the handler call the gateway.
- Do not introduce a DTO where a typed parameter is clearer.
- Do not build an event system for one synchronous caller.

Rule of thumb: introduce the abstraction on the *second* concrete need, not in anticipation of a hypothetical one (YAGNI). Prefer deleting a layer that no longer pays for itself.

## Ports And Adapters (Hexagonal-lite)

For anything touching the outside world (DB, HTTP clients, queue, cache, filesystem, clock), define the contract (port) in the inner layer and put the implementation (adapter) in Infrastructure. Benefits: the domain is testable with fakes, and vendors are swappable without touching business rules. Keep ports small and expressed in domain terms, not vendor terms.

## Concurrency And Idempotency

- Assume requests race. Protect invariants with database constraints (uniqueness) and locking (optimistic `version` column or `SELECT ... FOR UPDATE`), not just application checks.
- Make write operations idempotent where clients may retry (idempotency keys, upserts, or natural-key uniqueness).
- Guard critical sections that cross processes with an advisory/named lock or a queue with a single consumer, not an in-memory flag.
- Design workers to be at-least-once safe: the same message may be delivered twice.

## Failure And Resilience

- Set timeouts on every external call; add bounded retries with backoff only for idempotent operations.
- Consider a circuit breaker for a flaky dependency; degrade gracefully (cached/last-known value) where correctness allows.
- Persist intent before side effects; dispatch the side effect after commit so a rollback cannot leave an orphaned email/charge.

## Living Specification Update

Before final output, update `specs/architect-architecture.md` and `specs/MANIFEST.md` when architecture decisions changed.

If `specs/architect-architecture.md` does not exist, create it using `spec-desc.md` as the structure reference. Append task-specific sections with this shape:

```markdown
### [TASK-001] Invitation Registration (2026-06-29)

**Module:** Registration

**Placement:**
- Entry: `src/Http/Controller/RegistrationController.php`
- Request: `src/Http/Request/RegisterUserRequest.php`
- Use case: `src/Application/RegisterUser.php`
- Domain: `src/Domain/User.php`, `src/Domain/Invitation.php`
- Persistence: `src/Infrastructure/Persistence/PdoUserRepository.php`

**Decisions:**
- Validate input in a request DTO; authorize in an access-control service.
- Wrap registration writes in a PDO transaction.
- Dispatch the welcome-email job after commit.

**Risks:**
- Token guessing must be prevented with high-entropy invitation tokens.
```

## Final Output

Return:

- Architecture decision summary.
- Files/specs updated.
- Security and scalability considerations.
- Context Summary.
- Next by flow: `/api-designer`, `/architecture-implementer`, or `/writing-plans`.
