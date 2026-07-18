---
name: api-designer
description: Design Laravel REST APIs with routes/api.php routing, Form Request validation, API Resources/Resource Collections, Sanctum authorization, pagination, rate limits, error contracts, and OpenAPI documentation.
phase: planning
flow-next: frontend-design
flow-alternatives: [writing-plans, architecture-implementer, coder]
related: [architect, coder, documentation-generator, auth-scaffolding, eloquent]
---

# API Designer

## Overview

Design HTTP APIs that are natural to implement with Laravel and stable for clients. Output should be implementable with `routes/api.php`, controllers, Form Requests, Policies/Sanctum for authorization, Eloquent models, API Resources, and tests.

## REST Conventions

Use plural resources, matching Laravel's `Route::apiResource()` naming:

```
GET    /api/users
POST   /api/users
GET    /api/users/{user}
PATCH  /api/users/{user}
DELETE /api/users/{user}
```

Use action endpoints sparingly for state transitions that are not CRUD:

```
POST /api/invitations/{invitation}/accept
POST /api/orders/{order}/cancel
POST /api/events/{event}/publish
```

## Routing (routes/api.php)

```php
use App\Http\Controllers\Api\InvitationController;

Route::middleware(['auth:sanctum', 'throttle:60,1'])->group(function (): void {
    Route::apiResource('invitations', InvitationController::class)->only(['index', 'store']);
    Route::post('invitations/{invitation}/accept', [InvitationController::class, 'accept']);
});
```

`Route::apiResource()` wires the conventional CRUD verbs automatically and supports route-model binding (`{invitation}` resolves to an `Invitation` model, 404ing if missing). Apply authentication (`auth:sanctum`), throttling (`throttle:`), and any custom middleware as route/group middleware rather than reimplementing them.

For the one-off action endpoints above (`accept`, `cancel`, `publish`), a method on the resource controller is fine by default. Once one of those actions grows enough of its own validation/authorization/side-effect logic to feel crowded on the resource controller, extract it into a single-action invokable controller (`__invoke()`) instead, routed as `Route::post('invitations/{invitation}/accept', AcceptInvitationController::class)` — the same "one clear use case" reasoning `architect` applies when choosing between a Service and an Action.

## Authentication Strategy

Pick the lightest option that satisfies the actual client requirements:

| Option | Use when |
| --- | --- |
| **Sanctum** (`laravel/sanctum`) | Default choice. First-party SPA/mobile clients you control; simple API token or cookie-session auth; per-token ability scoping (`createToken('name', ['orders:read'])`) and revocation. Lowest setup and operational cost. |
| **Passport** (`laravel/passport`) | Only when you must support third-party OAuth2 clients: delegated authorization ("Login with [App]" for other apps), authorization-code/PKCE flows, or client-credentials machine-to-machine grants with real OAuth2 semantics. Heavier (OAuth2 server, token/refresh-token tables) than most projects need — don't reach for it just because it's "more standard". |
| **Stateless JWT** (e.g. `tymon/jwt-auth`) | Only when the API must be fully stateless across services with no shared session/database access (e.g. verified independently by multiple microservices). Flag the operational cost up front: you take on manual token rotation, revocation/blocklisting, and refresh-token handling that Sanctum and Passport otherwise give you for free. |

This table covers token-based API authentication only. For web/session login, registration, starter kits (Breeze/Jetstream/Fortify), or Policy/Gate authorization, use the `auth-scaffolding` skill instead.

## Request Contract

Validate and normalize input with a Form Request; authorize in the same class via `authorize()` or via a Policy called from the controller.

```php
<?php

declare(strict_types=1);

namespace App\Http\Requests;

use Illuminate\Foundation\Http\FormRequest;
use Illuminate\Validation\Rule;

final class StoreInvitationRequest extends FormRequest
{
    public function authorize(): bool
    {
        return $this->user()?->can('create', \App\Models\Invitation::class) ?? false;
    }

    /** @return array<string, mixed> */
    public function rules(): array
    {
        return [
            'email' => ['required', 'email', 'max:255'],
            'role' => ['required', Rule::in(['trainer', 'player', 'parent'])],
            'expires_at' => ['nullable', 'date', 'after:now'],
        ];
    }
}
```

For a handful of fields, pass `$this->validated()` (or a typed method parameter) straight into the Action/Service — it stays clear enough on its own. Once a request has enough fields that an untyped array becomes hard to follow (and array-key typos become a real risk), add a `toDto()` method that maps `validated()` into a typed value object instead of pushing the raw array further down the call stack:

```php
public function toDto(): InvitationData
{
    return InvitationData::from($this->validated());
}
```

`spatie/laravel-data` (see `dependency-manager`) is the common fit for the `InvitationData` object above. Per `architect`'s guidance, introduce this on the second concrete need (a genuinely large or reused payload), not as a default on every Form Request — most single-purpose requests are clearer left as a plain array.

## Response Contract

Use an API Resource for stable public JSON instead of returning Eloquent models directly.

```php
<?php

declare(strict_types=1);

namespace App\Http\Resources;

use Illuminate\Http\Request;
use Illuminate\Http\Resources\Json\JsonResource;

final class InvitationResource extends JsonResource
{
    /** @return array<string, mixed> */
    public function toArray(Request $request): array
    {
        return [
            'id' => $this->id,
            'email' => $this->email,
            'role' => $this->role,
            'accepted_at' => $this->accepted_at?->toAtomString(),
            'expires_at' => $this->expires_at?->toAtomString(),
        ];
    }
}
```

