---
name: repository-reviewer
description: Review Symfony Doctrine repositories for query correctness, parameter binding, N+1 risk, indexes, pagination, and persistence boundaries.
phase: quality
flow-next: code-reviewer
flow-alternatives: [performance-optimization, database-designer, coder]
---

# Symfony Repository Reviewer

Check:

- QueryBuilder/DQL/SQL uses bound parameters.
- Method names are business-readable.
- Queries match indexes and sort order.
- Pagination is stable and bounded.
- N+1 risk is understood and tested.
- Partial selects/hydration are used when appropriate.
- Writes happen through clear service transaction boundaries.
- Repositories do not authorize, format responses, or dispatch side effects.

Output findings with query impact and concrete fixes.

## Review Method

1. Read the calling service/use case and confirm the repository method expresses an application need rather than leaking QueryBuilder details.
2. Verify every untrusted value is bound with an explicit parameter/type where ambiguity matters.
3. Review joins, fetch strategy, hydration, selected columns, result cardinality, and duplicate-row behavior.
4. Compare filters and ordering with actual indexes, including column order and selectivity.
5. Check deterministic pagination, maximum page sizes, count-query cost, and cursor/keyset suitability.
6. Review locking and transaction assumptions for concurrent state changes.
7. Require integration tests for custom DQL/SQL, platform-sensitive behavior, and empty/boundary result sets.

Use query logs, Symfony Profiler, database explain plans, and measured production-like data before labeling a query slow. Do not move business policy or authorization into a repository to make a query convenient.

Compare query ownership, parameter binding, stable pagination, and transaction boundaries with [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md). Require evidence before introducing a generic repository abstraction or cache layer.
