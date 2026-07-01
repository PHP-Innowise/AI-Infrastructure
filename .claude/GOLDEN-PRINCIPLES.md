# Golden Principles

These principles guide implementation, review, and debugging in Laravel/PHP projects.

## 1. Laravel Conventions First

Use the framework shape before adding custom architecture. Routes, controllers, form requests, Eloquent models, policies, API resources, jobs, events, notifications, migrations, factories, and seeders are first-class Laravel tools.

Add services, actions, repositories, DTOs, or domain modules when they reduce real complexity. Do not add them only to imitate another ecosystem.

## 2. Boundaries Must Be Explicit

Validate and authorize at system boundaries:

- HTTP requests: Form Requests, validators, middleware, policies, gates.
- Console commands: argument validation and clear failure output.
- Jobs/listeners: typed payloads and retry/failure behavior.
- External APIs: typed clients, timeouts, retries, and error mapping.

## 3. Persistence Is A Contract

Schema changes belong in migrations. Model behavior must respect mass assignment, casts, relationships, scopes, and database constraints.

When data integrity matters, prefer database constraints plus application validation over application validation alone.

## 4. Tests Should Prove Behavior

Use the smallest test that gives confidence:

- Unit tests for pure logic and focused services/actions.
- Feature tests for HTTP behavior, validation, authorization, and persistence.
- Integration tests for queues, events, external API wrappers, and complex database behavior.

Use factories and fakes to keep tests readable and deterministic.

## 5. Security Is Part Of The Design

Review changes for authentication, authorization, injection, mass assignment, file upload, session/CSRF, rate limiting, sensitive logging, and secret handling risks.

Never rely on hidden UI controls as authorization.

## 6. Readability Beats Cleverness

Prefer clear names, small methods, explicit errors, and simple control flow. If a future teammate needs project history to understand the code, simplify it or document the decision in specs.

## 7. Verification Is Evidence

Do not claim success without evidence. Run the applicable DoD checks, report what passed, and call out missing tooling or remaining risk.
