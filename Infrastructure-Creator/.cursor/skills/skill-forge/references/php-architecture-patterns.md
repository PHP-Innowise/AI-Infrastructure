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

## Enforceable Architecture-Skill Contract

The architecture skill is evidence-gated, not automatically generated. Select it only when section 3 confirms or coherently infers meaningful boundaries/dependency direction and the target has repeated structure that warrants operational guidance. A single `src/` directory or framework package is insufficient.

- **Required evidence:** target-relative paths for deployable entry points, namespaces/modules/services, representative cross-boundary calls, dependency/wiring configuration, and any architecture ADR/tests. Record confidence and contradictions. For an inferred pattern, the necessity rationale must explain why a dedicated skill is useful despite uncertainty.
- **Owned scope:** place changes within existing modules/layers/services; enforce dependency direction and inter-boundary contracts; review boundary violations and architectural consequences.
- **Excluded scope:** feature scaffolding (`architecture-implementer`), generic code style, provider SDK mechanics, data schema design, and imposing an ideal DDD structure.
- **Procedure roles:** load the boundary map and authority; classify the requested change; identify owning boundary and allowed dependencies; trace inbound/outbound contracts; assess data/transaction/failure implications; propose the smallest architecture-conformant placement; verify actual dependency edges.
- **Decision points:** unknown owner -> stop and ask; cross-module behavior -> use explicit contract rather than internals; inferred pattern -> present evidence and confirmation checkpoint; conflicting ADR/code -> preserve conflict and use authority ranking.
- **Pattern branch:** monolith checks internal layers and controller/domain leakage; modular monolith checks module API and forbidden internal imports; microservices checks ownership, compatibility, timeout/retry and independent deployment; event-driven checks event schema, delivery semantics, idempotency, ordering, dead letters and observability. Apply only evidenced branches.
- **Verification:** use target tooling to inspect imports/dependencies/container wiring and run affected contract/architecture tests. When no automated architecture check exists, report a concrete manual dependency-edge review rather than inventing a command.
- **Output contract:** owning boundary, affected contracts, allowed/forbidden dependencies, placement decision, risks/unknowns, changed or proposed target-relative paths, and verification results.
- **Failure handling:** do not generate the skill when no meaningful architecture evidence exists; during use, stop on unresolved ownership or authority conflict instead of inventing a module.
- **Sibling boundaries:** `architecture-implementer` creates an approved skeleton; `api-designer` owns HTTP shape; integration skills own external providers. Reference only planned siblings.
- **Positive examples:** “Orders may call Billing through the cited application port, not import its infrastructure repository”; “service A changes a versioned event consumed by service B, so compatibility and replay tests are required.”
- **Negative generic example:** “Keep controllers thin, follow SOLID, use services, and run tests.” It contains no target boundary or decision procedure and is invalid.
