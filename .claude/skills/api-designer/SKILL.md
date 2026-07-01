---
name: api-designer
description: Design Laravel REST APIs with routes, Form Requests, API Resources, policies, pagination, rate limits, error contracts, and OpenAPI documentation.
phase: planning
flow-next: frontend-design
flow-alternatives: [writing-plans, coder]
related: [architect, coder, documentation-generator]
---

# API Designer

## Overview

Design HTTP APIs that feel natural in Laravel and stable for clients. API design output should be implementable with `routes/api.php`, controllers, Form Requests, policies, Eloquent models, API Resources, and tests.

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

## Laravel Route Design

```php
use App\Http\Controllers\Api\InvitationController;
use Illuminate\Support\Facades\Route;

Route::middleware(['auth:sanctum', 'throttle:api'])->group(function (): void {
    Route::get('invitations', [InvitationController::class, 'index']);
    Route::post('invitations', [InvitationController::class, 'store']);
    Route::post('invitations/{invitation}/accept', [InvitationController::class, 'accept']);
});
```

## Request Contract

Use Form Requests for validation and, when appropriate, request-level authorization.

```php
class StoreInvitationRequest extends FormRequest
{
    public function authorize(): bool
    {
        return $this->user()?->can('create', Invitation::class) ?? false;
    }

    public function rules(): array
    {
        return [
            'email' => ['required', 'email:rfc,dns', 'max:255'],
            'role' => ['required', Rule::in(['trainer', 'player', 'parent'])],
            'expires_at' => ['nullable', 'date', 'after:now'],
        ];
    }
}
```

## Response Contract

Use API Resources for stable public JSON.

```php
class InvitationResource extends JsonResource
{
    public function toArray(Request $request): array
    {
        return [
            'id' => $this->id,
            'email' => $this->email,
            'role' => $this->role,
            'accepted_at' => $this->accepted_at?->toISOString(),
            'expires_at' => $this->expires_at?->toISOString(),
        ];
    }
}
```

## Pagination And Filtering

Prefer Laravel paginator responses when clients can accept the standard shape. If the API already has a custom envelope, document it and use it consistently.

```php
return InvitationResource::collection(
    Invitation::query()
        ->whereBelongsTo($request->user(), 'trainer')
        ->latest()
        ->paginate($request->integer('per_page', 15))
);
```

## Error Contract

Laravel validation errors normally return HTTP 422:

```json
{
  "message": "The email field is required.",
  "errors": {
    "email": ["The email field is required."]
  }
}
```

For domain errors, define a consistent shape:

```json
{
  "message": "Invitation has expired.",
  "code": "INVITATION_EXPIRED"
}
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

## OpenAPI Guidance

Use the project's chosen OpenAPI package. PHP attributes are preferred when available; annotations are acceptable if the project already uses them.

Document:

- Endpoint path and method.
- Auth requirements.
- Request schema.
- Response schema.
- Validation errors.
- Authorization and rate-limit behavior.

## Output Template

```markdown
## API Design: [Feature]

### Routes
| Method | Path | Controller | Auth | Purpose |

### Requests
- `StoreInvitationRequest`: validation and authorization rules

### Responses
- `InvitationResource`: stable fields and nullable behavior

### Errors
- 401, 403, 404, 409, 422, 429

### Tests
- Feature tests for success, validation, authorization, and missing resource cases
```

## Living Specification Update

Update `specs/api-designer-spec.md` when API behavior is added or changed. Also update `specs/MANIFEST.md`.

## Final Output

Return API routes, request/response contracts, validation/authorization notes, test plan, Context Summary, and next step.
