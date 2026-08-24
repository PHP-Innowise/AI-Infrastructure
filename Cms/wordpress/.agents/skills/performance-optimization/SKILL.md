---
name: performance-optimization
description: Diagnose and optimize WordPress performance using measurement first across hooks, queries, object/page caches, options, REST, blocks, assets, cron, admin, multisite, and WooCommerce.
phase: quality
flow-next: verify
flow-alternatives: [coder, database-designer, systematic-debugger]
related: [code-reviewer, cron-background-processing, block-development]
---

# WordPress Performance Optimization

## Procedure

1. Define the failing metric, request/command, environment, data volume,
   concurrency and baseline. Do not optimize from intuition alone.
2. Reproduce with available tools such as Query Monitor, WP-CLI profile,
   application/APM traces, database slow logs, browser performance tools,
   Lighthouse or project benchmarks. Keep production data and secrets out of
   reports.
3. Attribute time and memory to hooks/callbacks, SQL and meta/term cache,
   autoloaded options, remote requests, block rendering, assets, cron/actions,
   filesystem work or WooCommerce CRUD/event paths.
4. Fix the dominant cause with the smallest change: scope hook registration,
   bound queries, prime caches, remove N+1 access, add a justified index, reduce
   autoload, batch work, avoid repeated parsing, split bundles, lazy-load, or
   move delay-tolerant work out of the request.
5. Cache only after defining key, site/user/locale/context dimensions, TTL,
   invalidation events, stampede behavior and stale-data tolerance. Never cache
   personalized/privileged responses under a public key.
6. Re-run the same measurement and full relevant tests. Report before/after,
   variance, data size and trade-offs; revert complexity that does not produce
   meaningful improvement.

## Output

Return baseline and evidence, root cause, change, cache/query/asset implications,
before/after result, regression checks, remaining bottlenecks, Context Summary,
and next step.
