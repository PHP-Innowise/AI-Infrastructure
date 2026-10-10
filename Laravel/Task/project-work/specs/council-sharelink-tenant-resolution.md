# Council verdict: tenant resolution on ShareLink acceptance

**Question.** A player or coach clicks a trainer's ShareLink and registers or
associates. `MembershipService` writes the **first** `PlayerTrainerMembership`
or `CoachMembership` row for that (account, trainer) pair. Which of
`architect-architecture.md`'s Layer 3 resolution sources sets the tenant for
that write — and if none does, what should?

**Status.** Recommendation only. Nothing in `architect-architecture.md` or
`api-designer-spec.md` is modified by this document; the exact amendment
wording is supplied below for the main session to apply.

**Inputs read.** `specs/architect-architecture.md` (Tenancy enforcement lines
120–209, Module map lines 44–78, Entity population lines 292–362, Cross-cutting
services lines 392–410, Decisions lines 645–678, Open architecture risks lines
728–776); `specs/api-designer-spec.md` (Identity module lines 375–433, Public
unauthenticated routes lines 857–915, Open questions lines 1166–1181);
`specs/requirements-analyst-epic-01-user-management-spec.md` (AC-01-9..15,
AC-01-17, AC-01-23, AC-01-31, AC-01-39..42, BR-01-11, BR-01-14/15, BR-01-27/28,
Data requirements line 316, Edge cases lines 323–341);
`specs/requirements-analyst-epic-03-crm-players-spec.md` (AC-03-2, AC-03-68,
BR-03-1/2, BR-03-20..23).

---

## Finding 1 — the gap is real. The api-designer **understated** it and
## mis-located it.

The api-designer did not overstate anything. Three corrections make the gap
larger, not smaller, and move its starting point one request earlier.

### 1a. It is a **read** failure before it is a write failure

`api-designer-spec.md` line 865 says the anonymous `GET /join/{code}` "Reads
the **global** `ShareLink` row by its code only". `ShareLink` is not global.
`architect-architecture.md` line 328 classifies it **trainer-scoped**
("`ShareLink` | Identity | Belongs to a trainer (BR-01-14/15/27)").

So on the anonymous GET, with no tenant resolved:

