---
name: performance-optimization
description: "Diagnose and fix Symfony performance problems with a measure-first workflow: Symfony Profiler/Blackfire, Doctrine N+1/query tuning, cache, Twig, Messenger, memory, OPcache."
phase: execution
flow-next: verify
flow-alternatives: [code-reviewer, systematic-debugger, test-generator]
---

# Symfony Performance Optimization

Optimize only measured bottlenecks. Preserve correctness, authorization, consistency, and operability while reducing latency or resource use.

## Required Context

Inspect the affected route/command/worker, service and repository calls, Doctrine mappings/indexes, cache configuration, Twig/Serializer behavior, Messenger transport/worker settings, deployment/runtime configuration, existing metrics, and performance tests. Confirm PHP/Symfony/database versions and whether Profiler, Blackfire, APM, or production-like data is available.

## Baseline First

Define one reproducible scenario and record:

- environment, dataset size, warm/cold cache state, concurrency, and request/command/message inputs;
- median and tail latency (`p50`/`p95`/`p99`) where multiple samples are possible;
- Doctrine query count/time, duplicate queries, rows/hydration volume, and database plan;
- memory peak, CPU time, network/external-call time, cache hit rate, or worker throughput as relevant;
- correctness assertions so a faster but wrong result cannot pass.

Do not compare a cold baseline with a warm result or development Profiler overhead with production numbers without stating the limitation.

## Profile

Use the least invasive available evidence:

1. Symfony Profiler/Web Debug Toolbar for request timeline, Doctrine, Twig, Serializer, cache, events, and logs.
2. Blackfire or the installed APM for call graphs, wall/CPU time, memory, I/O, and repeated production-like samples.
3. Doctrine SQL logger/profiler plus `EXPLAIN`/`EXPLAIN ANALYZE` on the supported database.
4. Stopwatch/PSR-3 metrics around the suspected service or external boundary.
5. Messenger worker metrics, transport depth, retry/failure rate, processing time, and memory growth.
6. `phpbench` or a focused repeatable harness for CPU-bound code when already configured.

Form one hypothesis at a time and connect it to measured evidence.

## Doctrine And Database

- Eliminate N+1 access with an intentional fetch join, projection/read model, batch query, or extra-lazy association as appropriate.
- Select only required data for read-heavy lists; avoid partial entities that may later be treated as complete managed objects.
- Align equality/range/order predicates with composite index order and verify the actual plan/cardinality.
- Bound result sets and use deterministic pagination; prefer keyset pagination when large offsets dominate cost.
- Avoid unbounded `IN` clauses, collection hydration, cascade operations, and flush loops. Batch and clear carefully for long commands/workers.
- Review transaction duration, locks, deadlocks, retry behavior, and connection use before adding concurrency.
- Never add an index without assessing write/storage cost and migration lock behavior.

## Symfony Runtime And Rendering

- Keep production debug off; warm and verify cache during deployment rather than clearing it destructively in request paths.
- Verify OPcache/preloading/runtime settings through deployment configuration, not source-code toggles.
- Reduce repeated container/config work only after profiling; compiled private services and autowiring are normally not the bottleneck.
- In Twig, measure expensive includes/components, repeated property access, translation loops, and lazy Doctrine access.
- In Serializer/API Platform, constrain groups/depth, avoid circular graphs, and use explicit read models/providers for large collections.
- Cache only data with a defined key, scope/tenant boundary, TTL, invalidation owner, stampede behavior, and observability.

## Messenger, External I/O, And Long Processes

- Keep handlers thin and services idempotent; tune worker count/prefetch/time/memory limits from measured throughput and downstream capacity.
- Investigate retry storms, poison messages, failure transport growth, serialization size, and database connection/memory retention.
- Batch safely and restart workers during deployments when code/container state changes.
- Set outbound HTTP connect/total timeouts, bounded retries with jitter, circuit/concurrency limits, and response-size constraints.
- Move work async only when latency/retry/isolation benefits justify queue complexity and consistency implications.

## Change And Re-measure

Make the smallest change that tests the top hypothesis. Repeat the same scenario and report before/after values, sample count, variance, cache state, and trade-offs. Re-run correctness, authorization, and concurrency tests.

Reject optimizations that merely move cost to another request, worker, tenant, deployment step, or failure path without documenting that trade-off.

## Regression Protection

- Add a query-count assertion only when stable and meaningful.
- Add repository integration tests for changed query shape and results.
- Add a benchmark/performance budget when the project has reliable infrastructure for it.
- Record monitoring thresholds and dashboards for production-sensitive improvements.
- Document cache invalidation, worker tuning, indexes/migrations, and rollback requirements.

Use [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md) to keep query optimization in repositories/query services and orchestration in application services. Do not trade boundary clarity for an unmeasured micro-optimization.

## Report Template

```text
Scenario/environment:
Baseline:
Evidence and root cause:
Change:
After:
Correctness checks:
Operational trade-offs:
Remaining bottlenecks/risks:
```

Include exact commands/tools and clearly label unavailable tooling or non-production measurements. Include Context Summary and Next Steps.
