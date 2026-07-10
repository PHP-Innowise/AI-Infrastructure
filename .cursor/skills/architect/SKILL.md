---
name: architect
description: Make Laravel architecture decisions. Use when designing features, choosing between Actions/Services/model logic, Eloquent model boundaries, Service Provider bindings, queues/async, scalability, or security trade-offs.
phase: planning
flow-next: api-designer
flow-alternatives: [architecture-implementer, writing-plans, coder]
related: [brainstorming, api-designer, architecture-implementer, coder]
---

# Architect

## Overview

Design features using Laravel's own conventions with the simplest structure that stays testable and changeable. Add layers (Actions, Services, interfaces, events) only when they reduce real complexity — Laravel already gives you routing, validation, ORM, DI, and queues out of the box.

This branch targets Laravel (PHP 8.2+, 8.3+ required for Laravel 13). Supports Laravel 12 (current LTS) and Laravel 13 (current release). For framework-agnostic native PHP, use the `main` branch instead.

## Core Rule

Prefer Laravel's standard, explicit building blocks over custom ceremony:

- Routes (`routes/web.php` / `routes/api.php`) + Controllers for HTTP entry points.
- Form Requests for input validation and normalization at the boundary.
- Policies/Gates for authorization.
- Eloquent models for entities, relationships, and simple business rules (scopes, accessors, casts).
- Actions (single-purpose invokable classes) or Services (broader collaborators) for workflows that don't belong in a controller or a model.
- Service Providers to bind interfaces to implementations (e.g. a swappable payment gateway) and to wire up anything that needs bootstrapping.
- Jobs/Events/Listeners/Notifications for async or side-effect-heavy operations.
- Laravel's container (constructor injection, auto-resolved) for dependency injection — avoid manual `new` for anything bound in a provider.
- Interfaces at integration boundaries so external services can be swapped and mocked; do not add an interface for a single first-party Eloquent model with no second implementation.

## Layering And Dependency Direction

```
HTTP (routes/controllers)  ->  Actions / Services  ->  Eloquent Models (domain + persistence)
        |                              |                       ^
        v                              v                       |
   Form Requests / Policies      Jobs / Events        Service Provider bindings
                                                       (external clients, gateways)
```

Controllers stay thin and depend on Actions/Services and models. Actions/Services depend on interfaces for anything external (payment gateways, third-party APIs); Service Providers bind the concrete implementation. Eloquent models may hold simple business rules (scopes, casts, small domain methods) — Laravel does not require a separate "domain layer" the way a framework-agnostic app might.

## Decision Tree

```
New behavior
        |
        |-- HTTP input?
        |     |-- Route -> Controller (thin)
        |     |-- Form Request for validation
        |
        |-- Authorization rule?
        |     |-- Policy (model-scoped) or Gate (ad hoc) via $this->authorize()
        |
        |-- Database-backed entity?
        |     |-- Migration (schema)
        |     |-- Eloquent model + relationships
        |     |-- Factory + seeder for tests/dev data
        |
        |-- Multi-step business workflow?
        |     |-- Action (single use case) or Service (shared collaborator) + DB::transaction()
        |
        |-- Slow/external/side-effect operation?
              |-- Queued Job, or Event + Listener (queued), or Notification
```

## Pattern Guidance

| Situation | Preferred Laravel approach |
| --- | --- |
| Simple CRUD | Controller + Form Request + Eloquent model + API Resource |
| Complex write workflow | Controller + Form Request + Action/Service + `DB::transaction()` |
| Logic reused by several controllers/commands | Action or Service class, injected via the container |
| Simple derived data on a model | Eloquent accessor, mutator, or scope |
| User-specific permissions | Policy method checked via `$this->authorize()` or `can:` middleware |
| Complex role/permission matrix | `spatie/laravel-permission` roles/permissions instead of ad hoc Gate logic |
| External API integration | Client behind an interface, bound in a Service Provider, built on `Http::` with timeout/retry |
| Expensive side effect | Queued Job or queued Listener dispatched after the transaction commits |
| Public API response | API Resource / Resource Collection with a documented, stable JSON shape |
| Async notification (email, DB, Slack) | Notification class (queued) rather than manual `Mail::` calls scattered around |
| Admin screens | Filament (the default choice for most new admin panels; see `filament` skill), Nova, or hand-rolled Blade views if the project already standardizes on one |
| Multi-tenant SaaS | Single-database tenancy (`tenant_id` column + global Eloquent scope) by default; multi-database tenancy (`stancl/tenancy`) only when isolation/compliance/scaling needs justify the added ops cost |
| Live updates (chat, presence, dashboards, notifications) | `ShouldBroadcast` events + Laravel Echo over Laravel Reverb (first-party WebSocket server) by default; `wire:poll`/polling as a low-effort fallback for infrequent updates |

