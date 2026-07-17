---
name: caching
description: Design and implement Laravel caching strategy - Cache facade patterns, stampede prevention, driver-specific tagging, model-level caching, and invalidation-on-write correctness. Use when a read is expensive/repeated or when caching is introduced as a fix for a measured slowdown.
phase: execution
flow-next: performance-optimization
flow-alternatives: [code-reviewer, verify]
related: [performance-optimization, architect, coder]
---

# Caching

## Overview

Implement an application-data caching layer correctly: pick the right `Cache::` facade pattern, protect hot keys from stampede under concurrent load, respect what the configured cache driver actually supports, and — the part most guides skip — make sure every cached value has a correct invalidation path. A cache that returns stale/wrong data is a correctness bug, not just a missed optimization.

This branch targets Laravel (PHP 8.2+, 8.3+ required for Laravel 13). Supports Laravel 12 (current LTS) and Laravel 13 (current release). For framework-agnostic native PHP, use the `main` branch instead.

## Scope Boundary

`caching` owns **how** to implement an application-data caching layer once one is warranted. Use a sibling skill when the question is different:

- **Whether caching is the right fix at all** -> `performance-optimization` decides this with a measure-first workflow (baseline, profile, fix the proven hotspot). Do not reach for `Cache::remember()` speculatively; that skill's Red Flags section explicitly calls out "adding a `Cache::remember()` to hide an N+1 instead of fixing the eager loading." Come here once a measurement has already justified caching as the fix, or once the user is explicitly asking to add/fix a caching layer.
- **Laravel Octane** (persistent in-memory application server, `singleton()`/`scoped()` binding lifecycle) -> already covered in `performance-optimization`'s Octane section; not repeated here. Octane is an architectural/runtime concern, not the `Cache::` facade.
- **Framework-artifact caching at deploy time** (`php artisan config:cache`, `route:cache`, `view:cache`) -> covered by `architect`'s Deployment Considerations note. This skill is about caching *application data* through the `Cache::` facade, a different concern from caching compiled framework artifacts.
- **Model/Observer mechanics in general** -> `coder` owns Eloquent models, relationships, and general model-layer conventions. This skill only covers the caching-specific parts of a model-level cache (what to cache, and where the invalidation trigger lives).

## Locate The Cache Configuration

Before recommending any pattern, read `config/cache.php` and confirm `CACHE_STORE` (or the legacy `CACHE_DRIVER`) in use. Laravel defaults to the `database` driver out of the box; many projects run `redis` or `file` in different environments (e.g. `file`/`array` locally, `redis` in production). The driver in use determines which patterns below are even valid — do not assume Redis-only features are available.

## Core `Cache::` Facade Patterns

### `remember()` / `rememberForever()` — the default read-through pattern

`remember()` is the standard cache-aside pattern: return the cached value if present, otherwise run the callback, store the result, and return it.

```php
$user = Cache::remember("user.{$id}.profile", now()->addMinutes(30), function () use ($id) {
    return User::with('profile')->findOrFail($id);
});
```

`rememberForever()` skips the TTL and keeps the value until explicitly forgotten — only appropriate when the invalidation trigger is fully reliable (see Invalidation below), since there is no expiry safety net if a write path is missed.

The trade-off with plain `remember()`: when the TTL expires, the *next* request pays the full cost of recomputing the value and every concurrent request in that window does too (see Stampede below).

### `flexible()` — stale-while-revalidate

`Cache::flexible()` (current in Laravel 12.x/13.x) serves stale data immediately instead of blocking, then refreshes in the background:

```php
$stats = Cache::flexible('dashboard.stats', [60, 300], function () {
    return [
        'total_users' => User::count(),
        'revenue_today' => Order::whereDate('created_at', today())->sum('total'),
    ];
});
```

The two-element array is `[fresh_seconds, stale_seconds]`: within the first window the cache is returned as-is with no work triggered; within the second window the stale value is still returned instantly, but a deferred callback refreshes it after the response is sent (this requires the `InvokeDeferredCallbacks` middleware, included by default in a current Laravel skeleton). Only past both windows does a request block on recomputation.

**When to use which:** default to `remember()` — it is simpler and guarantees the caller never sees data older than the TTL. Reach for `flexible()` specifically when a user-facing response must never be slow because of a cache miss (dashboard widgets, high-traffic public pages) and a few extra minutes of staleness is an acceptable trade for consistently fast responses. Do not use `flexible()` where the value must be provably fresh (e.g. an account balance about to be debited).

## Cache Stampede / Dog-Piling Prevention

A stampede happens when a hot key expires (or is missing on a cold start) and many concurrent requests all recompute the same expensive value at once — turning one expensive query into N simultaneous expensive queries against the database at the worst possible moment.

`Cache::lock()` gives an atomic, distributed lock so only one process recomputes while the rest wait (or fall back to a stale value):

