# PHP Architecture Patterns Reference

How `architecture-scanner` detects each pattern and what `skill-forge` should do about it. Detection is always evidence-based; the implications guide the generated architecture skill.

## Monolith (single deployable)

- **Signals:** one `composer.json`, one autoload root, one entry point, no service-to-service messaging.
- **Generated architecture skill emphasizes:** internal layering discipline, module boundaries within one codebase, avoiding cross-layer leaks, keeping controllers thin.

## Modular Monolith

- **Signals:** one deployable but clear module namespaces (multiple PSR-4 roots or a `Modules/`/`src/<Context>/` layout), per-module boundaries, possibly an internal event bus.
- **Generated architecture skill emphasizes:** enforcing module boundaries, explicit inter-module contracts, preventing a module from reaching into another's internals, dependency direction rules.

## Microservices

- **Signals:** multiple deployables (multiple `composer.json`/service dirs), inter-service HTTP/RPC/messaging, per-service pipelines in CI.
- **Generated architecture skill emphasizes:** service boundaries and ownership, integration contracts between services, resilience (timeouts/retries), versioning of shared contracts, and treating other services as external integrations.

## Event-Driven

- **Signals:** a message/event bus (Messenger, a queue-backed job/event system), domain events, async handlers, outbox patterns.
- **Generated architecture skill emphasizes:** event/message contracts, idempotent handlers, ordering/at-least-once delivery realities, dead-letter handling, and observability of async flows.

## Layering / DDD (orthogonal to the above)

- **Layered:** presentation -> application -> domain -> infrastructure folders. Skill emphasizes dependency direction (inward only).
- **Hexagonal / Ports-and-Adapters:** `Domain/`, `Application/`, `Infrastructure/` (or `Ports/`/`Adapters/`). Skill emphasizes ports as interfaces, adapters at the edges, domain purity.
- **None detected:** do not impose DDD; generate a pragmatic skill matching the actual structure.

## General Rules

- Name the real modules/services/boundaries found in the target; do not invent an idealized structure.
- If the pattern is `inferred` or `unknown`, the generated architecture skill must say so and prompt the team to confirm rather than assert a structure that was not verified.