Use a Resource Collection (`InvitationCollection`, or `InvitationResource::collection($invitations)`) for list endpoints so pagination metadata is included consistently.

## Pagination And Filtering

Default to `cursorPaginate()` for list endpoints, not `paginate()`. Offset pagination (`paginate()`) looks harmless on a small local/seeded dataset, which is exactly why it is easy to default to by mistake — it degrades on large tables (`OFFSET` scans get slower the deeper you page) and can skip or duplicate rows when data shifts between requests. Treat that as a real correctness/scalability cost to weigh in, not just an option reserved for tables you already know are huge. Return it via a Resource Collection rather than hand-rolling the envelope:

```php
return InvitationResource::collection(
    Invitation::query()->latest()->cursorPaginate($request->integer('per_page', 15))
);
```

```json
{
  "data": [],
  "links": { "first": null, "last": null, "prev": "...", "next": "..." },
  "meta": { "path": "...", "per_page": 15, "next_cursor": "...", "prev_cursor": null }
}
```

Reach for offset pagination (`paginate()`) instead only when there is a genuine, deliberate reason a cursor can't satisfy — a client-facing numbered pager ("page 3 of 10") or a required total row count:

```php
return InvitationResource::collection(
    Invitation::query()->latest()->paginate($request->integer('per_page', 15))
);
```

```json
{
  "data": [],
  "links": { "first": "...", "last": "...", "prev": null, "next": "..." },
  "meta": { "current_page": 1, "per_page": 15, "total": 42 }
}
```

Translate query parameters (filters, sort) into scoped Eloquent query builder calls with validated, whitelisted column names — never interpolate raw request input into `orderBy()`/`where()` column names.

## Error Contract

Laravel's default JSON error responses already follow a predictable shape; keep custom exceptions consistent with it. Validation failures return HTTP 422 automatically from a Form Request:

```json
{
  "message": "The email field is required.",
  "errors": { "email": ["The email field is required."] }
}
```

Domain errors should map to a stable code via a custom exception handled in `bootstrap/app.php`'s `->withExceptions()` (or `Handler::render()` pre-Laravel-11):

```json
{ "message": "Invitation has expired.", "code": "INVITATION_EXPIRED" }
```

## Status Codes

| Scenario | Status |
| --- | --- |
| Successful read | 200 |
| Created | 201 |
| Accepted async work | 202 |
| No response body | 204 |
| Validation failure | 422 |
| Unauthenticated | 401 |
| Unauthorized | 403 |
| Missing resource | 404 |
| Conflict | 409 |
| Rate limited | 429 |

## Versioning

- Version from day one for public/cross-team APIs. Prefer a URI prefix (`/api/v1/...`), implemented as a route group/namespace in `routes/api.php` (or a separate `routes/api_v2.php` included conditionally), for its simplicity and cache-friendliness. Header-based versioning (`Accept: application/vnd.app.v1+json`) is an option when you must keep URLs stable.
- Never make a breaking change within a version. Additive changes (new optional fields on an API Resource, new endpoints) are safe; removing/renaming fields or tightening Form Request validation is breaking.
- Document a deprecation policy: mark deprecated fields, announce, then remove only in the next major version. Send a `Deprecation`/`Sunset` header where useful.

## Idempotency

- GET, PUT, DELETE must be idempotent; POST is not. For POST that creates resources or charges money, support an `Idempotency-Key` header: store the key + response (e.g. in a dedicated table or cache), and return the stored response on retries within a TTL.
- Design retries to be safe: clients will retry on timeouts, so a duplicate request must not double-charge or double-create. Pair with `firstOrCreate()`/`updateOrCreate()` or a unique constraint where a natural key exists.

## Content Negotiation

- Honor `Accept`; default to `application/json`. Laravel's `Request::expectsJson()` and API-focused controllers already assume JSON — keep API routes under `routes/api.php` free of session/CSRF assumptions (they get the `api` middleware group, which is stateless by default).
- Set `Content-Type` on responses explicitly when returning non-standard payloads; API Resources default to `application/json` with UTF-8.

## OpenAPI Guidance

Use the project's chosen OpenAPI approach:

- `dedoc/scramble` — generates OpenAPI docs automatically from routes, Form Requests, and API Resources with minimal annotation; good default for new Laravel APIs.
- `darkaonline/l5-swagger` — wraps `zircote/swagger-php` annotations for teams that prefer explicit PHP attribute-based documentation.
- A hand-written `openapi.yaml` for full manual control.

Document path and method, auth requirements (`Sanctum` bearer token or session cookie), request schema (from the Form Request), response schema (from the API Resource), validation errors, and rate-limit behavior.

## Output Template

```markdown
## API Design: [Feature]

### Routes
| Method | Path | Controller@action | Auth | Purpose |

### Requests
- `StoreInvitationRequest`: validation and authorization rules

### Responses
- `InvitationResource`: stable fields and nullable behavior

### Errors
- 401, 403, 404, 409, 422, 429

### Tests
- Feature tests for success, validation, authorization, and missing-resource cases
```

## Living Specification Update

Update `specs/api-designer-spec.md` when API behavior is added or changed. Also update `specs/MANIFEST.md`.

## Final Output

Return API routes, request/response contracts, authorization notes (Sanctum/Policies), test plan, Context Summary, and next step (`/frontend-design`, `/writing-plans`, `/architecture-implementer`, or `/coder`).