## Multi-Tenancy

Choose tenancy isolation deliberately — most B2B SaaS should start with the simpler option, not the heavier one:

- **Single-database tenancy (default):** a `tenant_id` foreign key on every tenant-owned table plus a global Eloquent scope (`static::addGlobalScope(new TenantScope)`) that filters every query to the current tenant. Simplest to build, migrate, and operate; sufficient for most B2B SaaS at moderate scale. Enforce the scope via a shared base model/trait, not ad hoc `where()` calls, so it can't be forgotten on a new model.
- **Multi-database tenancy** (a database per tenant, typically provisioned via `stancl/tenancy`): stronger isolation — a query-layer bug can't leak data across tenants — and the realistic option when compliance requirements or genuinely independent tenant scaling demand it. It adds real operational complexity: migrations must run per-tenant-database rather than once, and every request/job must correctly switch the active database (and cache/queue) connection before touching tenant data.
- Default to single-database tenancy; move to multi-database only when a specific isolation, compliance, or scaling requirement justifies the added complexity — never as a default.

## Real-Time And Live Updates

For chat, presence, live dashboards, or notifications pushed to the browser:

- Default to Laravel's own broadcasting stack: `Event` classes implementing `ShouldBroadcast` (or ad hoc `broadcast()` calls) paired with Laravel Echo on the frontend, served by **Laravel Reverb** — Laravel's first-party WebSocket server. Reverb is production-viable (run via Supervisor, scales horizontally via Redis pub/sub) and removes the need for a third-party service (Pusher, Ably) for most projects.
- Fall back to polling — or Livewire's `wire:poll` for a near-zero-effort implementation — only for low-frequency updates where a few seconds of staleness is acceptable and avoiding a persistent connection outweighs real-time latency.

## Actions vs Services vs Model Logic

- **Model logic** (scopes, accessors/mutators, casts, small relationship-derived methods) — for behavior that is intrinsically about *this entity's* data and doesn't orchestrate other models or side effects.
- **Actions** (a single `handle()`/`__invoke()` method, one clear use case) — for one workflow used from one or a few call sites (a controller, a command, a job). Easy to test in isolation, easy to name after the use case (`CreateUser`, `CancelOrder`).
- **Services** — for a cohesive set of related operations shared across many call sites, or wrapping a stateful/external collaborator (e.g. a `BillingService` wrapping a payment client). Prefer Actions by default; reach for a Service when several related Actions would otherwise duplicate a shared collaborator or setup.

Rule of thumb: start logic in the model; extract to an Action the moment it orchestrates multiple models, external calls, or a transaction; group Actions into a Service only once there are several that clearly share state or a collaborator.

## Transaction Boundaries

Use `DB::transaction()` when multiple writes must succeed or fail together.

```php
DB::transaction(function () use ($orderData): Order {
    $order = Order::create($orderData);
    $order->items()->createMany($orderData['items']);

    return $order;
});
```

`DB::transaction()` auto-commits on success and rolls back on any thrown exception (including deadlock retries via its optional `$attempts` argument). Avoid holding a transaction open around slow external API calls (`Http::` requests, third-party SDKs). Persist intent first, then dispatch a queued Job/Event after commit (`DB::afterCommit()` or dispatching from inside the transaction closure, which Laravel defers automatically for queued listeners marked `ShouldQueue`).

## Security Checklist

