---
name: database-designer
description: "Design Symfony/Doctrine relational models: entities, relationships, keys, indexes, constraints, migrations, repository query patterns, and safe rollout."
phase: planning
flow-next: architecture-implementer
flow-alternatives: [coder, writing-plans, api-designer]
---

# Symfony Database Designer

## Goal

Design a Doctrine-backed relational model that enforces correctness at the database level and supports real application query patterns.

## Required Decisions

- Entities and table names.
- Identifiers: integer, UUID/ULID, natural keys, external IDs.
- Relationships and owning/inverse sides.
- Nullability and default values.
- Unique constraints and check constraints.
- Foreign keys and delete behavior.
- Indexes for real repository queries.
- Pagination strategy and sort stability.
- Migration rollout and backfill steps.
- Repository methods and query shapes.

## Symfony/Doctrine Rules

- Use Doctrine migrations for schema changes.
- Critical uniqueness belongs in a unique index, not just a service check.
- Use repository methods with business names.
- Bind query parameters; never concatenate user input into DQL/SQL.
- Watch for N+1 queries, accidental eager loading, and oversized result hydration.
- Prefer explicit indexes that match filters, joins, and sort orders.
- Consider optimistic locking or pessimistic locks for concurrent state transitions.

Design from concrete read/write paths and invariants, not from entity diagrams alone. For each constraint and index, name the correctness rule or measured query it supports. Keep persistence mapping out of public API contracts and compare repository/transaction ownership with [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md).

## Output

Provide:

- Entity/table model.
- Relationships and constraints.
- Index plan with query justification.
- Migration/backfill/rollback notes.
- Repository method list.
- Tests needed.
- Risks and assumptions.
