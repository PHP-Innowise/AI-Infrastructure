---
name: database-designer
description: Design relational schemas and Eloquent data-access patterns for Laravel projects. Use for table design, keys, indexing, constraints, normalization, migrations, and Eloquent relationships. Triggers on "schema", "database design", "table", "index", "migration", "data model", "normalize".
phase: planning
flow-next: architecture-implementer
flow-alternatives: [coder, writing-plans, api-designer]
related: [architect, coder, performance-optimization, eloquent, queues-jobs]
---

# Database Designer

## Overview

Design a relational schema that enforces correctness at the database level and supports the application's real query patterns. Laravel accesses the database through Eloquent (and the query builder for anything Eloquent doesn't express well), with migrations as the versioned source of truth for schema — the schema and query shapes still carry the integrity burden; Eloquent does not replace `unique()`, foreign keys, or `CHECK` constraints.

## Design Workflow

1. **Model the domain.** Identify entities, their attributes, and relationships (1:1, 1:N, N:M) — these map directly to Eloquent relationship methods (`hasOne`, `hasMany`, `belongsTo`, `belongsToMany`).
2. **Normalize (then decide).** Aim for 3NF to avoid update anomalies; denormalize only for a measured read-performance reason, and document it.
3. **Choose keys.** Prefer a stable primary key (auto-increment `id()` or UUID via `uuid()`/`ulid()` migration helpers). Use UUIDv7/ULID (`$table->ulid()`, and `HasUlids`/`HasUuids` traits on the model) when you need externally safe, sortable IDs.
4. **Enforce integrity.** Use migration column modifiers (`nullable()`, `unique()`) and `foreignId(...)->constrained()->cascadeOnDelete()` (or the appropriate `on delete` action), plus raw `CHECK` constraints via `DB::statement()` when the schema builder doesn't expose them. Push invariants into the schema, not only Eloquent validation.
5. **Index for queries.** Add indexes for real `WHERE`/`JOIN`/`ORDER BY` columns via `$table->index([...])` and composite indexes in the right column order; verify with `EXPLAIN`. Do not over-index (write cost).
6. **Plan migrations.** Express every change as a versioned, reversible Artisan migration (`up()`/`down()`), generated with `php artisan make:migration`.

## Schema Example

```php
<?php

declare(strict_types=1);

use Illuminate\Database\Migrations\Migration;
use Illuminate\Database\Schema\Blueprint;
use Illuminate\Support\Facades\Schema;

return new class extends Migration
{
    public function up(): void
    {
        Schema::create('invitations', function (Blueprint $table): void {
            $table->id();
            $table->string('email');
            $table->enum('role', ['trainer', 'player', 'parent']);
            $table->string('token_hash', 64)->unique();
            $table->timestamp('accepted_at')->nullable();
            $table->timestamp('expires_at');
            $table->timestamps();

            $table->index('email');
            $table->index('expires_at');
        });
    }

    public function down(): void
    {
        Schema::dropIfExists('invitations');
    }
};
```

## Data-Access Patterns (Eloquent)