- Is the route behind the correct auth guard/middleware (`auth`, `auth:sanctum`)?
- Is the action authorized via a Policy or Gate, not just hidden UI?
- Is input validated by a Form Request (or explicit validator)?
- Are queries done through Eloquent/query builder with bound parameters (no raw string concatenation)?
- Is output escaped in Blade templates (`{{ }}` escapes by default; avoid `{!! !!}` on untrusted input)?
- Are state-changing web requests CSRF-protected (Laravel's `VerifyCsrfToken` middleware, already on by default for the `web` group)?
- Are database constraints (unique, foreign key) backing important invariants, not just app-level checks?
- Are file uploads validated (`mimes`/`mimetypes`, `max`) and stored via a non-public disk when appropriate?
- Are secrets kept out of logs and source code (`.env`, never committed)?
- Are rate limits needed (`throttle:` middleware)?
- Does the feature expose personal, payment, or minor data that needs extra protection?

## Scalability Checklist

- Are indexes needed for new query patterns (add via migration `$table->index()`/`unique()`)?
- Could N+1 queries appear? Eager-load with `with()`/`load()`, or use `withCount()`/`loadCount()` for counts.
- Should slow work be queued (`ShouldQueue` job/listener) instead of run inline?
- Is cache invalidation clear (`Cache::` facade, tagged cache where the driver supports it)?
- Are external calls retried safely with timeouts (`Http::timeout()->retry()`)?
- Does the design behave correctly under concurrent requests (see Concurrency below)?
- Would Horizon (queue monitoring) or Telescope (debugging) help observe this in practice?

### Deployment Considerations

- Zero-downtime deploys must invalidate and regenerate `config:cache`, `route:cache`, and `view:cache` post-deploy, not reuse caches built before the deploy — a stale cache can serve old config/routes/views.
- Use `php artisan down --secret=...` / `php artisan up` for planned maintenance windows that need a bypass link for verification before going fully live.
- Queued job payload formats can change across a Laravel major version — drain queues (or keep a compatible worker running through the cutover) during a major-version upgrade rather than mixing old-format payloads with a new-version worker.

## When NOT To Add A Layer

Layers have a cost. Add one only when it earns its keep:

- Do not add an interface/binding for an Eloquent model or a single first-party implementation with no second implementation or test-seam benefit.
- Do not add an Action/Service class for a pure passthrough; let the controller call the model directly.
- Do not introduce a DTO where the Form Request's `validated()` array or a typed parameter is clearer.
- Do not dispatch an Event for one synchronous listener with no other subscriber — call the method directly.
- Do not reach for `spatie/laravel-data` or a custom DTO layer until plain arrays/Form Requests stop being clear enough.

Rule of thumb: introduce the abstraction on the *second* concrete need, not in anticipation of a hypothetical one (YAGNI). Prefer deleting a layer that no longer pays for itself.

## Interfaces And Service Provider Bindings

For anything touching the outside world beyond Eloquent's own persistence (payment gateways, third-party APIs, external file stores), define a small interface and bind the concrete implementation in a Service Provider:

```php
// app/Providers/AppServiceProvider.php
public function register(): void
{
    $this->app->bind(PaymentGateway::class, StripePaymentGateway::class);
}
```

This keeps Actions/Services testable with a fake/mock binding in tests and lets the concrete vendor be swapped without touching business logic. Keep interfaces small and expressed in domain terms, not vendor terms.

## Concurrency And Idempotency

- Assume requests race. Protect invariants with database constraints (`unique()` in a migration) and locking — optimistic (a `version`/`updated_at` check) or pessimistic (`lockForUpdate()` inside a `DB::transaction()`) — not just application checks.
- Make write operations idempotent where clients may retry (an `Idempotency-Key` stored per request, `updateOrCreate()`/`firstOrCreate()` upserts, or natural-key uniqueness).
- Guard critical sections that cross processes with `Cache::lock()` (atomic lock via the cache driver) or a queue with a single consumer, not an in-memory flag.
- Design queued jobs to be at-least-once safe: the same job may run twice. Make `handle()` idempotent, or use `WithoutOverlapping` / unique job IDs (`ShouldBeUnique`) to prevent duplicate dispatch.

## Failure And Resilience

- Set timeouts on every external call (`Http::timeout(5)`); add bounded retries with backoff only for idempotent operations (`Http::retry(3, 100)`).
- For queued jobs, configure `$tries`, `backoff()` (fixed or exponential array), and a `failed(Throwable $exception)` method to handle permanent failure (alert, compensating action, dead-letter record).
- Consider a circuit breaker for a flaky dependency; degrade gracefully (cached/last-known value) where correctness allows.
- Persist intent before side effects; dispatch the queued job/event after the transaction commits so a rollback cannot leave an orphaned email/charge.

## Living Specification Update

Before final output, update `specs/architect-architecture.md` and `specs/MANIFEST.md` when architecture decisions changed.

If `specs/architect-architecture.md` does not exist, create it using `spec-desc.md` as the structure reference. Append task-specific sections with this shape:

```markdown
### [TASK-001] Invitation Registration (2026-06-29)

**Module:** Registration

**Placement:**
- Route: `routes/api.php` (`POST /api/invitations`)
- Request: `app/Http/Requests/StoreInvitationRequest.php`
- Action: `app/Actions/RegisterInvitedUser.php`
- Models: `app/Models/User.php`, `app/Models/Invitation.php`
- Policy: `app/Policies/InvitationPolicy.php`

**Decisions:**
- Validate input in a Form Request; authorize via `InvitationPolicy`.
- Wrap registration writes in `DB::transaction()`.
- Dispatch the welcome-email Notification after commit.

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
