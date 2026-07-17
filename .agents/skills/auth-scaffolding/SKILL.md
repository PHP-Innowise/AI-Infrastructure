---
name: auth-scaffolding
description: Set up Laravel web/session authentication - starter kits (Breeze, Jetstream, Fortify), multi-guard configurations, and deep Policy/Gate authorization patterns. For token-based API authentication (Sanctum/Passport/JWT), use api-designer instead.
phase: execution
flow-next: security-reviewer
flow-alternatives: [test-generator, code-reviewer]
related: [api-designer, coder, architect]
---

# Auth Scaffolding

## Overview

Set up or extend web/session-based authentication — starter-kit scaffolding, guard configuration, and the Policy/Gate authorization layer — for server-rendered Blade, Livewire, or Inertia applications.

This branch targets Laravel (PHP 8.2+, 8.3+ required for Laravel 13). Supports Laravel 12 (current LTS) and Laravel 13 (current release, shipped March 2026). For framework-agnostic native PHP, use the `main` branch instead.

## Scope Boundary

`api-designer` owns **token-based API authentication** decisions (Sanctum vs Passport vs stateless JWT) and already has a decision table for that trade-off — go there for a mobile/SPA/third-party API client. `auth-scaffolding` owns **web/session-based authentication**: which starter kit to scaffold from, how guards are configured, and the Policy/Gate layer that both API and web contexts share. If a project needs both (a Blade admin area plus a public JSON API), scaffold web auth here and layer Sanctum in via `/api-designer` — the two are complementary, not alternatives.

## Starter Kit Decision

Verify current package state before recommending anything — this area has changed materially since Laravel 12 shipped. As of Laravel 12/13 (2026):