- Use Eloquent models or the query builder with bound parameters; never interpolate raw input into `DB::raw()` or `whereRaw()`.
- Set `strict` mode on the MySQL connection (`config/database.php` `'strict' => true`, Laravel's default) so invalid dates/values are rejected rather than silently coerced.
- Wrap multi-statement writes in `DB::transaction()`; it rolls back automatically on any thrown exception.
- Avoid N+1: eager-load related rows with `with()`/`load()`, or use `withCount()` for aggregate counts instead of looping and querying per row.
- Select only needed columns on hot/wide tables (`select([...])`); paginate large sets with `paginate()`/`simplePaginate()`, or `cursorPaginate()` for keyset-style pagination.

This skill designs the schema and the basic access patterns it implies; for deeper model-layer behavior once the schema exists — polymorphic relationships, custom accessor/cast classes, query scopes, model events/Observers, and large-dataset iteration (`chunk()`/`cursor()`) — see the `eloquent` skill.

```php
$invitations = Invitation::query()
    ->select(['id', 'email', 'role'])
    ->whereNull('accepted_at')
    ->where('expires_at', '>', now())
    ->get();
```

## Model Factories And Seeders

- Every model that needs test fixtures should have a factory (`php artisan make:model Invitation -f` or `make:factory InvitationFactory`) defining realistic default state via `Faker`/`fake()`.
- Use factory states (`->trainer()`, `->expired()`) for common variations instead of overriding attributes ad hoc in every test.
- Use seeders (`php artisan make:seeder`) for reference/lookup data and local dev fixtures; keep production seed data (if any) idempotent (`updateOrCreate()`).

```php
Invitation::factory()
    ->count(3)
    ->expired()
    ->create(['role' => 'player']);
```

## Migration Safety

- Every migration has a tested `down()` rollback.
- Large tables: prefer additive changes; backfill in batches (e.g. a queued job — see `queues-jobs` skill — or `chunkById()`); avoid long locks. Add a nullable column, backfill, then add the constraint in a follow-up migration.
- Never edit a released (already-deployed) migration; add a new one.
- Keep migrations independent of application/Eloquent model code (avoid referencing `App\Models\*` inside migrations) so they replay cleanly even as models evolve.

## Transactions, Isolation, And Locking

- Keep `DB::transaction()` closures short; do no slow work (external `Http::` calls, heavy computation) inside them.
- Know the isolation level: MySQL/InnoDB defaults to `REPEATABLE READ`, PostgreSQL to `READ COMMITTED`. Raise the level only when a specific anomaly (non-repeatable read, phantom) must be prevented (`DB::transaction($callback, $attempts)` doesn't change isolation — set it explicitly via `DB::statement('SET TRANSACTION ISOLATION LEVEL ...')` if needed).
- Prevent lost updates with optimistic locking (compare a `version`/`updated_at` column before saving) or pessimistic locking (`Model::query()->lockForUpdate()->find($id)` inside a `DB::transaction()`) for hot rows.
- Deadlocks: acquire locks in a consistent order, keep transactions small, and let `DB::transaction($callback, $attempts = 3)` retry automatically on deadlock (SQLSTATE 40001 / 1213).

## Zero-Downtime Migrations

Never break a running deploy. Split risky schema changes into phases using Laravel's schema builder:

1. **Expand:** add the new nullable column/table/index (`Schema::table(...)->addColumn(...)`); do not remove anything yet.
2. **Backfill:** populate in batches (`chunkById()` in a command or queued job) to avoid long locks; deploy code that writes both old and new.
3. **Migrate reads:** switch the app/Eloquent accessors to read the new shape.
4. **Contract:** once nothing uses the old shape, drop it in a later migration (`Schema::table(...)->dropColumn(...)`).

- Renames = add new + backfill + switch + drop (never a bare `renameColumn()` on a live, actively-read/written column in a zero-downtime deploy).
- Add indexes concurrently where the engine supports it; Laravel's schema builder doesn't expose `CREATE INDEX CONCURRENTLY` directly, so use `DB::statement()` for PostgreSQL when avoiding write locks matters.

## Soft Deletes And Auditing

- Use Eloquent's `SoftDeletes` trait (`deleted_at` nullable column, added via `$table->softDeletes()`) only when history/recovery is required; otherwise hard-delete to keep the model simple. Remember unique constraints must account for soft-deleted rows (a composite unique index including `deleted_at`, or application-level checks scoped with `withTrashed()`).
- For auditing, rely on Eloquent's automatic `created_at`/`updated_at` timestamps plus, when required, an append-only audit/event table (or a package like `spatie/laravel-activitylog`) rather than scattering database triggers.

## Data Types To Get Right

- Money: `$table->decimal('amount', ...)` or integer minor units (`unsignedBigInteger` cents), never `float`/`double`.
- Timestamps: store UTC (Laravel's default); be explicit about `timestamp()` vs `dateTime()` (timezone-aware vs naive) per engine.
- Text/charset: `utf8mb4` on MySQL (Laravel's default charset since 5.7+); size `string()` columns to real limits.
- Booleans/enums: native `boolean()` / `enum()` column types, or a constrained lookup table for values that will grow.

## Review Checklist

- Are relationships enforced with foreign keys (`foreignId()->constrained()`), matching the Eloquent relationship methods?
- Is every uniqueness rule a `unique()` constraint, not just Form Request validation?
- Are the indexes justified by real queries (checked with `EXPLAIN`)?
- Are money/decimal values stored as `decimal`, not float?
- Are timestamps and charsets/collations consistent?
- Is PII minimized and are secrets/tokens hashed, not stored raw?

## Final Output

Return the schema (migration code), key/index/constraint decisions, Eloquent relationship/access-pattern notes, rollout/backfill risks, Context Summary, and next step (`/architecture-implementer`, `/coder`, or `/writing-plans`).
