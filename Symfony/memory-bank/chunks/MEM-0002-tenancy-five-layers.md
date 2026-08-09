---
{
  "id": "MEM-0002",
  "title": "Tenancy enforcement in five layers, two outside PHP",
  "type": "architecture",
  "status": "active",
  "scope": ["platform", "security"],
  "tags": ["tenancy", "row-level-security", "database-isolation"],
  "created": "2026-08-09",
  "last_verified": "2026-08-09",
  "review_after": "2027-02-09",
  "sources": [
    "specs/architect-architecture.md",
    "Task/app/docker/php/docker-entrypoint.sh",
    "Task/app/docker/postgres/initdb/00-roles.sh"
  ],
  "supersedes": [],
  "superseded_by": null
}
---

# Tenancy Enforcement in Five Layers, Two Outside PHP

## Durable Context

A trainer-scoped row cannot be read or written outside its tenant. Five independent layers enforce this; none is a superset of another. The two strongest — the mandatory tenant context and PostgreSQL Row-Level Security — fail loudly and fail closed respectively.

### Layer 1: Denormalized Tenant Key
Every trainer-scoped entity carries the trainer directly, never by join. This enables Layer 2's column-level filters and Layer 5's row policies. The trainer is the key rather than the membership because camp form submissions (Epic-08) have a trainer and no member (decision A3).

### Layer 2: Doctrine Filter
A tenant filter on every trainer-scoped entity, enabled by a `kernel.request` listener and by Messenger middleware. Covers DQL selects and lazy proxy initialisation. Kept even though RLS covers the same ground: the filter survives a misconfigured database role, and RLS survives a forgotten listener.

### Layer 3: Mandatory Tenant Context
`TenantContext` is required. Unresolved means exception, never "all". A worker, console command or webhook handler with no resolved tenant crashes on the first trainer-scoped query rather than silently reading the whole table. This converts RLS's fail-closed *silence* into a loud failure: without it, an unscoped batch job would quietly process zero rows forever.

### Layer 4: Post-Load Ownership Assertion
On Doctrine entity hydration: if the row's trainer is not the active tenant and no crossing scope is open, throw immediately. One comparison per hydrated object. **This is the only layer that closes the Doctrine identity-map hole** — a `find()` that hits the identity map issues no SQL, so no policy from Layer 2 or 5 is evaluated.

### Layer 5: PostgreSQL Row-Level Security
**RLS is IN for MVP**, enabled by startup gate and verified before container accepts traffic. It is the only layer covering native SQL, console commands and Messenger workers — all three of which this platform has. Retrofitting RLS later on populated data is where you discover which paths accidentally relied on cross-tenant reads, a breach-class discovery.

## Three Database Roles

- **Owner**: runs migrations and schema changes. Not subject to RLS; never used by the running application.
- **Application** (`pp_app`): what php-fpm and the worker connect as. Subject to RLS, holds `SELECT`/`INSERT`/`UPDATE`/`DELETE`, no `BYPASSRLS` or superuser.
- **Crossing** (`pp_crossing`): read-only with `BYPASSRLS`. Used only by `CrossTenantReadService`, which returns arrays and has no EntityManager, so it is structurally incapable of producing a managed entity.

`FORCE ROW LEVEL SECURITY` is deliberately **not** used, so the owner can migrate and backfill.

## Startup Gate

A container refuses to boot if:

1. The application connects as the owner or with `BYPASSRLS` (layer 5 would be silent).
2. RLS is not enabled on every table declared trainer-scoped.
3. Any trainer-scoped table lacks a policy.
4. The `DATABASE_URL` requests a persistent connection (layer 3's session variable would leak across requests).

The gate is implemented in `Task/app/docker/php/docker-entrypoint.sh` and runs before accepting requests.

## Consequences

When writing trainer-scoped access — queries, mutations, commands, webhook handlers:

- Assume all five layers will be enforced and tested. Remove one for "performance" only with evidence of a live bottleneck.
- RLS is fail-closed: a misconfigured application role **silently** stops applying. Layer 3 is what converts this into a crash.
- The identity-map hole is real: `find()` and `doctrine:dql` are the only safe entry points. Native SQL and identity-map hits both need Layer 4.
- Never disable the Doctrine filter inline. Its purpose is orthogonal to RLS: it survives a misconfigured role, and RLS survives a forgotten listener.

## Verification

The startup gate asserts the converse for the resolver's own sources: `AccountTrainerLink` and `PublicTenantCode` must carry **no** RLS policy. A migration that enabled RLS on either would break every login and every public route.
