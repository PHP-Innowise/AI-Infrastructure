---
name: coder
description: Implement Laravel backend features and bug fixes (behavior-changing work). Use for routes/controllers, Form Request validation, Eloquent models, Policies, API Resources, Actions/Services, and queued jobs. For pure behavior-preserving cleanups use refactorer; for scaffolding an approved architecture use architecture-implementer.
phase: execution
flow-next: code-reviewer
flow-alternatives: [test-generator, verify]
related: [architect, architecture-implementer, api-designer, test-generator, eloquent, queues-jobs, events-notifications, auth-scaffolding, caching, console-scheduler, file-storage]
---

# Coder

## Overview

Implement backend work in Laravel using the framework's conventions. Read the relevant routes, controllers, models, migrations, policies, tests, and specs before editing.

This branch targets Laravel (PHP 8.2+, 8.3+ required for Laravel 13). Supports Laravel 12 (current LTS) and Laravel 13 (current release). For framework-agnostic native PHP, use the `main` branch instead.

## Scope Boundary

`coder` owns **behavior-changing** work — new features, bug fixes, and the incidental cleanup that comes with them. Use a sibling skill when the task is narrower:

- **`/refactorer`** — pure behavior-preserving change (extract, dedupe, improve types, Laravel/PHP version upgrade) under a characterization test net. If observable behavior must stay identical, that is refactorer, not coder.
- **`/architecture-implementer`** — lay down the structural skeleton (models, policies, Form Requests, Resources, Service Provider bindings) for an approved architecture before feature logic exists. If there is no structure yet from `/architect`, scaffold there first, then return here to fill in behavior.
- **`/coder-frontend`** — server-rendered templates (Blade), HTML, CSS, and progressive-enhancement JS.
- **`/eloquent`** — deep model-layer behavior beyond basic CRUD: polymorphic relationships, custom accessor/cast classes, query scopes, model events/Observers, mass-assignment protection, large-dataset iteration.
- **`/queues-jobs`** — a queued Job needs job middleware, batching/chaining, uniqueness, or Horizon configuration beyond a simple `dispatch()`.
- **`/events-notifications`** — new Events/Listeners/Observers or Notifications/Mailables across mail/database/broadcast/Slack channels.
- **`/auth-scaffolding`** — web/session auth starter kits (Breeze/Jetstream/Fortify), multi-guard setup, or a Policy/Gate layer being introduced from scratch.
- **`/caching`** — introducing or fixing an application-data caching layer (stampede prevention, tagging, invalidation-on-write).
- **`/console-scheduler`** — a new Artisan console command or scheduled/recurring task.
- **`/file-storage`** — a feature that stores, serves, or accepts user-uploaded files (disk config, secure uploads, signed URLs).

## Project Structure

A conventional Laravel layout:

```
app/
├── Http/
│   ├── Controllers/     # thin controllers
│   ├── Middleware/      # HTTP middleware
│   ├── Requests/        # Form Request validation
│   └── Resources/       # API Resources / Resource Collections
├── Models/               # Eloquent models
├── Policies/             # authorization policies
├── Actions/              # single-purpose business logic (or Services/)
├── Services/             # multi-step / cross-cutting business logic
├── Providers/            # Service Providers (bindings, boot logic)
├── Jobs/                 # queued jobs
├── Events/ Listeners/    # event-driven side effects
├── Notifications/        # notifications (mail, database, etc.)
└── Exceptions/           # custom exceptions, Handler.php (Laravel <11)

bootstrap/
└── app.php               # exception handling + middleware config (Laravel 11+)

config/                   # config files, read via config()
database/
├── migrations/
├── factories/
└── seeders/
routes/
├── web.php
└── api.php
resources/views/          # Blade templates (if any)
tests/
├── Unit/
└── Feature/
```

Locate the app root by finding `composer.json` and confirming the Laravel version (`composer.json` `laravel/framework` constraint, or `php artisan --version`). Follow the structure already present rather than imposing this one.

## Workflow

1. **Understand** - read existing routes, controllers, models, migrations, policies, and tests.
2. **Plan** - decide route, Form Request, authorization (Policy/Gate), Eloquent model/relationships, Action/Service, and tests.
3. **Implement** - keep changes scoped, typed, and idiomatic to Laravel conventions.
4. **Test** - run focused tests first, then applicable DoD checks.
5. **Review** - check input validation, authorization, and data integrity.

## Laravel Implementation Rules

