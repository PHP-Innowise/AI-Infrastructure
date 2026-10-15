---
{
  "id": "MEM-0003",
  "title": "Tenant resolution before a tenant exists requires two global tables",
  "type": "architecture",
  "status": "active",
  "scope": ["platform", "security"],
  "tags": ["tenancy", "resolution", "circular-dependency"],
  "created": "2026-08-09",
  "last_verified": "2026-08-09",
  "review_after": "2027-02-09",
  "sources": [
    "specs/architect-architecture.md",
    "specs/council-sharelink-tenant-resolution.md"
  ],
  "supersedes": [],
  "superseded_by": null
}
---

# Tenant Resolution Before a Tenant Exists Requires Two Global Tables

## Durable Context

Two tables must stay global because tenant resolution happens before any tenant is established: `AccountTrainerLink` and `PublicTenantCode`. This is not an implementation choice; it is the precondition that breaks a fundamental circularity.

### The Circularity

**To learn which trainer a ShareLink belongs to, you must read `ShareLink`. To read `ShareLink`, you must already have that trainer as the tenant.**

`ShareLink` is trainer-scoped (architect-architecture.md line 328). When a player clicks a ShareLink code on the public `/join/{code}` route, there is no authenticated tenant yet. The resolver must look up the trainer before issuing any trainer-scoped query. But learning which trainer from the link requires reading the link, which requires the tenant already.

Every alternative to `PublicTenantCode` is therefore circular:

- The api-designer's proposal to read the link and hand the trainer to `MembershipService` requires reading a trainer-scoped row with no tenant. Circular.
- Ordering writes so `AccountTrainerLink` is created first requires setting `AccountTrainerLink.trainer` — the link's trainer. Must read the link. Circular.
- Resolving the code through `CrossTenantReadService` works structurally but makes it the application's hottest path and drowns the audit log with product usage (council-sharelink-tenant-resolution.md, options table).

### Two Sources, One Pattern

`PublicTenantCode` is a minimal, **global** mapping: code → trainer + opaque `(kind, referenceId)` pair. Nothing else. It breaks the circle by providing a tenancy-exempt lookup at `kernel.request` time.

This mechanism already existed silently for `Form` (Epic-08). The architecture's Layer 3 resolution source 5 ("A public code carried in the route") was performing this lookup for forms without naming the mechanism. `PublicTenantCode` gives source 5 a name and a second kind (`sharelink` and `form`).

Similarly, `AccountTrainerLink` is global for the same reason: the tenant resolver reads it *before* a tenant exists (architect-architecture.md line 311). It holds account, trainer, role-in-tenant and status — minimal enough to change rarely.

### Precedence Rule

On an allow-listed code-resolved route, the code's trainer outranks the session's selected trainer:

- Without this, an existing player clicking a second trainer's ShareLink resolves to their current trainer A and writes a membership for trainer B, rejected by RLS `WITH CHECK`.
- The precedence rule routes the write to the correct tenant.
- Resolution is not authorization: the tenant is derived from the code, never from the submitted payload. The voter on the ShareLink itself still decides whether the actor may proceed.

### Drift Is Benign By Construction

A `PublicTenantCode` row that outlives its `ShareLink` grants a tenant and nothing else. The trainer-scoped `ShareLink` load plus voter is the authority, so the user sees the same expired page either way. The mapping cannot drift into an authority.

## Consequences

When adding a new public code-resolved route or form kind:

1. Issue a code in `PublicTenantCode` via `PublicTenantCodeRegistry` (the only writer).
2. Declare the route in the allow-list so `TenantResolver` honors it.
3. The voter on the underlying entity (ShareLink, Form, etc.) is still the authority.
4. Stale mappings are harmless; new mappings are the only thing that matters.

Never attempt to replace this pattern with a service-opened scope or a crossing read on a player-facing or unauthenticated code-resolved route.

## Verification

The startup gate asserts that `AccountTrainerLink` and `PublicTenantCode` carry **no** RLS policy. A migration that enabled RLS on either would break every login and every public route, in a way that is hard to diagnose because the resolver would simply find nothing.
