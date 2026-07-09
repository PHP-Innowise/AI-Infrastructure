---
name: performance-optimization
description: Diagnose and fix performance problems in native PHP applications with a measure-first workflow. Use for slow endpoints/scripts, high latency, high memory, N+1 queries, and throughput tuning. Triggers on "slow", "performance", "optimize", "latency", "memory", "profiling", "bottleneck".
phase: execution
flow-next: verify
flow-alternatives: [debugger, code-reviewer, test-generator]
related: [systematic-debugger, code-reviewer, architect, database-designer]
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
- Micro: `phpbench` for hot functions; a `microtime(true)`/`memory_get_peak_usage()` harness for a quick suspicion.
- Record environment: PHP version, OPcache on/off, dataset size. Numbers are meaningless without context.

Save the baseline to `performance-optimization-baseline.md`.

## Step 2: Profile

Find where time and memory actually go. Pick the tool for the environment:

| Tool | Best for | Notes |
| --- | --- | --- |
| Xdebug profiler | Local dev deep dive | High overhead; over-weights frequent small calls. Reads as callgrind (KCachegrind/QCachegrind). |
| SPX (`php-spx`) | Local/CLI flame graphs | Low overhead, easy setup, subtracts profiling overhead. |
| Blackfire | On-demand + CI perf assertions | Production-safe triggered profiles; before/after comparison. |
| Tideways / XHProf | Always-on APM, sampled traces | Continuous monitoring, regression detection. |

Read the profile top-down: which few calls dominate wall time and memory. Fix causes, not leaves.

## Step 3: Fix The Top Hotspots

Address the biggest levers first (usually in this order):

### Database (most common)
- Kill N+1 queries: batch with `WHERE id IN (...)` or a join instead of a query per row.
- Add targeted indexes; confirm with `EXPLAIN`.
- Use prepared statements for repeated queries; paginate with `LIMIT`/keyset.
- Avoid `SELECT *` on wide/hot tables; select needed columns.

### I/O and external calls
- Set timeouts; batch or parallelize independent calls.
- Move slow, non-critical work to a queue/worker.

### Caching
- Cache hot, expensive, reusable results: APCu (single process) or a shared cache (Redis) via PSR-6/PSR-16.
- Make cache keys and invalidation explicit; a wrong cache is worse than none.

### Algorithms and memory
- Replace O(n^2) scans over large arrays; index with maps.
- Stream large datasets with generators (`yield`) instead of building giant arrays.
- Free large variables and use `unset()` in long-running scripts.

### Runtime
- Ensure OPcache is enabled in production (`opcache.validate_timestamps=0`, sized `memory_consumption` and `max_accelerated_files`).
- Consider OPcache preloading and, for CPU-bound work, evaluate JIT (measure; it does not help I/O-bound code).
- Optimize the Composer autoloader for production: `composer dump-autoload -o` (or `--classmap-authoritative`).

## Step 4: Re-measure And Prevent Regression

- Re-run the exact baseline scenario; report before/after with the same environment.
- Add a `phpbench` benchmark or a Blackfire assertion so the win is protected in CI.
- If a change did not help, revert it. Keeping "probably faster" code adds risk without benefit.

## Report Template

```markdown
# Performance Report: [Target]

## Baseline
- Scenario: [endpoint/script + dataset]
- p95: [X ms], throughput: [Y req/s], peak memory: [Z MB]
- Environment: PHP [ver], OPcache [on/off]

## Profile Findings
1. [Hotspot] - [% of wall time] - [cause]

## Changes
1. [Change] -> [before] to [after]

## Result
- p95: [X -> X'] ms ([%] improvement)
- Regression guard: [phpbench/Blackfire assertion added]

## Remaining Risks / Follow-ups
- [...]
```

## Red Flags - STOP

- "This is obviously the slow part" without a profile.
- Changing several things at once so you cannot attribute the win.
- Adding a cache to hide an N+1 instead of fixing the query.
- Enabling JIT and declaring victory without measuring.

## Final Output

Return the baseline, profiling findings, changes with before/after numbers, regression guard, Context Summary, and next step (`/verify`, `/code-reviewer`, or `/debugger`).
