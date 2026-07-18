# Golden Principles

These principles guide implementation, review, and debugging in native PHP projects. They are framework-agnostic; framework-specific conventions belong in the matching sibling folder (`Laravel/`, `Symfony/`).

## 1. PSR And Explicit, Minimal Architecture First

Write modern PHP: `declare(strict_types=1)`, typed properties, parameters, and return types, PSR-1/PSR-12/PER style, and Composer PSR-4 autoloading.

Reach for the simplest structure that fits: a front controller, request handlers, use-case/service classes, domain objects, and data-access gateways. Add layers (interfaces, DTOs, repositories, event dispatchers) only when they reduce real complexity, not to imitate a framework.

## 2. Boundaries Must Be Explicit

Validate and authorize at system boundaries:

- HTTP requests: validate and normalize input into typed DTOs/value objects; authorize before acting.
- CLI commands: validate arguments and produce clear failure output and exit codes.
- Background workers: typed payloads and explicit retry/failure behavior.
- External APIs: typed clients with timeouts, retries, and error mapping.

Depend on interfaces at these boundaries and inject collaborators; avoid global state and hidden singletons.

## 3. Persistence Is A Contract

Schema changes belong in versioned migrations or reviewed SQL. Access data through PDO (or a documented data layer) using prepared statements with bound parameters.

When data integrity matters, prefer database constraints (keys, uniqueness, foreign keys) plus application validation over application validation alone.

## 4. Tests Should Prove Behavior

Use the smallest test that gives confidence:

- Unit tests for pure logic, services, and value objects.
- Integration/feature tests for HTTP handlers, validation, authorization, and persistence.
- Contract tests for external API clients and message workers.

Use fixtures, factories, and test doubles to keep tests readable and deterministic; isolate the database with transactions or a disposable test database.

## 5. Security Is Part Of The Design

Review changes for authentication, authorization, SQL injection, output escaping/XSS, CSRF, session handling, file upload, rate limiting, sensitive logging, unsafe deserialization, and secret handling.

Never rely on hidden UI controls as authorization.

## 6. Readability Beats Cleverness

Prefer clear names, small methods, explicit errors (typed exceptions), and simple control flow. If a future teammate needs project history to understand the code, simplify it or document the decision in specs.

## 7. Verification Is Evidence

Do not claim success without evidence. Run the applicable DoD checks, report what passed, and call out missing tooling or remaining risk.