- Layer 3 throws before the query is issued ("Unresolved means exception,
  never 'all'", line 144).
- Even if it did not, Layer 2's filter and Layer 5's policy would return zero
  rows, and the landing page would 404 for **every** valid link.

The landing page cannot render. `MembershipService` is never reached. The gap
starts at `identity_sharelink_join_show`, not at
`identity_sharelink_join_register`.

### 1b. There is an anonymous trainer-scoped **write** on that same GET

`ShareLinkOpen` is trainer-scoped (`architect-architecture.md` line 329, "Click
log (BR-03-22)"). BR-03-22 requires each link click logged and counted;
BR-03-20 requires per-link opens; AC-03-59/61..63 and AC-01-73 surface those
counts to the trainer. The only route where a click happens is the anonymous
GET. `api-designer-spec.md` line 865's "no trainer-scoped write happens on this
request" is therefore incorrect unless click logging is moved off that route,
which no spec suggests and BR-03-22 does not permit.

### 1c. The **authenticated** branch mis-resolves, which is worse than not
### resolving

`identity_sharelink_associate` (`GET, POST /join/{code}/associate`) is gated
`ROLE_PLAYER` (`api-designer-spec.md` line 408). For an existing player already
associated with trainer A, source 3 **does** resolve — to **A** — because the
session holds A and `AccountTrainerLink` validates it. The request then writes a
membership for trainer **B**.

The five layers catch it, but as an opaque failure, not a decision:

- RLS `WITH CHECK` rejects the INSERT (`SQLSTATE 42501`, "new row violates
  row-level security policy") on the platform's mainline onboarding flow.
- Layer 4 fires on any subsequent hydration.

That is a 500 on AC-01-13 and AC-01-14 — the "player with an existing account
clicks a different trainer's ShareLink" case the epic names explicitly
(Epic-01 Edge cases, line 340). And it is a mis-resolution: if any single layer
were ever misconfigured, the row lands in the **wrong tenant** rather than
failing. "No source resolves" is a loud failure; "the wrong source resolves" is
a silent one.

The same shape recurs on `identity_portal_child_trainer_add`
(`api-designer-spec.md` line 432, AC-01-17/AC-01-23): a parent whose session
context is trainer A adding a child to trainer C.

### Correction table

| `api-designer-spec.md` claim | Actual | Consequence |
|---|---|---|
| line 865: `ShareLink` is "global" | Trainer-scoped (`architect-architecture.md` line 328) | The anonymous **GET** already fails; the gap starts one request earlier |
| line 865: GET performs "no trainer-scoped write" | `ShareLinkOpen` is trainer-scoped (line 329) and BR-03-22 requires the click log | An anonymous trainer-scoped **INSERT** is also uncovered |
| line 866: gap is on the two `PUBLIC_ACCESS` POSTs | Also on `identity_sharelink_associate` (`ROLE_PLAYER`) and `identity_portal_child_trainer_add` | The gap is not confined to unauthenticated requests, so it cannot be folded into source 5's "unauthenticated" wording |
| lines 870–905: framed as `MembershipService` needing a scope | The tenant is unknown *before* `MembershipService` is called, because learning it requires the trainer-scoped read | See Finding 2 |

Nothing here is invented. Every element is a row or line already in the settled
documents; the gap is that they were never read against each other.

---

## Finding 2 — the circularity that decides the question

**To learn which trainer a ShareLink belongs to, you must read `ShareLink`. To
read `ShareLink`, you must already have that trainer as the tenant.**

This single fact disposes of two of the four candidate mechanisms before any
trade-off is weighed:

- The api-designer's proposal has `MembershipService` open a scope **for the
  ShareLink's trainer**. It must be *handed* that trainer. The controller can
  only learn it by reading the trainer-scoped `ShareLink`. Circular.
- Ordering the writes so `AccountTrainerLink` is created first requires setting
  `AccountTrainerLink.trainer` — the ShareLink's trainer. Same read. Circular.

Therefore **a tenancy-exempt `code → trainer` lookup is not one option among
equals; it is the precondition for every option.** Once it exists, the tenant
is genuinely resolved at `kernel.request` by the component the architecture
already names for the job, and any *additional* scope-opening inside a domain
service is pure surplus attack surface.

**The architecture already requires this mechanism and never wrote it down.**
Source 5 ("The public form's shareable code", line 157) resolves the tenant for
`GET /forms/{code}`, and `Form` is **trainer-scoped** (line 361). Source 5 is
therefore already, silently, doing exactly this lookup. The recommendation below
gives that mechanism a name and a second caller rather than inventing one.

**And the architecture has already solved this shape twice.**
`AccountTrainerLink` is global for precisely this reason — "**The tenant
resolver's own source.** … Cannot be trainer-scoped: the resolver reads it
*before* a tenant exists" (line 311). `TrainerBillingSettings` was split out of
`Trainer` so the non-sensitive half could be read where the sensitive half must
not (line 656). The recommended fix is the same move a third time.

---

## Options considered

| # | Option | One-line shape |
|---|---|---|
| **A** | Order the writes: global `AccountTrainerLink` first, then source 3 resolves | Reuse an existing source by creating its input |
| **B** | api-designer's candidate: code-authorized scope opened by `MembershipService` | `AdministrativeScope`'s shape, self-triggered, no role gate |
| **C** | Membership creation becomes a `Platform` operation under an audited scope | Move the write to the module that owns tenancy |
| **D** | Reclassify `ShareLink` as global | Delete the read half of the problem |
| **E** | Reclassify the membership tables as global | Delete the write half of the problem |
| **F** | Resolve the code through `CrossTenantReadService` | Reuse the one existing crossing reader |
| **G** ✅ | **A route-scoped resolution source keyed on a public code, backed by a minimal global `code → trainer` mapping, resolved in `TenantResolver`** | Give source 5 a name, a mechanism and a second kind |

---

## Judged against the five stated criteria

Legend: ✅ good · ⚠️ acceptable with conditions · ❌ fails.

| Criterion | A (order writes) | B (service scope) | C (Platform op) | D (global ShareLink) | E (global memberships) | F (crossing read) | **G (7th source)** |
|---|---|---|---|---|---|---|---|
| **Cross-tenant write surface** | ❌ Any path able to insert an `AccountTrainerLink` grants itself a tenant. The resolver's own trust source becomes self-service | ❌ `MembershipService` has four callers (`Identity` ×2, `Scheduling`, `Forms`). A scope it can open for any trainer argument makes the surface "anywhere `MembershipService` is reachable", with no role gate — unlike `AdministrativeScope`, which requires `ROLE_SUPER_ADMIN` plus a voter | ⚠️ Bounded, but the scope is now a general `Platform` capability callable by four modules | ⚠️ Unchanged for writes; ❌ widens **reads** — trainer B reads trainer A's invite list | ❌ Dissolves tenancy on the CRM's most-read table | ✅ Read-only, `SELECT`-only connection | ✅ Bounded to an explicit route allow-list; the tenant is **derived from the code**, never from the payload |
| **"Exactly one code path writes trainer-scoped data"** | ⚠️ Preserved only by re-running resolution mid-request — the same weakening it was meant to avoid | ❌ Creates a **second setter of `TenantContext`** outside `TenantResolver`, which the architecture names as the single resolving component (line 399) | ⚠️ Preserved, but by relocating the writer | ✅ Untouched | ✅ Untouched | ✅ Untouched | ✅ One more enumerated source inside `TenantResolver`; `MembershipService` stays unaware of tenancy |
| **Survives a forged or stale code** | ❌ Cannot even be evaluated — it never learns the trainer (Finding 2) | ❌ Same circularity | ⚠️ Same circularity unless G is added underneath | ⚠️ Forged: 404. Stale: voter denies. But the write is still unresolved | ⚠️ Same | ✅ Forged → no row → no tenant → 404 | ✅ Forged → no tenant → 404 before any trainer-scoped statement. Stale → tenant resolves, `ShareLinkVoter::SHARELINK_RESOLVE` denies under a full context (AC-01-42) |
| **`Identity` remains sole creator of memberships** | ✅ | ✅ | ❌ Breaks it. `PlayerTrainerMembership` is an `Identity` entity; `Platform` "Must not… contain epic business logic" and "May call: Nothing" (line 51) | ✅ | ✅ | ✅ | ✅ Untouched — `MembershipService` gains no tenancy code at all |
| **Auditable** | ❌ The audit artifact is an `AccountTrainerLink` row that grants access, indistinguishable from one created legitimately | ⚠️ An `AdministrativeScope`-shaped entry per registration drowns the admin audit log with the product's most common flow — the architecture's own stated reason for rejecting crossing-based resolution (line 655) | ✅ Audited by construction | ⚠️ Nothing new to audit | ⚠️ Nothing left to audit | ❌ "Every crossing read writes an audit entry" (line 182) means **an anonymous scanner writes an audit entry per probe** — audit-log amplification | ✅ Audited at the two high-signal points (membership created; stale link denied), and **not** on unknown codes, which are rate-limited and counted |

---

## Council

**Symfony Maintainer.** Resolution belongs in `TenantResolver`, invoked from the
`kernel.request` listener the architecture already specifies (line 399) — that
is where the framework puts it and where every other source already lives. The
route allow-list should be a **route option or attribute** read from `_route`,
not an `if` in the listener, so the set of code-resolved routes is data on the
routes and greppable in one place. Two ordering constraints: the listener must
run **after** `RouterListener` (so `_route` exists) and **after** the firewall
(so sources 3 and 4 have a token), while still preceding any trainer-scoped
access — it is the same listener that enables the Doctrine filter. Option B is
also the least testable of the seven: a scope whose entire lifetime is inside
one service method cannot be asserted from `KernelBrowser` without instrumenting
the service, whereas resolver-based resolution is observable at the request
boundary.

**Doctrine Specialist.** The failing statement is the `INSERT`'s RLS
`WITH CHECK`, not a `SELECT` — which is why the mis-resolution in Finding 1c
surfaces as `SQLSTATE 42501` on a mainline flow rather than a clean 403. Two
schema notes. First, BR-01-28 requires ShareLink codes to be **unique**; codes
are bearer tokens in URLs, so uniqueness must be global, and a global mapping
carries that constraint naturally. Second, keep the mapping's payload to
`code`, `trainer`, and an opaque `(kind, referenceId)` pair — no Doctrine
association to `ShareLink` or `Form`, or `Platform` acquires a dependency on
`Identity` and `Forms` and violates line 51. Drift (a mapping row outliving its
`ShareLink`) is then **benign by construction**: the row grants a tenant and
nothing else, and the trainer-scoped `ShareLink` load plus voter is the
authority, so the user sees the same expired page.

**Security Reviewer.** The load-bearing sentence is **resolution is not
authorization**. A resolved tenant makes trainer-scoped tables *reachable*;
whether the actor may do anything inside remains a voter decision on the row the
code points at. That is what makes a stale code harmless and why resolving one
is safe. The tenant must be **derived from the code**, never accepted from the
submitted payload — otherwise a forged `trainer_id` field re-opens the whole
problem. Option B is the one I would block: it reproduces `AdministrativeScope`'s
shape while dropping its two safety properties (a `ROLE_SUPER_ADMIN` gate and a
voter), and it grants that capability to a service reachable from four modules.
Option F is a trap in the opposite direction: it looks safest because it reuses
the audited crossing reader, but attaching an anonymous, scannable route to the
`BYPASSRLS` connection and to an unconditional audit write turns the audit log
into the amplification target. Required alongside G: rate-limit the anonymous
code lookup, and treat unknown-code volume as a monitored metric.

**Test Lead.** Six tests make this real, all cheap:
1. Anonymous `GET /join/{unknown}` → 404 and **zero** trainer-scoped statements.
2. Anonymous `GET /join/{valid}` → 200 and exactly one `ShareLinkOpen` row under
   trainer T.
3. Register via a valid code → exactly one `PlayerTrainerMembership` under T with
   `source = 'sharelink'` and the ShareLink recorded (AC-03-68, BR-01-27).
4. **Regression for Finding 1c**: log in a player whose session context is
   trainer A, accept trainer B's code, assert the membership lands in **B** and
   no `SQLSTATE 42501` occurs.
5. Stale/exhausted code → tenant resolves, voter denies, expired page (AC-01-42),
   **no** membership row.
6. **Structural**: assert the allow-list against the router, so adding a route to
   the code-resolved set is a deliberate, reviewed act rather than a one-line
   attribute nobody notices.

**Operations Engineer.** Rate-limiting rides the PostgreSQL-backed cache pool
that already exists for BR-01-6's login limiter — no sixth container, consistent
with the deployment shape (line 618). One addition worth its weight: extend the
existing startup gate (line 285) to assert that the resolver's global tables —
`AccountTrainerLink` and the new mapping — carry **no** RLS policy. A future
migration that enables RLS on the resolver's own source would brick every public
route and every login in a way that is very hard to diagnose; the gate turns it
into a container that refuses to start, which is exactly the pattern already
established for the opposite failure.

**Pragmatic Tech Lead.** G costs one small global table, one registry service,
one branch in `TenantResolver`, and one route attribute. It reuses a pattern the
architecture has already applied twice, closes an unstated hole in source 5 for
free, and leaves `MembershipService` — the piece four modules call — completely
untouched. Option B is cheaper today by roughly nothing and creates a mechanism
that is very hard to remove once four callers exist. Take G.

**Verdict: unanimous for G.** Option B is rejected unanimously; the Security
Reviewer and Symfony Maintainer would block it.

---

## Recommendation

**Adopt option G: a route-scoped tenant resolution source keyed on a public
code, backed by a minimal `Platform`-owned global `code → trainer` mapping,
resolved by `TenantResolver` at `kernel.request`, on an explicit route
allow-list. Do not open a tenant scope inside `MembershipService` or any other
domain service.**

Generalize the existing source 5 rather than bolt on a seventh: source 5 already
performs this lookup for Epic-08's `Form` code without saying how. Give it a
mechanism and a second kind.

### Concrete wiring

**Module ownership — `Platform`.** It owns `TenantContext`, `TenantResolver`,
`AccountTrainerLink` and `AuditLogger`, and it is the bottom of the stack. No
other module gains tenancy code.

| Piece | Module | Shape |
|---|---|---|
| `PublicTenantCode` (entity) | `Platform` | **Global.** Columns: `code` (unique, URL-safe), `trainer`, `kind` (`sharelink` \| `form`), `referenceId` (opaque scalar — **no** Doctrine association, so `Platform` stays dependency-free). Nothing else: no expiry, no use count, no status. Those stay on the trainer-scoped `ShareLink`/`Form`, where they already are |
| `PublicTenantCodeRegistry` (service) | `Platform` | `issue(code, trainer, kind, referenceId)` / `revoke(code)`. The **only** writer of `PublicTenantCode` |
| `TenantResolver` (amended) | `Platform` | On an allow-listed route, reads `PublicTenantCode` by `{code}` and sets the tenant to its trainer. One extra branch |
| Route allow-list | `Platform` | A route attribute/option (e.g. `#[TenantFromPublicCode]`) on exactly: `identity_sharelink_join_show`, `identity_sharelink_join_register`, `identity_sharelink_associate`, `identity_sharelink_invite_show`, `identity_sharelink_invite_register`, `forms_public_show`, `forms_public_submit`, `forms_public_confirmation`, `forms_public_convert_account` |
| ShareLink creation | `Identity` | Calls `PublicTenantCodeRegistry::issue(...)` in the same transaction that persists the `ShareLink`. Per the boundary rule, `Identity` writes a `Platform` entity only through a `Platform` service |
| Form publication | `Forms` | Same call, `kind = 'form'` |
| `MembershipService` | `Identity` | **Unchanged.** It keeps requiring an explicit association source (line 403, AC-03-68) and stays free of tenancy concerns |
| `identity_portal_child_trainer_add` | `Identity` | Its ShareLink-code branch **redirects** to `/join/{code}/associate` rather than joining the allow-list, keeping code resolution on one route group. Its "select from My Trainers" branch needs the source-3 clarification below |

**What is audited.**

| Event | Audited? | Why |
|---|---|---|
| Membership created under a code-resolved tenant | ✅ `AuditLogEntry`: actor (or the account just created), resolved trainer, code kind, ShareLink id, association source, family members associated | This is the record that answers "who let this account into trainer T's tenant" |
| Code resolved a tenant, then the voter denied (inactive / expired / exhausted / trainer deactivated) | ✅ One entry | The enumeration signal — low volume, high value |
| Code not found | ❌ **Deliberately not audited.** Rate-limited per IP and exposed as a counter | Auditing it makes the audit log an amplification target. Same reasoning the architecture used to keep resolution out of `CrossTenantReadService` (line 655) |
| Link clicked | Already covered by `ShareLinkOpen` (BR-03-22) — not duplicated into the audit log | Client-required analytics, not a security record |
| Provenance, durable | Already covered: the membership row records its source and which ShareLink (AC-03-68, BR-01-27), which outlives audit-log retention | |

**Failure modes.**

| Situation | Behavior |
|---|---|
| Code absent from `PublicTenantCode` (forged, random, scanned) | No tenant resolved → **404**, indistinguishable from any unknown URL. **Zero** trainer-scoped statements issued. Rate-limited; counted, not audited |
| Code maps to a trainer, but the `ShareLink` is inactive / expired / exhausted | Tenant **is** resolved (harmless — resolution is not authorization). The trainer-scoped `ShareLink` loads under all five layers; `ShareLinkVoter::SHARELINK_RESOLVE` denies → "this invitation has expired" with the resend path (AC-01-42, BR-01-15). No membership row. Audited |
| Mapping row survives a deleted `ShareLink` (drift) | Tenant resolves; no `ShareLink` under that tenant; same expired/404 page. **Benign by construction** — the mapping grants a tenant and nothing else |
| Owning trainer deactivated or deleted | Same denial path; the voter checks the owning trainer's status |
| Logged-in **child** clicks a link | Tenant resolves; AC-01-31 branch runs (tell the child to ask a parent, email the parent); `MembershipService` is **not** called; no membership |
| Coach already active under another trainer | Tenant resolves; `CoachMembership` creation is refused by `Identity`'s own rule (BR-01-11, AC-01-41, Epic-01 Edge cases line 329) — a domain refusal, not a tenancy one |
| Submitted payload names a trainer other than the code's | **Impossible by construction.** The trainer is never read from the payload on these routes. If a payload field ever carries one, it is ignored; a mismatch is a hard failure |
| Existing player, session context A, accepts trainer B's code | Source 5 outranks source 3 on allow-listed routes → tenant is **B**. Finding 1c's `SQLSTATE 42501` cannot occur |

---

## Rejected options, and why each lost

- **B — the api-designer's `MembershipService`-opened scope. Lost on three
  counts, any one of them sufficient.** (i) It is **circular**: the service must
  be handed the ShareLink's trainer, which can only be learned by the
  trainer-scoped read that has not happened yet (Finding 2). (ii) It **starts too
  late** — the anonymous GET's `ShareLink` read and `ShareLinkOpen` write both
  precede any `MembershipService` call, so the flow still fails one request
  earlier (Findings 1a, 1b). (iii) It creates a **second setter of
  `TenantContext`** outside `TenantResolver`, granted to a service with four
  callers across three modules, reproducing `AdministrativeScope`'s shape while
  dropping its `ROLE_SUPER_ADMIN` gate and its voter. Once G exists the tenant is
  already genuinely set, so this scope buys nothing and costs the surface.

- **A — write the global `AccountTrainerLink` first.** Lost because it is
  **circular for the same reason** (it must set `trainer` = the ShareLink's
  trainer), and because it **inverts the meaning of the resolver's own source**:
  `AccountTrainerLink` exists to *record* an established relationship, not to
  *authorize* creating one. Making its insertion the act that grants tenant
  access turns the resolver's trust root into a self-service primitive, and any
  path that fails after the link but before the membership leaves an orphan grant
  with no membership behind it. It also still requires resolution to be re-run
  mid-request, so it does not avoid the mechanism it was proposed to avoid.

- **C — membership creation as a `Platform` operation.** Lost on the module map.
  `Platform` "May call: Nothing. It is the bottom of the stack" and "Must not…
  contain epic business logic" (line 51). `PlayerTrainerMembership` is an
  `Identity` entity; relocating its creation inverts the dependency direction
  (line 68) and breaks "`Identity` [is] the sole creator of memberships"
  (line 66) — one of the two invariants the boundary rule exists to protect. The
  auditing it was reaching for is achievable without moving a line of code.

- **D — reclassify `ShareLink` as global.** Lost on disclosure. The entity carries
  recipient email for unique links, creator, use counts and per-link analytics
  (Epic-01 Data requirements line 316; BR-03-20/21/22; AC-03-61..63). Global
  placement would let trainer B read trainer A's invitation list and recipients
  behind voters alone — the exact reasoning that split `TrainerBillingSettings`
  out of `Trainer` (line 656). And it fixes only the read half: the membership
  write is still unresolved. G takes the same idea but splits off **only the
  `code → trainer` pair**, which discloses nothing a holder of the code does not
  already have, given that `TrainerBrandingSettings` is deliberately public
  (line 310).

- **E — reclassify the membership tables as global.** Lost decisively. It puts
  the CRM's entire roster — trainer-set skill level, status, association source —
  behind voters only, on the most-read trainer-scoped table in the product, and
  contradicts BR-03-2 ("each trainer sees only their own association"). This is
  the precise failure the five layers exist to prevent.

- **F — resolve the code through `CrossTenantReadService`.** The most tempting
  rejection: it reuses an existing, audited, projection-only reader owned by the
  same module. Lost on two properties that reader derives its safety from.
  (i) "Every crossing read writes an audit entry" (line 182) is **unconditional**,
  so every anonymous scanner probe writes an audit entry — audit-log
  amplification, and the architecture rejected crossing-based resolution for the
  trainer switcher for exactly this reason (line 655). (ii) It attaches an
  anonymous, scannable, unauthenticated route to the `BYPASSRLS` connection.
  Carving the first exception into an unconditional rule costs more than the
  small global table G adds.

---

## Does this warrant amending the architecture's resolution-source list?

**Yes — unambiguously, and it is not optional.** Three reasons:

1. The list is **normative and ordered** ("Resolution sources, in order",
   line 150) and it is the document every later stage reads. A source that is
   required but absent will be invented ad hoc by whoever implements Epic-01
   first — most likely as option B, the one this council rejects.
2. The flow is the product's **most common** — every player and every coach
   enters the platform through it (BR-01-14, BR-01-15, BR-03-1).
3. Source 5 is **already relying on the unnamed mechanism** for `Form` (line 157
   vs. line 361). The amendment documents something the architecture already
   depends on; it does not add a capability.

### Exact wording to add

**(1) Replace source 3** (`architect-architecture.md` line 152–154) with:

> 3. The player's or coach's selected trainer context — held in the session, or
>    supplied by the request on a route whose purpose is to change or target it —
>    validated in either case against the global `AccountTrainerLink`. An
>    unvalidated candidate resolves nothing.

*(Required by AC-01-17/AC-01-23: a parent whose session context is trainer A
adds a child to trainer C. The security-bearing half — validation against
`AccountTrainerLink` — is unchanged; only where the candidate identifier may
come from is widened.)*

**(2) Replace source 5** (lines 156–158) with:

> 5. A **public code carried in the route**, on the routes explicitly
>    allow-listed as code-resolved: the Epic-08 form's shareable code, and an
>    `Identity` ShareLink code on the `/join/{code}` and `/invite/{code}` route
>    group. The code is looked up in the global `PublicTenantCode` mapping, which
>    holds the code, the owning trainer, and an opaque `(kind, referenceId)` pair
>    — nothing else. Like `AccountTrainerLink`, this mapping is global because the
>    resolver runs before a tenant does, and it is read for no purpose other than
>    resolution. The allow-list is declared as a route attribute in one place so
>    the surface is enumerable.

**(3) Add immediately after the numbered list** (after line 159):

> **Precedence.** Sources 1 and 2 outrank everything. On an allow-listed
> code-resolved route, source 5 outranks sources 3 and 4: the tenant is the
> code's trainer, never the session's selected context and never a trainer named
> in the submitted payload. Elsewhere, 3 then 4. Without this precedence, an
> existing player accepting a second trainer's ShareLink resolves to the trainer
> already in their session and the membership INSERT is rejected by RLS.
>
> **Resolution is not authorization.** A resolved tenant only makes the
> trainer-scoped tables reachable. Whether the actor may do anything inside that
> tenant is still a voter decision on the trainer-scoped row the code points at
> (`ShareLinkVoter::SHARELINK_RESOLVE`,
> `FormSubmissionVoter::FORM_SUBMISSION_CREATE`). A code absent from
> `PublicTenantCode` resolves no tenant and the route 404s before any
> trainer-scoped statement is issued; a code whose underlying `ShareLink` or
> `Form` is missing, inactive, expired or exhausted is denied by that voter under
> a fully established tenant context. A stale mapping row therefore grants
> nothing. Source 5 is the platform's only tenant resolution from an
> unauthenticated or bearer-code input, and code lookups are rate-limited.

**(4) Add to the Entity population → Global table** (after line 311):

> | `PublicTenantCode` | Platform | **The tenant resolver's second source.** Minimal mapping of a public code to its owning trainer, with an opaque kind and reference. Cannot be trainer-scoped: the resolver reads it *before* a tenant exists. Carries no expiry, status or use count — those stay on the trainer-scoped `ShareLink`/`Form`, so the mapping cannot drift into an authority |

**(5) Add to Cross-cutting services** (after line 399):

> | `PublicTenantCodeRegistry` | Issue and revoke the global `code → trainer` mapping. The only writer of `PublicTenantCode` | `Identity` (ShareLink creation), `Forms` (form publication) |

**(6) Add two rows to the Decisions table:**

> | ShareLink tenant resolution | A route-scoped code source (5) backed by a minimal global `code → trainer` mapping, resolved in `TenantResolver` | A scope opened by `MembershipService`; writing `AccountTrainerLink` first; moving membership creation to `Platform`; making `ShareLink` or the membership tables global; resolving via `CrossTenantReadService` | Learning a ShareLink's trainer requires reading a trainer-scoped row, so every alternative is circular without this mapping — it is the precondition, not an option. Once the tenant is genuinely resolved at `kernel.request`, a service-opened scope adds surface and buys nothing, and it would be a second setter of `TenantContext` outside the resolver. Source 5 already performed this lookup for `Form` without naming the mechanism |
> | Code-lookup auditing | Audit the membership created and the stale-link denial; rate-limit and count unknown codes without auditing them | Auditing every lookup, e.g. by routing it through `CrossTenantReadService` | An unconditional audit write on an anonymous, scannable route makes the audit log the amplification target — the same objection that kept per-request resolution out of the crossing service |

**(7) Amend the layering exception bullet** (line 116) from "The public form
route runs without a user" to:

> - **Code-resolved public routes run without a user, or with one whose session
>   context is a different trainer.** They resolve the tenant from the code in
>   the route (see Tenancy, resolution source 5). This covers Epic-08's form
>   routes and Epic-01's ShareLink acceptance routes; the allow-list is declared
>   on the routes themselves.

**(8) Widen Open architecture risk 7** (lines 762–763) from camp submissions
alone to: camp form submissions **and ShareLink acceptance** are unauthenticated
writes into trainer-scoped tables (`FormSubmission`, `ShareLinkOpen`,
`PlayerTrainerMembership`, `CoachMembership`) with the tenant resolved from a
public code; both need rate limiting and abuse review, which no epic specifies.

**(9) Extend the startup gate** (line 285) to assert that
`AccountTrainerLink` and `PublicTenantCode` carry **no** RLS policy — a
migration that enables RLS on the resolver's own sources would break every
public route and every login in a way that is hard to diagnose.

---

## Consequential corrections for `api-designer-spec.md` (for the main session; not applied here)

1. Line 865: `ShareLink` is trainer-scoped, not global; and `ShareLinkOpen`
   (BR-03-22) makes the anonymous GET a trainer-scoped **write**. Both GET rows
   change from "No" to "Yes — resolution source 5, code-resolved".
2. Line 866: the two POSTs change from "**Gap**" to "Yes — resolution source 5".
3. Add `identity_sharelink_associate` and `identity_portal_child_trainer_add` to
   the tenant-resolution table; the first is code-resolved, the second uses the
   clarified source 3 (or redirects for its code branch).
4. Lines 870–915: replace "The gap" with a pointer to this verdict.
5. Lines 1168–1181: the Open questions item is resolved; retain a one-line
   reference.
6. Line 867's "the only unauthenticated tenant resolution the architecture
   names" becomes "the only tenant resolution from a public code, shared with
   Epic-01's ShareLink routes".

---

## Risks of the recommendation

- **The allow-list is the whole security boundary.** A route added to it can
  resolve a tenant from a bearer code. *Cheapest early check*: the structural
  test above, asserting the allow-list against the router, so growth is a
  reviewed act.
- **A second global table is a second place tenancy does not apply.** Mitigated
  by keeping it to four columns with no expiry/status, so it can never become an
  authority, and by the startup-gate assertion.
- **Two-row write on ShareLink creation.** `ShareLink` plus `PublicTenantCode`
  must be created in one transaction. *Cheapest early check*: a test that a
  rolled-back ShareLink creation leaves no mapping row. Drift in the other
  direction is benign (see failure modes).
- **Code enumeration.** High-entropy URL-safe codes (Epic-01 Data requirements
  line 316) plus per-IP rate limiting on the lookup. A successful probe yields a
  registration page for a trainer whose branding is already public by design
  (line 310) — no roster, no data.
- **Unresolved and untouched by this verdict**: whether the rate-limit threshold
  matches BR-01-6's (unspecified) login threshold; Epic-01 leaves both open.

## Decision criteria — what would change this verdict

- If `ShareLink` were reclassified global for an independent reason, D's read
  half comes free — but the membership write still needs source 5, so G stands.
- If the client required the ShareLink landing page to disclose nothing at all
  until authenticated, the anonymous GET disappears and only the POSTs need
  source 5 — G shrinks but does not change shape.
- If a connection pooler ever entered the stack, the session-variable model
  breaks first (architecture Risks, line 691) and this decision is downstream of
  that, not independent of it.
- If `MembershipService` ever needed to write into a tenant **not** derivable
  from a code or from the actor's own `AccountTrainerLink`, option B would have
  to be reconsidered — but that requirement does not exist in any epic, and
  inventing it is out of scope.

---

*Council convened 2026-08-09. Recommendation only: `architect-architecture.md`
and `api-designer-spec.md` are unmodified. `Task/Epics/` and `Task/designs/`
were not written to.*
