---
name: api-designer
description: Design versioned WordPress REST API contracts before implementation, including routes, schemas, permissions, field contexts, pagination, errors, compatibility, caching, and client impact.
phase: planning
flow-next: rest-api
flow-alternatives: [writing-plans, architecture-implementer, coder]
related: [architect, content-modeling, documentation-generator]
---

# WordPress API Designer

## Procedure

1. Identify consumers, authentication method, supported versions, existing
   namespace/routes/controllers, data ownership and compatibility promises.
2. Design a stable vendor namespace and explicit version. Define collection,
   item and action routes with correct HTTP methods and retry/idempotency
   behavior.
3. Specify path/query/body argument schemas: types, required/default values,
   enums, formats, bounds, sanitization and domain validation.
4. Define `permission_callback` behavior per route and object. Separate public,
   authenticated and privileged fields using response contexts where useful.
5. Specify response schemas, `_links`, embedding, pagination limits/headers,
   cache semantics and stable `WP_Error` codes/statuses. Never expose raw
   database objects or arbitrary metadata.
6. Plan N+1 avoidance, field preparation, write conflicts, backward-compatible
   additive changes, deprecation and the next-version migration path.
7. Define tests for public/authenticated/unauthorized/invalid/missing/conflict,
   schema, pagination, field visibility and legacy clients.

## Output

Return route table, request/response/error schemas, permission matrix,
pagination/cache/versioning decisions, implementation/test plan, Context
Summary, and next step (`rest-api`, `writing-plans`, or `coder`).
