# Golden Principles

These principles guide implementation, review, and debugging in Laravel projects. Framework-agnostic native-PHP conventions live in the sibling `PHP Core/` folder; this `Laravel/` folder specializes them for Laravel.

## 1. Laravel Conventions First

Follow the framework's own conventions before inventing abstractions: Eloquent models, Form Requests, Policies, Resources, migrations, and the standard `app/` structure. Write modern PHP underneath (`declare(strict_types=1)`, typed properties/parameters/returns, PSR-12 via Pint).

Add layers (Actions, Services, DTOs, repositories) only when they reduce real complexity for the specific feature, not to imitate a different architecture style. Laravel's "convention over configuration" is a feature — do not fight it without a concrete reason.

## 2. Boundaries Must Be Explicit

Validate and authorize at system boundaries:

- HTTP requests: validate via Form Requests; authorize via Policies/Gates before acting.
- Artisan commands: validate arguments, produce clear failure output and exit codes.
- Queued jobs: typed constructor payloads and explicit retry/failure (`$tries`, `failed()`).
- External APIs: `Http::` client with timeouts, retries, and error mapping, or a dedicated typed client class.

Bind abstractions to implementations in a Service Provider when a boundary needs to be swappable (e.g., a payment gateway); do not reach for interfaces everywhere by default.

## 3. Persistence Is A Contract

Schema changes belong in versioned Artisan migrations. Access data through Eloquent or the query builder; both parameterize bindings automatically. Never build SQL by string-concatenating request input, even with the query builder's raw methods.

When data integrity matters, prefer database constraints (keys, uniqueness, foreign keys) plus model/Form Request validation over validation alone. Use eager loading (`with()`, `load()`) to prevent N+1 queries.

## 4. Tests Should Prove Behavior

Use the smallest test that gives confidence:

- Unit tests for pure logic, Actions, and value objects.
- Feature tests for routes, Form Request validation, Policy authorization, and Eloquent persistence.
- Contract tests for external API clients and queued jobs.

Use model factories and `RefreshDatabase`/`DatabaseTransactions` to keep tests deterministic and isolated.

## 5. Security Is Part Of The Design

Review changes for authentication, authorization, SQL injection, output escaping/XSS (Blade escapes by default with `{{ }}`; never disable it for untrusted data), CSRF (`@csrf` on state-changing forms), session handling, file upload, rate limiting (`throttle` middleware), sensitive logging, unsafe deserialization, and secret handling.

Never rely on hidden UI controls as authorization; always assert Policy checks server-side.

## 6. Readability Beats Cleverness

Prefer clear names, small methods, explicit errors (typed/custom exceptions with an `Exception::report()`/`render()` or a handler mapping), and simple control flow. If a future teammate needs project history to understand the code, simplify it or document the decision in specs.

## 7. Verification Is Evidence

Do not claim success without evidence. Run the applicable DoD checks, report what passed, and call out missing tooling or remaining risk.