- Start new files with `declare(strict_types=1);` and full type declarations (return types, property types, param types) even though Laravel doesn't require it — it still catches bugs.
- Keep controllers thin: resolve the model via route-model binding, validate via a Form Request, call an Action/Service or model method, return an API Resource.
- Validate input with Form Requests (`php artisan make:request`), not inline `$request->validate()` in controllers, once rules grow beyond a couple of trivial fields.
- Authorize protected actions through Policies (`php artisan make:policy`) or Gates, called via `$this->authorize()`, `Gate::authorize()`, or the `can:` middleware — never by hiding UI only.
- Access the database through Eloquent models or the query builder; avoid raw SQL string concatenation. Use `DB::raw()` only with bound parameters when the query builder can't express something.
- Depend on interfaces at integration boundaries (e.g. `PaymentGateway`) bound in a Service Provider, so the concrete client is swappable and mockable.
- Manage schema changes through versioned Artisan migrations; never edit a released migration.
- Let Laravel's exception handling map exceptions to responses: customize in `bootstrap/app.php`'s `->withExceptions()` (Laravel 11+) or `app/Exceptions/Handler.php` (Laravel 10 and earlier / explicit Handler still supported). Throw typed, specific exceptions from domain code.
- Prefer guard clauses and early returns over deep nesting; keep methods small and single-purpose.
- Preserve backward compatibility of public signatures and API Resource shapes: add optional parameters/fields rather than reordering/removing, and version the API before making a breaking change.
- Never read or edit `.env`; read configuration through `config()` and document required variables in `config/*.php` with `env()` defaults.
- Use Eloquent relationships with eager loading (`with()`, `load()`) to avoid N+1 queries; verify with `DB::listen()`/Telescope/Debugbar during development.

## Common Patterns

### Routes

```php
// routes/api.php
use App\Http\Controllers\Api\UserController;

Route::middleware('auth:sanctum')->group(function (): void {
    Route::apiResource('users', UserController::class)->only(['index', 'store', 'show', 'update']);
});
```

Route-model binding resolves `{user}` to an `App\Models\User` instance automatically, returning a 404 if not found — prefer it over manual `User::findOrFail($id)` lookups in the controller.

### Form Request

```php
<?php

declare(strict_types=1);

namespace App\Http\Requests;

use Illuminate\Foundation\Http\FormRequest;
use Illuminate\Validation\Rule;

final class StoreUserRequest extends FormRequest
{
    public function authorize(): bool
    {
        return $this->user()?->can('create', \App\Models\User::class) ?? false;
    }

    /** @return array<string, mixed> */
    public function rules(): array
    {
        return [
            'email' => ['required', 'email', 'max:255', Rule::unique('users', 'email')],
            'name' => ['required', 'string', 'max:255'],
        ];
    }
}
```

### Controller

```php
<?php

declare(strict_types=1);

namespace App\Http\Controllers\Api;

use App\Actions\CreateUser;
use App\Http\Requests\StoreUserRequest;
use App\Http\Resources\UserResource;
use Illuminate\Http\JsonResponse;

final class UserController extends Controller
{
    public function __construct(private readonly CreateUser $createUser)
    {
    }

    public function store(StoreUserRequest $request): JsonResponse
    {
        // Authorization already ran in StoreUserRequest::authorize().
        $user = $this->createUser->handle($request->validated());

        return UserResource::make($user)->response()->setStatusCode(201);
    }
}
```

### Policy

```php
<?php

declare(strict_types=1);

namespace App\Policies;

use App\Models\User;

final class UserPolicy
{
    public function create(User $actor): bool
    {
        return $actor->hasRole('admin');
    }

    public function update(User $actor, User $target): bool
    {
        return $actor->id === $target->id || $actor->hasRole('admin');
    }
}
```