```php
$report = Cache::remember('report.monthly', now()->addHour(), function () {
    return Cache::lock('report.monthly:lock', 15)->block(5, function () {
        // Re-check: another process may have populated the cache while this one waited.
        return Cache::get('report.monthly') ?? Report::generateMonthly();
    });
});
```

Key details:
- `Cache::lock($name, $seconds)` requires the `memcached`, `redis`, `dynamodb`, `database`, `file`, or `array` cache driver (essentially any first-party driver except leaving it unconfigured) and, in a multi-server setup, all servers must share the same central cache backend for the lock to be effective.
- `->block($waitSeconds, $callback)` waits up to `$waitSeconds` to acquire the lock, then runs the callback and releases the lock automatically (even on exception) — prefer this over `->get()`/`->release()` pairs to avoid leaking a held lock.
- Size the lock's own duration (`$seconds`, the second argument to `Cache::lock()`) to comfortably outlive the work it protects; a lock that expires mid-computation lets a second process in and defeats the purpose.
- The re-check (`Cache::get(...) ?? ...`) inside the lock matters: without it, every waiter that acquires the lock after the first recomputes the value again instead of reusing what the winner just stored.

## Driver-Specific Caveats: Cache Tags

`Cache::tags([...])` lets related keys be invalidated together (`Cache::tags(['users'])->flush()`), but **tags are only supported by the `redis` and `memcached` drivers** — not `database`, `file`, `dynamodb`, or (as of Laravel 13.x) `storage`. Calling `->tags()` against an unsupported driver throws (`BadMethodCallException`), so this fails loudly in most cases, but a project that switches its default cache driver between environments (e.g. `array`/`file` locally, `redis` in production) can pass locally and break in a way only visible in one environment.

```php
// Only correct if config/cache.php's active driver is redis or memcached:
Cache::tags(['users', "user:{$id}"])->put("user.{$id}.profile", $profile, 3600);

// Portable across every driver — track related keys explicitly instead:
Cache::put("user.{$id}.profile", $profile, 3600);
Cache::forget("user.{$id}.profile"); // on the relevant write path
```

Always check `config/cache.php`'s configured store before recommending `->tags()`. If the project needs tag-like invalidation but runs on `database`/`file`, prefer explicit key naming/forgetting (as above) or a manually tracked key registry over reaching for tags.

## Model-Level Caching

Caching an expensive derived value on a model (a computed aggregate, an expensive relationship count, an external-lookup result) follows the same `remember()`/`flexible()` pattern, keyed by the model's identity:

```php
final class Team extends Model
{
    public function averagePlayerRating(): float
    {
        return Cache::remember(
            "team.{$this->id}.average_rating",
            now()->addHour(),
            fn () => $this->players()->avg('rating') ?? 0.0,
        );
    }
}
```

The invalidation trigger belongs on the write path, not inside the cached method itself: a model Observer (`php artisan make:observer`, registered for the model that changes) calling `Cache::forget()` on `saved`/`deleted`, or an explicit `Cache::forget()` call at the end of the Action/Service that performs the write. See `eloquent` for Observer mechanics in general (registration, when an Observer is the right tool vs. an Action) — this skill only covers *what* triggers the forget, not how Observers are wired.

```php
final class PlayerObserver
{
    public function saved(Player $player): void
    {
        Cache::forget("team.{$player->team_id}.average_rating");
    }
}
```

## Cache Invalidation Is The Hard Part

The `remember()` call is the easy 10%; correct invalidation is the hard 90%. Prefer this ordering of strategies:

1. **Short TTL, no explicit invalidation** — simplest, safe default for data where a few minutes of staleness is fine and there is no natural "write path" to hook.
2. **Explicit `Cache::forget()` on the write path** (Observer or end of Action/Service) — use once a specific write clearly invalidates a specific key, and the key can be reconstructed deterministically from the write's context (e.g. the model ID).
3. **Tags** (`Cache::tags()`) — only on `redis`/`memcached`, and only when several keys must invalidate together and explicit `forget()` calls would be error-prone to keep in sync.

A long TTL (or `rememberForever()`) with no invalidation story is not a performance shortcut — it is a correctness bug waiting to happen: the first write that isn't wired to a `forget()` call silently serves stale data indefinitely. When in doubt, prefer a short TTL over an unmanaged long one.

## Verification

Possible checks:

```bash
php artisan test --filter=Cache
vendor/bin/pint --test
vendor/bin/phpstan analyse
php artisan tinker --execute="dd(config('cache.default'))"   # confirm active driver before recommending tags
```

Use only commands present in the project.

## Final Output

Return: which pattern was used (`remember`/`flexible`/`lock`) and why, the configured cache driver and whether tags were safe to use, where the invalidation trigger lives (Observer/explicit `forget()`/TTL-only), tests run, Context Summary, and next step.
