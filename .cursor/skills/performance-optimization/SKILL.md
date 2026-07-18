---
name: performance-optimization
description: Diagnose and fix performance problems in Laravel applications with a measure-first workflow. Use for slow endpoints/scripts, high latency, high memory, N+1 queries, and throughput tuning. Triggers on "slow", "performance", "optimize", "latency", "memory", "profiling", "bottleneck".
phase: execution
flow-next: verify
flow-alternatives: [debugger, code-reviewer, test-generator]
related: [systematic-debugger, code-reviewer, architect, database-designer, caching, queues-jobs]
---

# Performance Optimization

## Overview

Make it measurably faster without guessing. The iron rule: measure, then change one thing, then measure again. Optimizing without a baseline wastes effort and risks regressions.

**Core loop:** baseline -> profile -> fix top 1-3 hotspots -> re-measure -> lock in a budget.

## Generated File Naming Convention (MANDATORY)

Any file created by this skill MUST be prefixed with `performance-optimization-`:
- Correct: `performance-optimization-baseline.md`, `performance-optimization-report.md`
- Incorrect: `PERF.md`, `benchmark.md`

## The Iron Law

```
NO OPTIMIZATION WITHOUT A MEASUREMENT THAT PROVES THE HOTSPOT
```

If you cannot point to a number, you are guessing. Do not micro-optimize cold code.

## Step 1: Baseline

Capture a realistic, repeatable measurement before touching code.

- End-to-end: response time (p50/p95), requests/sec (e.g. `ab`, `wrk`, `k6`).
- Query-level: enable `Model::preventLazyLoading(! app()->isProduction())` in `AppServiceProvider::boot()` to make N+1 lazy loads throw in local/testing instead of silently degrading production.
- Micro: `phpbench` for hot functions; a `microtime(true)`/`memory_get_peak_usage()` harness for a quick suspicion.
- Record environment: PHP version, OPcache on/off, queue driver, dataset size. Numbers are meaningless without context.

Save the baseline to `performance-optimization-baseline.md`.

## Step 2: Profile

Find where time, queries, and memory actually go. Pick the tool for the environment:

| Tool | Best for | Notes |
| --- | --- | --- |
| Laravel Telescope | Local/staging deep dive | Inspects requests, queries (with duplicate-query detection), jobs, cache hits, exceptions in one dashboard. Gate it off in production. |
| Laravel Debugbar (`barryvdh/laravel-debugbar`) | Fast local feedback loop | Inline query count/time, memory, view render time on every page load. |
| Laravel Pulse | Always-on production monitoring | Slow requests, slow queries, queue throughput, exception rates, server load — safe for production, low overhead. |
| Xdebug profiler | Local dev deep dive on PHP-level hotspots | High overhead; over-weights frequent small calls. Reads as callgrind (KCachegrind/QCachegrind). |
| Blackfire | On-demand + CI perf assertions | Production-safe triggered profiles; before/after comparison. |

Read the profile top-down: which few queries/calls dominate wall time, query count, and memory. Fix causes, not leaves. Telescope's query list and Debugbar's query count badge are usually the fastest way to spot N+1s.

## Step 3: Fix The Top Hotspots

Address the biggest levers first (usually in this order):

### Eloquent / database (most common)
- Kill N+1 queries: eager load with `with()`/`load()` (and `loadMissing()` for conditional loading) instead of accessing relationships inside a loop.
- Turn on `Model::preventLazyLoading()` outside production so future N+1s fail loudly in tests/CI instead of shipping.
- Add targeted indexes via a migration; confirm with `EXPLAIN`.
- Avoid `Model::all()`/wide `select('*')` on hot tables; select only needed columns and paginate (`paginate()`/`simplePaginate()`/cursor pagination).
- Stream large result sets with `chunk()`, `chunkById()`, or `cursor()` instead of loading the whole collection into memory.

### I/O and external calls
- Set timeouts on `Http::timeout()->retry()`; batch or parallelize independent calls with `Http::pool()`.
- Move slow, non-critical work off the request cycle and onto a queued job (`ShouldQueue`), backed by a real queue worker or Horizon rather than the `sync` driver.

### Caching
- Cache hot, expensive, reusable results with `Cache::remember()`/`Cache::rememberForever()` against a shared driver (Redis/Memcached) in production.
- Cache framework artifacts in production: `php artisan route:cache`, `php artisan view:cache`, `php artisan config:cache` (and `event:cache` if applicable). Remember to clear/re-cache on every deploy.
- Make cache keys and invalidation explicit (tag-based invalidation with `Cache::tags()` where the driver supports it); a wrong cache is worse than none.
- For the implementation itself — stampede prevention, driver-specific tagging support, model-level caching, and a correct invalidation-on-write path — use the `caching` skill rather than bolting on an ad hoc `Cache::remember()` call.

### Algorithms and memory
- Replace O(n^2) scans over large Collections; index with `keyBy()`/maps instead of nested `->first()` lookups.
- Use `LazyCollection`/`cursor()` for very large datasets instead of building giant Eloquent collections.
- Free large variables and `unset()` in long-running Artisan commands/queue workers.

