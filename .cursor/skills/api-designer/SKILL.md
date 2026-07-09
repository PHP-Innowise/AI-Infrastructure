---
name: api-designer
description: Design native PHP REST APIs with routing, PSR-7 requests/responses, input validation, response serializers/DTOs, authorization, pagination, rate limits, error contracts, and OpenAPI documentation.
phase: planning
flow-next: frontend-design
flow-alternatives: [writing-plans, architecture-implementer, coder]
related: [architect, coder, documentation-generator]
---

# API Designer

## Overview

Design HTTP APIs that are natural to implement in plain PHP and stable for clients. Output should be implementable with a router, controllers/handlers, PSR-7 messages, request validators, an access-control layer, PDO repositories, response serializers, and tests.

## REST Conventions

Use plural resources:

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

## Routing (framework-agnostic)

```php
$dispatcher = FastRoute\simpleDispatcher(function (FastRoute\RouteCollector $r): void {
    $r->addGroup('/api', function (FastRoute\RouteCollector $r): void {
        $r->addRoute('GET', '/invitations', [InvitationController::class, 'index']);
        $r->addRoute('POST', '/invitations', [InvitationController::class, 'store']);
        $r->addRoute('POST', '/invitations/{invitation:\d+}/accept', [InvitationController::class, 'accept']);
    });
});
```

Apply authentication, throttling, and CORS as PSR-15 middleware around the dispatcher.

## Request Contract

Validate and normalize input into a typed DTO at the boundary; authorize before acting.

```php
<?php

declare(strict_types=1);

namespace App\Http\Request;

final class StoreInvitationRequest
{
    private const ROLES = ['trainer', 'player', 'parent'];

    public function __construct(
        public readonly string $email,
        public readonly string $role,
        public readonly ?string $expiresAt,
    ) {
    }

    /** @param array<string, mixed> $input */
    public static function fromArray(array $input): self
    {
        $errors = [];

        $email = trim((string) ($input['email'] ?? ''));
        if (filter_var($email, FILTER_VALIDATE_EMAIL) === false) {
            $errors['email'] = 'A valid email is required.';
        }

        $role = (string) ($input['role'] ?? '');
        if (! in_array($role, self::ROLES, true)) {
            $errors['role'] = 'Role must be one of: ' . implode(', ', self::ROLES) . '.';
        }

        if ($errors !== []) {
            throw ValidationException::withErrors($errors);
        }

        return new self($email, $role, $input['expires_at'] ?? null);
    }
}
```

## Response Contract

Use a serializer/DTO for stable public JSON instead of dumping entities directly.

```php
final class InvitationResponse
{
    /** @return array<string, mixed> */
    public static function fromEntity(Invitation $invitation): array
    {
        return [
            'id' => $invitation->id,
            'email' => $invitation->email,
            'role' => $invitation->role,
            'accepted_at' => $invitation->acceptedAt?->format(DATE_ATOM),
            'expires_at' => $invitation->expiresAt?->format(DATE_ATOM),
        ];
    }
}
```

## Pagination And Filtering

Document a consistent envelope and use it everywhere:

```json
{
  "data": [],
  "meta": { "page": 1, "per_page": 15, "total": 42 }
}
```

Translate query parameters (`page`, `per_page`, filters) into bounded, validated SQL with `LIMIT`/`OFFSET` (or keyset pagination for large tables).

## Error Contract

Return a predictable shape. Validation failures use HTTP 422:

```json
{
  "message": "Validation failed.",
  "errors": { "email": ["A valid email is required."] }
}
```

Domain errors use a stable code:

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

- Version from day one for public/cross-team APIs. Prefer a URI prefix (`/api/v1/...`) for its simplicity and cache-friendliness; header-based versioning (`Accept: application/vnd.app.v1+json`) is an option when you must keep URLs stable.
- Never make a breaking change within a version. Additive changes (new optional fields, new endpoints) are safe; removing/renaming fields or tightening validation is breaking.
- Document a deprecation policy: mark deprecated fields, announce, then remove only in the next major version. Send a `Deprecation`/`Sunset` header where useful.

## Idempotency

- GET, PUT, DELETE must be idempotent; POST is not. For POST that creates resources or charges money, support an `Idempotency-Key` header: store the key + response, and return the stored response on retries within a TTL.
- Design retries to be safe: clients will retry on timeouts, so a duplicate request must not double-charge or double-create.

## Content Negotiation

- Honor `Accept`; default to `application/json`. Return `406` for unsupported types and `415` for an unsupported request `Content-Type`.
- Set `Content-Type` on responses explicitly; use UTF-8.

## Cursor vs Offset Pagination

- Offset/`LIMIT` is simple but degrades on large tables and can skip/duplicate rows when data shifts.
- Prefer keyset/cursor pagination (`WHERE id > :last_id ORDER BY id LIMIT :n`, returning an opaque `next_cursor`) for large or frequently changing collections.

## OpenAPI Guidance

Use the project's chosen OpenAPI approach (PHP attributes/annotations via `zircote/swagger-php`, or a hand-written `openapi.yaml`). Document path and method, auth requirements, request schema, response schema, validation errors, and rate-limit behavior.

## Output Template

```markdown
## API Design: [Feature]

### Routes
| Method | Path | Handler | Auth | Purpose |

### Requests
- `StoreInvitationRequest`: validation and normalization rules

### Responses
- `InvitationResponse`: stable fields and nullable behavior

### Errors
- 401, 403, 404, 409, 422, 429

### Tests
- Integration tests for success, validation, authorization, and missing-resource cases
```

## Living Specification Update

Update `specs/api-designer-spec.md` when API behavior is added or changed. Also update `specs/MANIFEST.md`.

## Final Output

Return API routes, request/response contracts, validation/authorization notes, test plan, Context Summary, and next step (`/frontend-design`, `/writing-plans`, `/architecture-implementer`, or `/coder`).
