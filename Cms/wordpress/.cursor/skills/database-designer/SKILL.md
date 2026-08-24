---
name: database-designer
description: Design WordPress data storage and migrations across posts, taxonomy, metadata, options, transients, users, and custom tables using real query, scale, lifecycle, multisite, and compatibility requirements.
phase: planning
flow-next: architecture-implementer
flow-alternatives: [content-modeling, coder, writing-plans]
related: [architect, plugin-development, performance-optimization]
---

# WordPress Database Designer

## Storage Decision

1. Capture ownership, cardinality, expected growth, reads/writes, filters,
   sorting, joins, uniqueness, retention, export/erase, multisite scope and
   migration requirements.
2. Use posts/taxonomy for editorial content and classification; metadata for
   bounded attributes; options/site options for small configuration; transients
   for disposable cached values only; custom tables for high-volume,
   transactional or relational access that core tables cannot serve reliably.
3. Reject large autoloaded options, unbounded serialized arrays, meta-query
   pseudo-relations, transients as durable queues, and direct assumptions about
   core table internals when public APIs exist.

## Custom Tables and Queries

- Derive names from `$wpdb->prefix` or `$wpdb->base_prefix` according to site
  versus network ownership.
- Define WordPress-compatible charset/collation, primary keys, bounded columns,
  uniqueness and indexes from real query shapes. Verify hot queries with
  `EXPLAIN` on representative data.
- Use `$wpdb->prepare()` for values. Dynamic identifiers are selected from an
  allowlist, never user input. Handle `null`, booleans and numeric types
  intentionally.
- Use `dbDelta()` only for changes it supports; do not pretend it is a general
  rollback-capable migration engine.

## Migration Contract

Track a schema/data version. Upgrades are idempotent and concurrency-safe.
Large changes use expand/backfill/read-switch/contract phases and bounded
WP-CLI/background batches with progress and recovery. Network activation must
cover existing and newly created sites without synchronous unbounded loops.
Destructive changes require explicit backup, rollback and user approval.

## Output

Return storage decision, schema/indexes, query contracts, schema-version and
rollout plan, multisite/privacy/cache implications, tests, Context Summary,
and next step.