### Runtime
- Ensure OPcache is enabled in production (`opcache.validate_timestamps=0`, sized `memory_consumption` and `max_accelerated_files`).
- Consider OPcache preloading and, for CPU-bound work, evaluate JIT (measure; it does not help I/O-bound Laravel requests).
- Optimize the Composer autoloader for production: `composer install --optimize-autoloader --no-dev` (or `composer dump-autoload -o --classmap-authoritative`).
- Confirm queue workers are sized appropriately (Horizon supervisors, `--max-jobs`, `--timeout`) so a slow job doesn't starve the queue.

## Step 4: Re-measure And Prevent Regression

- Re-run the exact baseline scenario; report before/after with the same environment.
- Confirm query count didn't just move — check Telescope/Debugbar again, not only wall time.
- Add a `phpbench` benchmark or a Blackfire assertion so the win is protected in CI; keep `Model::preventLazyLoading()` on in tests so N+1s regress loudly.
- If a change did not help, revert it. Keeping "probably faster" code adds risk without benefit.

## Laravel Octane

Octane (`laravel/octane`) replaces Laravel's normal per-request bootstrap with a persistent application server (FrankenPHP, RoadRunner, or Swoole) that keeps the framework booted in memory and reuses it across requests. This removes bootstrap/autoload overhead per request and can meaningfully raise throughput and lower latency — but it is an architectural change, not a tuning knob. Most CRUD apps are bottlenecked on database I/O or external calls, not framework bootstrap, and gain little from it. Consistent with the measure-first philosophy above: only adopt Octane after Steps 1-3 show bootstrap overhead as a real hotspot under sustained, measured throughput/latency pressure — not preemptively.

### The Gotcha: Singletons And Static State Leak Across Requests

Under Octane, the service container and any `singleton()` bindings are resolved once per worker and reused for every subsequent request — they are never torn down the way a normal PHP-FPM/CLI request lifecycle would tear them down. Anything bound as `singleton()` that captures per-request state (the authenticated user, a request-scoped cache/collector, a mutable static property) leaks that state into later requests on the same worker: a classic symptom is one user's data appearing on another user's response.

The fix is `scoped()` bindings for anything holding per-request state: `scoped()` behaves like `singleton()` (same instance within a request) but is automatically reset between requests under Octane, and behaves exactly like `singleton()` under a normal, non-Octane request lifecycle — so it stays correct even if the project only sometimes runs under Octane. Octane resets scoped bindings itself; do not hand-roll a "reset this static property" workaround when a `scoped()` binding solves it.

```php
// Unsafe under Octane: this singleton captures the authenticated user once
// per worker, then serves the same user's data to every later request.
class AppServiceProvider extends ServiceProvider
{
    public function register(): void
    {
        $this->app->singleton(CurrentUserContext::class, function ($app) {
            return new CurrentUserContext($app->make('request')->user());
        });
    }
}

// Safe: scoped() re-resolves per request under Octane and behaves like a
// normal singleton outside Octane — no manual reset required.
class AppServiceProvider extends ServiceProvider
{
    public function register(): void
    {
        $this->app->scoped(CurrentUserContext::class, function ($app) {
            return new CurrentUserContext($app->make('request')->user());
        });
    }
}
```

### Adoption Checklist

- Audit every Service Provider `singleton()` binding before adopting Octane; convert anything holding per-request or per-user state to `scoped()`.
- Never accumulate request- or user-derived data in static properties or global arrays; if a package or legacy class does this and can't be refactored, reset it via an Octane `RequestTerminated`/`RequestHandled` listener rather than leaving it to leak.
- Soak-test before shipping: run sustained load (`--watch` during development, or a real load test in staging) and watch worker memory over time — steady-state memory is healthy, continuous growth means a leak.
- Treat `--max-requests` (worker recycling) as a safety net that limits the blast radius of an undiscovered leak, not a fix for a known one; a real leak still needs to be found and fixed.

## Report Template

```markdown
# Performance Report: [Target]

## Baseline
- Scenario: [route/command + dataset]
- p95: [X ms], throughput: [Y req/s], query count: [N], peak memory: [Z MB]
- Environment: PHP [ver], Laravel [ver], OPcache [on/off], queue driver [sync/redis/...]

## Profile Findings
1. [Hotspot] - [% of wall time / query count] - [cause]

## Changes
1. [Change] -> [before] to [after]

## Result
- p95: [X -> X'] ms ([%] improvement)
- Query count: [N -> N']
- Regression guard: [phpbench/Blackfire assertion / preventLazyLoading added]

## Remaining Risks / Follow-ups
- [...]
```

## Red Flags - STOP

- "This is obviously the slow part" without a profile.
- Changing several things at once so you cannot attribute the win.
- Adding a `Cache::remember()` to hide an N+1 instead of fixing the eager loading.
- Enabling JIT and declaring victory without measuring.

## Final Output

Return the baseline, profiling findings, changes with before/after numbers, regression guard, Context Summary, and next step (`/verify`, `/code-reviewer`, or `/debugger`).
