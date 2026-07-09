---
name: database-designer
description: Design relational schemas and data-access patterns for native PHP projects. Use for table design, keys, indexing, constraints, normalization, migrations, and safe PDO access. Triggers on "schema", "database design", "table", "index", "migration", "data model", "normalize".
phase: planning
flow-next: architecture-implementer
flow-alternatives: [coder, writing-plans, api-designer]
related: [architect, coder, performance-optimization]
---

# Database Designer

## Overview

Design a relational schema that enforces correctness at the database level and supports the application's real query patterns. Native PHP means the database is accessed through PDO (or a documented data layer), not an ORM, so the schema and query shapes carry more of the integrity burden.

## Design Workflow

1. **Model the domain.** Identify entities, their attributes, and relationships (1:1, 1:N, N:M).
2. **Normalize (then decide).** Aim for 3NF to avoid update anomalies; denormalize only for a measured read-performance reason, and document it.
3. **Choose keys.** Prefer a stable primary key (auto-increment `BIGINT` or UUID). Use UUIDv7/ULID when you need externally safe, sortable IDs.
4. **Enforce integrity.** Add `NOT NULL`, `UNIQUE`, `FOREIGN KEY` (with intentional `ON DELETE`/`ON UPDATE`), and `CHECK` constraints. Push invariants into the schema, not only the app.
5. **Index for queries.** Add indexes for real `WHERE`/`JOIN`/`ORDER BY` columns and composite indexes in the right column order; verify with `EXPLAIN`. Do not over-index (write cost).
6. **Plan migrations.** Express every change as a versioned, reversible migration (Phinx, Doctrine Migrations, or reviewed SQL with an up/down).

## Schema Example

```sql
CREATE TABLE invitations (
    id           BIGINT UNSIGNED NOT NULL AUTO_INCREMENT PRIMARY KEY,
    email        VARCHAR(255) NOT NULL,
    role         ENUM('trainer', 'player', 'parent') NOT NULL,
    token_hash   CHAR(64) NOT NULL,
    accepted_at  DATETIME NULL,
    expires_at   DATETIME NOT NULL,
    created_at   DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE KEY uq_invitations_token_hash (token_hash),
    KEY idx_invitations_email (email),
    KEY idx_invitations_expires_at (expires_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
```

## Data-Access Patterns (PDO)

- Always use prepared statements with bound parameters; never concatenate input.
- Set `PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION` and `PDO::ATTR_EMULATE_PREPARES => false`.
- Wrap multi-statement writes in transactions; roll back on failure.
- Avoid N+1: fetch related rows with a join or a single `WHERE id IN (...)` batch.
- Select only needed columns on hot/wide tables; paginate with `LIMIT`/keyset for large sets.

```php
$statement = $pdo->prepare(
    'SELECT id, email, role FROM invitations WHERE expires_at > :now AND accepted_at IS NULL'
);
$statement->execute(['now' => (new DateTimeImmutable())->format('Y-m-d H:i:s')]);
$rows = $statement->fetchAll(PDO::FETCH_ASSOC);
```

## Migration Safety

- Every migration has a tested rollback (down).
- Large tables: prefer additive changes; backfill in batches; avoid long locks. Add a nullable column, backfill, then add the constraint.
- Never edit a released migration; add a new one.
- Keep migrations independent of application code so they replay cleanly.

## Transactions, Isolation, And Locking

- Keep transactions short; do no slow work (external calls, heavy computation) inside them.
- Know the isolation level: MySQL/InnoDB defaults to `REPEATABLE READ`, PostgreSQL to `READ COMMITTED`. Raise the level only when a specific anomaly (non-repeatable read, phantom) must be prevented.
- Prevent lost updates with optimistic locking (a `version` column checked on `UPDATE ... WHERE version = :v`) or pessimistic locking (`SELECT ... FOR UPDATE`) for hot rows.
- Deadlocks: acquire locks in a consistent order, keep transactions small, and retry the transaction on deadlock (SQLSTATE 40001 / 1213) with backoff.

## Zero-Downtime (Expand/Contract) Migrations

Never break a running deploy. Split risky schema changes into phases:

1. **Expand:** add the new nullable column/table/index (online where supported); do not remove anything yet.
2. **Backfill:** populate in batches to avoid long locks; deploy code that writes both old and new.
3. **Migrate reads:** switch the app to read the new shape.
4. **Contract:** once nothing uses the old shape, drop it in a later migration.

- Renames = add new + backfill + switch + drop (never a bare `RENAME` on a live column).
- Add indexes concurrently where the engine supports it (`CREATE INDEX CONCURRENTLY` in PostgreSQL) to avoid write locks.

## Soft Deletes And Auditing

- Use soft deletes (`deleted_at` nullable) only when history/recovery is required; otherwise hard-delete to keep the model simple. Remember unique constraints must account for soft-deleted rows.
- For auditing, prefer `created_at`/`updated_at` plus, when required, an append-only audit/event table rather than scattering triggers.

## Data Types To Get Right

- Money: `DECIMAL(…, …)` or integer minor units, never `FLOAT`/`DOUBLE`.
- Timestamps: store UTC; be explicit about `TIMESTAMP` vs `DATETIME` (and timezone handling per engine).
- Text/charset: `utf8mb4` on MySQL; size `VARCHAR` to real limits.
- Booleans/enums: native `BOOLEAN` / `ENUM` or a constrained `TINYINT`/lookup table.

## Review Checklist

- Are relationships enforced with foreign keys?
- Is every uniqueness rule a `UNIQUE` constraint, not just app validation?
- Are the indexes justified by real queries (checked with `EXPLAIN`)?
- Are money/decimal values stored as `DECIMAL`, not float?
- Are timestamps and charsets/collations consistent?
- Is PII minimized and are secrets/tokens hashed, not stored raw?

## Final Output

Return the schema (DDL or migration), key/index/constraint decisions, access-pattern notes, rollout/backfill risks, Context Summary, and next step (`/architecture-implementer`, `/coder`, or `/writing-plans`).
