---
name: architect
description: Make Laravel architecture decisions. Use when designing features, choosing module boundaries, evaluating authorization, persistence, queues, scalability, or security trade-offs.
phase: planning
flow-next: api-designer
flow-alternatives: [writing-plans, coder]
related: [brainstorming, api-designer, coder]
---

# Architect

## Overview

Design Laravel features using Laravel-native conventions first. Add extra layers only when they make the code easier to reason about, test, or change.

## Core Rule

Do not translate patterns from another ecosystem mechanically. Laravel already provides strong architectural primitives:

- Routes and controllers for HTTP entry points.
- Form Requests for validation and request authorization.
- Policies and gates for authorization.
- Eloquent models, migrations, factories, seeders, casts, scopes, and relationships for persistence.
- API Resources for stable response contracts.
- Jobs, queues, events, listeners, mailables, and notifications for asynchronous or side-effect work.
- Services or actions for business workflows that do not belong in controllers or models.
- Repositories only when they isolate a real persistence complexity or external data source.

## Decision Tree

```
New behavior
        |
        |-- HTTP input?
        |     |-- routes/api.php or routes/web.php
        |     |-- Controller
        |     |-- Form Request
        |
        |-- Authorization rule?
        |     |-- Policy or Gate
        |
        |-- Database-backed entity?
        |     |-- Migration
        |     |-- Eloquent Model
        |     |-- Factory for tests
        |
        |-- Multi-step business workflow?
        |     |-- Service or Action
        |
        |-- Slow/external/side-effect operation?
              |-- Job, Event, Listener, Notification, or Mail
```

## Common Laravel Module Shape

Use this as a starting point, not as a mandatory folder explosion:

```
app/
├── Http/
│   ├── Controllers/
│   ├── Requests/
│   └── Resources/
├── Models/
├── Policies/
├── Services/        # when workflows need a home
├── Actions/         # when command-style use cases are clearer
├── Jobs/
├── Events/
└── Listeners/

database/
├── migrations/
├── factories/
└── seeders/

routes/
├── api.php
└── web.php
```

## Pattern Guidance

| Situation | Preferred Laravel approach |
| --- | --- |
| Simple CRUD | Controller + Form Request + Model + Resource |
| Complex write workflow | Controller + Form Request + Service/Action + transaction |
| User-specific permissions | Policy methods and `authorize()` calls |
| Reusable query filters | Eloquent scopes or query objects |
| External API integration | Dedicated client/service with timeout and error mapping |
| Expensive side effect | Job queued after commit |
| Public API response | API Resource and documented JSON shape |
| Admin screens | Blade, Livewire, Filament, Nova, or project-standard admin stack |

## Transaction Boundaries

Use `DB::transaction()` when multiple writes must succeed or fail together.

```php
use Illuminate\Support\Facades\DB;

DB::transaction(function () use ($request): Order {
    $order = Order::create($request->validated());

    $order->items()->createMany($request->validated('items'));

    return $order;
});
```

Avoid transactions around slow external API calls. Persist intent first, then dispatch a job after commit when possible.

## Security Checklist

- Is the route authenticated?
- Is the action authorized with a policy/gate?
- Is input validated at the boundary?
- Are mass assignment rules explicit?
- Are database constraints backing important invariants?
- Are file uploads validated and stored safely?
- Are secrets kept out of logs and source code?
- Are rate limits needed?
- Does the feature expose personal, payment, or child/minor data?

## Scalability Checklist

- Are indexes needed for new query patterns?
- Could N+1 queries appear? Use eager loading where appropriate.
- Should slow work be queued?
- Is cache invalidation clear?
- Are external calls retried safely?
- Does the design behave correctly under concurrent requests?

## Living Specification Update

Before final output, update `specs/architect-architecture.md` and `specs/MANIFEST.md` when architecture decisions changed.

If `specs/architect-architecture.md` does not exist, create it using `spec-desc.md` as the structure reference. Append task-specific sections with this shape:

```markdown
### [TASK-001] Invitation Registration (2026-06-29)

**Module:** Registration

**Laravel Placement:**
- Routes: `routes/api.php`
- Controller: `app/Http/Controllers/RegistrationController.php`
- Requests: `app/Http/Requests/RegisterPlayerRequest.php`
- Models: `app/Models/User.php`, `app/Models/Invitation.php`

**Decisions:**
- Use Form Requests for validation and request authorization.
- Use policies for trainer-owned invitations.
- Dispatch notification jobs after transaction commit.

**Risks:**
- Token guessing must be prevented with high-entropy invitation tokens.
```

## Final Output

Return:

- Architecture decision summary.
- Files/specs updated.
- Security and scalability considerations.
- Context Summary.
- Next by flow: `/api-designer` or `/writing-plans`.
