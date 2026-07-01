---
name: coder
description: Implement Laravel backend features, bug fixes, and refactors. Use for controllers, routes, Form Requests, Eloquent models, migrations, services/actions, policies, jobs, resources, and PHP business logic.
phase: execution
flow-next: code-reviewer
flow-alternatives: [test-generator, verify]
related: [architect, api-designer, test-generator]
---

# Coder

## Overview

Implement backend work in Laravel using the project's existing conventions. Read the relevant routes, controllers, requests, models, migrations, tests, policies, and specs before editing.

## Project Structure

Laravel code normally belongs in the Laravel application root:

```
app/
├── Http/
│   ├── Controllers/
│   ├── Requests/
│   └── Resources/
├── Models/
├── Policies/
├── Services/
├── Actions/
├── Jobs/
├── Events/
└── Listeners/

routes/
├── api.php
└── web.php

database/
├── migrations/
├── factories/
└── seeders/

tests/
├── Feature/
└── Unit/
```

If the repository contains multiple apps, first identify the Laravel app root by locating `artisan` and `composer.json`.

## Workflow

1. **Understand** - read existing implementation, specs, migrations, and tests.
2. **Plan** - decide route, request, model, service/action, policy, migration, and tests.
3. **Implement** - keep changes scoped and idiomatic.
4. **Test** - run focused tests first, then applicable DoD checks.
5. **Review** - check security, validation, authorization, and data integrity.

## Laravel Implementation Rules

- Keep controllers focused on HTTP orchestration.
- Put validation in Form Requests unless the project uses a different clear convention.
- Put authorization in policies/gates or Form Request `authorize()`.
- Use API Resources for stable JSON response shapes.
- Use migrations for schema changes and factories for test data.
- Use services/actions for multi-step business behavior.
- Use jobs/events for slow or side-effect-heavy operations.
- Use `DB::transaction()` for atomic multi-write workflows.
- Avoid repositories unless they provide a real boundary, not just a wrapper around Eloquent.
- Never read or edit `.env`; use config keys and document required variables.

## Common Patterns

### Route

```php
use App\Http\Controllers\UserController;
use Illuminate\Support\Facades\Route;

Route::middleware('auth:sanctum')->group(function (): void {
    Route::apiResource('users', UserController::class);
});
```

### Form Request

```php
namespace App\Http\Requests;

use Illuminate\Foundation\Http\FormRequest;

class StoreUserRequest extends FormRequest
{
    public function authorize(): bool
    {
        return $this->user()?->can('create', User::class) ?? false;
    }

    public function rules(): array
    {
        return [
            'email' => ['required', 'email:rfc,dns', 'max:255', 'unique:users,email'],
            'name' => ['required', 'string', 'max:120'],
        ];
    }
}
```

### Controller

```php
namespace App\Http\Controllers;

use App\Http\Requests\StoreUserRequest;
use App\Http\Resources\UserResource;
use App\Models\User;

class UserController
{
    public function store(StoreUserRequest $request): UserResource
    {
        $user = User::create($request->validated());

        return new UserResource($user);
    }
}
```

### Service Or Action

```php
namespace App\Actions;

use App\Models\Invitation;
use App\Models\User;
use Illuminate\Support\Facades\DB;

class AcceptInvitation
{
    public function handle(Invitation $invitation, array $data): User
    {
        return DB::transaction(function () use ($invitation, $data): User {
            $user = User::create($data);

            $invitation->forceFill(['accepted_at' => now()])->save();

            return $user;
        });
    }
}
```

### API Resource

```php
namespace App\Http\Resources;

use Illuminate\Http\Request;
use Illuminate\Http\Resources\Json\JsonResource;

class UserResource extends JsonResource
{
    public function toArray(Request $request): array
    {
        return [
            'id' => $this->id,
            'email' => $this->email,
            'name' => $this->name,
        ];
    }
}
```

## Verification

Run focused checks first:

```bash
php artisan test --filter=UserRegistrationTest
```

Then run applicable project checks:

```bash
composer validate
php artisan test
vendor/bin/pint --test
vendor/bin/phpstan analyse
```

If a tool is missing, report `N/A - tooling not configured`.

## Final Output

Include:

- What changed.
- Tests/checks run.
- Any security or migration notes.
- Context Summary.
- Next by flow: `/code-reviewer`, `/test-generator`, or `/verify`.
