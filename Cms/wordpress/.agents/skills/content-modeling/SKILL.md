---
name: content-modeling
description: Model WordPress content with post types, taxonomies, metadata, users, terms, options, and editor/REST exposure. Use when deciding registration, capabilities, rewrite behavior, storage ownership, or content migration.
phase: planning
flow-next: coder
flow-alternatives: [database-designer, rest-api, writing-plans]
related: [architect, plugin-development, block-development]
---

# Content Modeling

## Procedure

1. Model editor/user behavior, ownership, lifecycle, querying, relationships,
   permissions, scale, exportability, and API needs before choosing storage.
2. Use a post type for content with editorial lifecycle/permalinks; taxonomy
   for shared classification; metadata for bounded attributes; options for
   configuration; user/term meta only when ownership truly belongs there.
   Choose custom tables for high-volume transactional or relational access
   that core tables cannot support predictably.
3. Register post types and taxonomies on `init` with stable keys, explicit
   labels, capabilities, REST exposure, rewrite behavior, supports, visibility,
   and deletion semantics. Map capabilities when object-level control matters.
4. Register metadata with type, single/multiple shape, sanitization,
   authorization, default, and REST schema. Do not expose private metadata by
   setting `show_in_rest` casually.
5. Avoid unbounded serialized structures and meta-query-heavy pseudo-relations.
   Specify expected cardinality and hot query shapes, then validate indexes and
   caching implications.
6. Plan rewrite changes, CLI/backfill migration, rollback, redirects, export,
   and compatibility for existing IDs/URLs/API consumers.
7. Test registration, capabilities, editor visibility, REST shape, query
   behavior, deletion, migration, and multisite scope.

## Output

Return the content model, registration contract, storage rationale, capability
and REST exposure, migration/query risks, tests, Context Summary, and next step.
