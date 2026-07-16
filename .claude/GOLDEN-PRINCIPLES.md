# Golden Principles - Symfony Layered Architecture

These principles guide implementation, review, and debugging in Symfony projects using Controller -> Service -> Repository.

## 1. Symfony Conventions First

Use Symfony's strengths before inventing custom infrastructure: routing attributes/config, dependency injection, Validator, Forms, Security voters, Messenger, Console, EventDispatcher, Serializer, Doctrine migrations, and Symfony testing tools.

Custom abstractions are justified when they remove real complexity, protect a boundary, or match existing project patterns.

## 2. Boundaries Must Be Explicit

Validate and authorize at system boundaries:

- HTTP controllers map request data, validate, authorize, call a service, and return a response.
- Console commands validate arguments/options, call services, and return correct exit codes.
- Messenger handlers accept typed messages, call services, and define retry/failure behavior.
- Event subscribers adapt framework events and delegate business decisions to services.
- External APIs are accessed through typed clients with timeouts, retries, and error mapping.

## 3. Services Own Workflows

Business workflows belong in services/use cases. A service should be understandable without HTTP, Twig, Messenger, or Console context.

Services may coordinate repositories, transactions, domain objects, voters/policy checks, messages, and external clients. They should not format HTTP responses or read raw request objects.

## 4. Persistence Is A Contract

Doctrine repositories own query shape, persistence helpers, and performance-sensitive data access. Controllers must not contain QueryBuilder/DQL/SQL.

When data integrity matters, enforce it with database constraints, indexes, foreign keys, optimistic/pessimistic locking, idempotency keys, or a documented transaction strategy. Application validation alone is not enough under concurrency.

## 5. Tests Should Prove Behavior

Use the smallest test that gives confidence:

- Unit tests for services, value objects, DTOs, and pure policies.
- Functional tests for controllers, validation, authorization, and API responses.
- Repository integration tests for Doctrine queries and persistence edge cases.
- Messenger/console tests for async and CLI workflows.

Use fixtures, factories, Foundry, object mothers, and builders to keep tests readable and deterministic.

## 6. Security Is Part Of The Design

Review every change for authentication, authorization, IDOR, validation, Doctrine injection, output escaping/XSS, CSRF, session handling, file uploads, rate limiting, sensitive logging, unsafe deserialization, SSRF, and secret handling.

Never rely on hidden UI controls as authorization.

## 7. Readability Beats Cleverness

Prefer clear names, small methods, guard clauses, typed DTOs, typed exceptions, and simple control flow. If a future teammate needs project history to understand the code, simplify it or document the decision in specs.

## 8. SOLID Is A Design Test, Not A Class Count

Use SOLID to protect cohesion, dependency direction, substitutable behavior, and narrow consumer contracts. Add interfaces for real variants, volatile infrastructure, or package ownership; do not create pass-through layers or one interface per class. Prefer composition, typed immutable contracts, explicit side effects, and business-readable names.

Use `examples/symfony-clean-code-patterns.md` as an illustrative review catalog, never as a reason to override a consuming project's simpler valid convention.

## 9. Verification Is Evidence

Do not claim success without evidence. Run the applicable DoD checks, report what passed, and call out missing tooling or remaining risk.