- **The first-party Starter Kits (`laravel new`)** are now the primary, actively-developed recommendation for new applications. The installer prompts for a React, Vue, Svelte, or Livewire frontend flavor (or accepts `--using=vendor/kit` for a community kit), and every official flavor is built on **Fortify** under the hood — you still get routes, controllers, and views published directly into your app, so you own and can freely modify the generated code. Laravel 13's kits also scaffold native WebAuthn/passkey support (`Features::passkeys()` in `config/fortify.php`) and an opt-in Teams feature (`laravel new my-app --teams`) with URL-scoped multi-tenancy (`/teams/{team:slug}/...`) that replaces the old Jetstream Teams implementation.
- **Breeze** and **Jetstream** still install and run correctly on Laravel 12/13, but Laravel's own 12.x release notes state that, with the new Starter Kits, "Breeze and Jetstream will no longer receive additional updates" — both are in maintenance-only mode (compatibility patches only, no new features). They are a legitimate choice for a team already standardized on their generated code, or when following an older tutorial/course, but **default to the current first-party Starter Kits for new projects** rather than Breeze/Jetstream unless there's a specific reason (team familiarity, an existing Breeze/Jetstream codebase being extended) to reach for the older kits.
- **Fortify** (`laravel/fortify`) is the headless engine — routes, controllers, rate-limited login logic, 2FA, passkeys — with **no views**, powering both the new Starter Kits and Jetstream. Install it standalone only when building a fully custom UI (a hand-rolled Blade/Livewire/Inertia frontend, or a mobile app's web-based auth backend) where the generated Starter Kit views aren't the right fit, but you still want Laravel's auth logic, routes, and rate limiting handled for you rather than hand-rolling login/registration/password-reset controllers.

| Situation | Recommendation |
| --- | --- |
| New app, no special auth requirements | First-party Starter Kit (`laravel new`, pick React/Vue/Svelte/Livewire) |
| New app needing team-based multi-tenancy on day one | First-party Starter Kit with `--teams` |
| Fully custom frontend, still want Laravel's auth logic/routes/2FA | Fortify standalone (no views) |
| Existing Breeze/Jetstream codebase | Keep it; it still works, just isn't gaining new features |
| Following an older guide/course that references Breeze/Jetstream | Note the maintenance status to the user before scaffolding; recommend the current Starter Kit unless they have a specific reason to match the guide |

```bash
laravel new my-app                 # prompts for React/Vue/Svelte/Livewire, Fortify under the hood
laravel new my-app --teams          # adds team-scoped multi-tenancy
composer require laravel/fortify   # headless-only, when building a fully custom UI
```

## Multi-Guard Setups

Add a guard in `config/auth.php` when the app has more than one authenticatable "type" — e.g. a `User` web guard alongside a separate `Admin` guard, or a `sanctum` API guard alongside the `web` session guard:

```php
// config/auth.php
'guards' => [
    'web' => ['driver' => 'session', 'provider' => 'users'],
    'admin' => ['driver' => 'session', 'provider' => 'admins'],
    'sanctum' => ['driver' => 'sanctum', 'provider' => 'users'],
],

'providers' => [
    'users' => ['driver' => 'eloquent', 'model' => App\Models\User::class],
    'admins' => ['driver' => 'eloquent', 'model' => App\Models\Admin::class],
],
```

```php
// routes/web.php
Route::middleware('auth:admin')->prefix('admin')->group(function (): void {
    Route::get('/dashboard', AdminDashboardController::class);
});
```

The common mistake: forgetting to pass the guard name to `auth:` middleware, `Auth::guard('admin')->user()`, and `Auth::guard('admin')->attempt()` on every admin-facing route/check — omitting it silently falls back to the default guard (`web`), which either authenticates against the wrong provider or leaves the route effectively unprotected against the guard you intended. Audit every `auth`/`Auth::` call in a new guarded area explicitly, and confirm login/logout routes and their controllers target the correct guard, not just the routes in between.

## Policy/Gate Deep Patterns

Beyond the one-line `$this->authorize()` pattern already in `AGENTS.md`/`architect`:

- **`Policy::before()` for a super-admin bypass** — runs before any other policy method; return `true`/`false` to short-circuit, or `null` to fall through to the specific method:

```php
final class OrderPolicy
{
    public function before(User $actor, string $ability): ?bool
    {
        return $actor->hasRole('super-admin') ? true : null;
    }

    public function update(User $actor, Order $order): bool
    {
        return $actor->id === $order->user_id;
    }
}
```

- **Auto-discovery/registration** — Laravel auto-discovers a Policy named `{Model}Policy` in `app/Policies` for a model in `app/Models` by convention; register explicitly in `AuthServiceProvider`'s `$policies` array (or via `Gate::policy()`) only when the model/policy don't follow that naming convention or live outside the conventional directories.
- **Authorizing a collection** — don't loop `Gate::authorize()` per item to build a filtered list (it throws on the first denial); instead filter the query/collection with the ability, and reserve `$this->authorize()`/`Gate::authorize()` for a single record or action that should reject the whole request:

```php
$visibleOrders = Order::query()->get()->filter(fn (Order $order): bool => $actor->can('view', $order));
// or, pushed into the query itself where the rule is expressible in SQL:
$visibleOrders = Order::query()->where('user_id', $actor->id)->get();
```

- **Testing Policies directly vs through HTTP** — unit-test the Policy class in isolation for its authorization matrix (fast, no HTTP/DB-route overhead beyond model factories), and additionally cover at least one Feature test per protected route asserting `assertForbidden()`/`assertOk()` end-to-end, since a unit-tested Policy that is never actually wired to the route (missing `$this->authorize()` call, wrong ability name) would otherwise pass silently:

```php
it('only lets the order owner update it', function (): void {
    $owner = User::factory()->create();
    $other = User::factory()->create();
    $order = Order::factory()->for($owner)->create();

    expect((new OrderPolicy())->update($owner, $order))->toBeTrue();
    expect((new OrderPolicy())->update($other, $order))->toBeFalse();
});

it('forbids a non-owner from updating an order via HTTP', function (): void {
    $other = User::factory()->create();
    $order = Order::factory()->create();

    $this->actingAs($other)->patchJson("/orders/{$order->id}", [])->assertForbidden();
});
```

## Password And Account Security Basics

Starter kits already wire these up, but confirm they're actually in place rather than assuming — worth stating explicitly since they're easy to accidentally bypass with a custom controller:

- **Hashed passwords** — always via the `Hash::` facade (`Hash::make()`, `Hash::check()`) or the model's `password` cast (`'password' => 'hashed'` in `casts()` on Laravel 10.14+); never roll a custom hashing scheme or compare passwords with `===`.
- **Rate-limited login attempts** — Fortify/Breeze/Jetstream apply `RateLimiter::for('login', ...)` (keyed by email + IP) out of the box; if hand-rolling a login controller, add the `throttle:` middleware (or a named `RateLimiter` matching the login route) so credential-stuffing can't be attempted at unlimited speed.
- **Email verification** — implement `MustVerifyEmail` on the `User` model and apply the `verified` middleware to routes that require a confirmed address; starter kits scaffold the verification notice/notification but the app must still decide *which* routes require it.

```php
// app/Models/User.php
final class User extends Authenticatable implements MustVerifyEmail { /* ... */ }

// routes/web.php
Route::middleware(['auth', 'verified'])->group(function (): void {
    Route::get('/dashboard', DashboardController::class);
});
```

## Verification

Possible checks:

```bash
php artisan test --filter=Auth
php artisan test --filter=Policy
vendor/bin/pint --test
vendor/bin/phpstan analyse
php artisan route:list --name=login   # confirm guard/middleware wiring
```

Use only commands present in the project; report others as `N/A - tooling not configured`.

## Final Output

Return which starter kit/guard/Policy changes were made and why, confirmation of password/rate-limit/verification wiring, tests run, Context Summary, and next step.
