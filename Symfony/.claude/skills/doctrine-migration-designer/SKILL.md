---
name: doctrine-migration-designer
description: Design safe Doctrine migrations, backfills, indexes, constraints, rollout, rollback, and production data-safety plans.
phase: planning
flow-next: coder
flow-alternatives: [database-designer, writing-plans, verify]
---

# Doctrine Migration Designer

Plan migrations before implementation:

- Entity metadata changes and generated migration review.
- Online-safe rollout order: additive columns, backfills, constraints, not-null changes, cleanup.
- Index creation strategy and lock risk.
- Foreign keys, unique constraints, check constraints, and delete behavior.
- Data backfill commands or one-off scripts.
- Rollback limitations and recovery plan.
- Tests and verification: migration up/down where safe, schema validation, repository coverage.

Never approve destructive migrations, table drops, or irreversible data loss without explicit user consent.

## Production-Safety Workflow

1. Inspect entity metadata, current migrations, database platform/version, table size, write volume, and deployment topology.
2. Separate schema expansion, application compatibility, data backfill, constraint enforcement, and cleanup into independently deployable stages.
3. Review generated SQL manually. Doctrine-generated output is a starting point, not proof of production safety.
4. For large tables, assess rewrite/lock behavior, online index capabilities, transaction duration, replication lag, and statement timeouts.
5. Make backfills resumable, idempotent, observable, and bounded. Prefer a dedicated command/service over an unbounded migration loop.
6. Add uniqueness, foreign keys, checks, and not-null constraints only after existing data is proven valid.
7. Define forward recovery when `down()` cannot safely restore deleted or transformed data.

Keep data backfill orchestration in a dedicated command/service when it needs batching, retries, or observability; migrations should remain bounded and deterministic. See [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md) for service, repository, and console ownership.

## Deliverable

Document migration classes, exact rollout order, compatibility window, backfill ownership, indexes/constraints, expected locks, verification queries, monitoring, abort conditions, rollback limitations, and tests. Include `doctrine:migrations:migrate` and schema validation commands appropriate to the project.