Register the policy in a Service Provider (or rely on Laravel's naming-convention auto-discovery) and enforce it with `$this->authorize('update', $user)`, `Gate::allows(...)`, or the `can:update,user` middleware on the route. For complex role/permission models beyond simple Gates, consider `spatie/laravel-permission`.

### API Resource

```php
<?php

declare(strict_types=1);

namespace App\Http\Resources;

use Illuminate\Http\Request;
use Illuminate\Http\Resources\Json\JsonResource;

final class UserResource extends JsonResource
{
    /** @return array<string, mixed> */
    public function toArray(Request $request): array
    {
        return [
            'id' => $this->id,
            'email' => $this->email,
            'name' => $this->name,
            'created_at' => $this->created_at?->toAtomString(),
        ];
    }
}
```

### Eloquent Model With Relationships

```php
<?php

declare(strict_types=1);

namespace App\Models;

use Illuminate\Database\Eloquent\Factories\HasFactory;
use Illuminate\Database\Eloquent\Model;
use Illuminate\Database\Eloquent\Relations\HasMany;

final class User extends Model
{
    use HasFactory;

    /** @var list<string> */
    protected $fillable = ['email', 'name'];

    protected function casts(): array
    {
        return [
            'email_verified_at' => 'datetime',
        ];
    }

    /** @return HasMany<Order, $this> */
    public function orders(): HasMany
    {
        return $this->hasMany(Order::class);
    }
}
```

Eager-load relationships that will be accessed (`User::with('orders')->get()`) instead of triggering N+1 lazy loads; enable `Model::preventLazyLoading()` in local/testing environments to catch violations early.

### Action / Service With A DB Transaction

Use an Action (single-purpose invokable class) or a Service (broader collaborator) once controller logic grows past simple CRUD. Wrap multi-step writes in `DB::transaction()` so partial failures roll back cleanly.

```php
<?php

declare(strict_types=1);

namespace App\Actions;

use App\Models\User;
use Illuminate\Support\Facades\DB;
use Illuminate\Support\Facades\Log;

final class CreateUser
{
    /** @param array<string, mixed> $data */
    public function handle(array $data): User
    {
        return DB::transaction(function () use ($data): User {
            $user = User::create($data);

            Log::info('User created', ['user_id' => $user->id]);

            // Dispatch side effects (welcome email, event) after the transaction commits.
            \App\Events\UserRegistered::dispatch($user);

            return $user;
        });
    }
}
```

Enforce uniqueness in the schema (`unique()` in the migration), not just in the Form Request — concurrent requests can both pass an application-level "does this email exist?" check. See `/architect` for concurrency, idempotency, and locking guidance the code must respect.

## Modern PHP Best Practices

Use the language's current features to make intent explicit and errors unrepresentable — these apply just as much inside a Laravel app:

- **Enums** for closed sets instead of string/int constants: `enum Role: string { case Admin = 'admin'; ... }`. Cast Eloquent attributes to enums via `casts()`.
- **`readonly` properties / classes** for value objects and DTOs (e.g. Action input objects) so state cannot mutate after construction.
- **Constructor property promotion** to keep Actions/Services/DTOs concise.
- **`match`** (strict, exhaustive) over long `switch`/`if` ladders.
- **Named arguments** for calls with several optional parameters; **nullsafe** `?->` for optional chains (handy on Eloquent relationships that may be null).
- **First-class callable syntax** (`$this->handle(...)`) for cleaner callbacks and job dispatch.
- **`never` return type** for functions that always throw or exit.
- Avoid `mixed` where a union or generic-via-docblock (`@param list<User>`) is clearer; let Larastan enforce it.

## Error Handling

- Define a small typed exception hierarchy for domain errors (e.g. `DomainException` base, `InsufficientStockException`) rather than throwing bare `\Exception`.
- Throw at the point of failure; catch only where you can add value (map to a response, add context, retry). Never swallow with an empty `catch`.
- Map exceptions to HTTP responses centrally: in Laravel 11+, register handling in `bootstrap/app.php`'s `->withExceptions(function (Exceptions $exceptions) { ... })`; on Laravel 10 and earlier, override `register()`/`render()` in `app/Exceptions/Handler.php`.
- Preserve the original cause with the `$previous` argument when re-throwing.
- Fail fast on programmer errors (invalid state) with exceptions; reserve return-value error signalling for expected, recoverable outcomes.
- Let validation exceptions (`ValidationException` from Form Requests) surface as Laravel's standard 422 response; don't catch and re-wrap them without reason.

## Logging

- Use the `Log::` facade (`Log::info()`, `Log::error()`, ...) or inject `Psr\Log\LoggerInterface` for testability; configure channels in `config/logging.php` (stack, single, daily, Slack, etc.).
- Use appropriate levels (`error` for failures needing attention, `warning` for recoverable anomalies, `info` for milestones, `debug` for diagnostics).
- Log structured context as the second argument (`Log::error('Payment failed', ['order_id' => $id])`); never log secrets, tokens, passwords, or full PII.

## Configuration

- Read config through `config('app.name')` / typed config, never `getenv()` scattered across the code or `.env` reads in app logic.
- Add new environment variables to a `config/*.php` file with an `env()` default, and document them; don't reference `env()` directly outside config files.
- Validate required config at boot (a Service Provider or a config validation package) and fail loudly if missing.

## Middleware

Use Laravel HTTP middleware for cross-cutting concerns (auth, rate limiting, logging) so controllers stay focused. Register global/group/route middleware in `bootstrap/app.php`'s `->withMiddleware()` (Laravel 11+) or `app/Http/Kernel.php` (Laravel 10 and earlier):

```php
<?php

declare(strict_types=1);

namespace App\Http\Middleware;

use Closure;
use Illuminate\Http\Request;
use Symfony\Component\HttpFoundation\Response;

final class EnsureUserIsActive
{
    public function handle(Request $request, Closure $next): Response
    {
        if (! $request->user()?->is_active) {
            abort(403, 'Account is disabled.');
        }

        return $next($request);
    }
}
```

Prefer built-in middleware (`auth`, `auth:sanctum`, `throttle:60,1`, `can:`, `verified`) before writing custom middleware for a concern Laravel already covers.

## Verification

Run focused checks first:

```bash
php artisan test --filter=CreateUserTest
# or: vendor/bin/pest --filter=CreateUserTest
```

Then run applicable project checks (prefer Composer scripts if defined):

```bash
composer validate --strict
php artisan test              # or vendor/bin/pest
vendor/bin/pint --test        # formatting check (drop --test to auto-fix)
vendor/bin/phpstan analyse    # Larastan-configured static analysis
```

If a tool is missing, report `N/A - tooling not configured`.

## Final Output

Include:

- What changed.
- Tests/checks run.
- Any security or migration notes.
- Context Summary.
- Next by flow: `/code-reviewer`, `/test-generator`, or `/verify`.
