# Design: PracticePerfect platform

Platform shape for a white-label, multi-tenant Symfony application covering
eight epics and 419 acceptance criteria, built under `Task/app/` and run
exclusively via Docker Compose.

**Inputs already settled — do not reopen.** Section A of
`specs/requirements-analyst-open-questions.md` (owner decisions A1–A12 and the
fee call); the council verdict on tenancy enforcement; the council verdict on
the token and payment ledger; the researcher's six stack recommendations. This
document records, completes and reconciles them. Where it goes beyond them it
says so in the Decisions table.

**This document does not contain schema.** Entities and relationships are
named; columns, types, keys and indexes belong to the database-designer stage
that runs next. Endpoint contracts belong to the api-designer stage after that.

---

## Approach

A **modular monolith**: one Symfony 7.4 application, one PostgreSQL database,
one shared schema, partitioned into nine modules that map to the epics. Inside
every module the layering is `Controller -> Service -> Repository`, with the
exceptions enumerated below. There is no service mesh, no second application,
no per-tenant deployment.

Three properties are load-bearing and everything else is arranged to protect
them:

1. **A trainer-scoped row cannot be read or written outside its tenant.** Five
   independent layers enforce this, and the two strongest — the mandatory
   tenant context and PostgreSQL Row-Level Security — fail loudly and fail
   closed respectively.
2. **The token ledger is append-only and reconstructible.** Balances are
   projections, refunds are compensating entries, and nothing is ever mutated.
3. **White-label branding is a value read at request time**, never a build
   input. A new trainer's brand ships without a deploy.

Everything is server-rendered Twig with progressive enhancement. Not a SPA.

---

## Module map

Namespace root `App\`. One directory per module; each carries its own
`Controller/`, `Service/`, `Repository/`, `Entity/`, `Voter/`, `Dto/`.

| Module | Epic | Owns | May call | Must not |
|---|---|---|---|---|
| `Platform` | — | Tenancy kernel, tenant context and resolver, cross-tenant read service, branding runtime, audit log writer, impersonation and administrative tenant scope, `Trainer` tenant registry | Nothing. It is the bottom of the stack | Depend on any other module; contain epic business logic |
| `Identity` | 01 | Account, profiles, parent/child links, player profiles, coach and player memberships, ShareLinks, availability, child approval requests, auth | `Platform` | Know about events, money, content or forms |
| `Scheduling` | 02 | Events, RSVPs, attendance, coach assignments, invitations, duplication | `Platform`, `Identity`, `Billing` (to take payment), `Growth` (to price a coupon) | Write token entries directly; write CRM state |
| `Crm` | 03 | Labels, flags, notes, segmentation, Quick View | `Platform`, `Identity`, `Scheduling` (read), `Content` (read) | Write memberships except through `Identity`; write attendance |
| `Content` | 04 | Playlists, content items, drills, assignments, progress, access grants, usage | `Platform`, `Identity`, `Billing` (to take payment), `Crm` (read labels for group assignment) | Write token entries directly; write CRM state |
| `Billing` | 05 | Token ledger, balances, payment records, Stripe gateway, Connect settings, entitlements, webhook receipts | `Platform`, `Identity` | Call `Scheduling`, `Content`, `Forms` or `Growth`. Billing is called, it does not call up |
| `Growth` | 06 | Referral links, referrals, assist counts, coupons, redemptions | `Platform`, `Identity`, `Billing` (to grant a reward entry) | Write token entries directly; own the reward record |
| `Administration` | 07 | Super Admin console, feature toggles, operational dashboards, Event Master tool, audit log viewer | `Platform` plus every module's services | Write any trainer-scoped row without an active administrative tenant scope; read another module's repositories |
| `Forms` | 08 | Camp and evaluation forms, public submission, conversion to account | `Platform`, `Identity` (to convert), `Billing` (to take payment) | Create a membership itself; write a payment record itself |

**Boundary rule.** A module may read another module's entities through that
module's repository or service. A module may **write** another module's
entities only through the owning module's service — never through its
repository, never by calling a setter on an entity it does not own. This is what
keeps `Billing` the sole writer of the ledger and `Identity` the sole creator of
memberships.

**Dependency direction.** `Platform` <- `Identity` <- everything else;
`Billing` is called by `Scheduling`, `Content`, `Growth` and `Forms` and calls
none of them back. Payment outcomes reach the caller by return value inside the
request, or by a domain event dispatched from `Billing` and subscribed to by the
caller — never by `Billing` reaching into another module.

**Build order** (A10, A11): 01 -> 02 -> {03, 04} -> 05 -> {06, 07, 08}.
`Content` is built structurally before `Billing` exists; the paywall is wired
last. Until `Billing` exists, `Content` and `Scheduling` depend on a narrow
payment-intent interface that `Billing` later implements.

---

## Layering, and where the exceptions are

The default is `Controller -> Service -> Repository`. Controllers resolve
input, authorize, delegate, and render. Services hold one cohesive workflow
each and own the transaction boundary. Repositories hold query and persistence
methods with business-readable names.

| Concern | Placement |
|---|---|
| Entry points | Controllers (HTTP), Messenger handlers (async), console commands (batch), the public form route (unauthenticated) |
| Input validation | Request DTOs plus Symfony Forms for browser posts; Validator constraints on the DTO, custom constraints for cross-field rules (capacity vs. current RSVPs, event end after start) |
| Authorization | Voters, always, for object-level decisions. `access_control` only for coarse firewall gates on route prefixes |
| Business workflow | One service per use case. `RsvpToEventService`, `CancelRsvpService`, `PurchasePlaylistAccessService`, `ConvertFormSubmissionService` |
| Transaction boundary | The service. Never a controller, never a repository, never a listener |
| Persistence and queries | Repositories. Read-heavy screens return DTO projections from the repository rather than entity graphs |
| Response | Twig view models for HTML; redirect-after-post for mutations |

**Deliberate exceptions, each justified:**

- **Doctrine `postLoad` listener.** The post-load ownership assertion (tenancy
  layer 4) lives in a Doctrine event listener. This is business logic in a
  listener only in the narrowest sense: it asserts an invariant and throws; it
  performs no workflow, dispatches nothing, and writes nothing.
- **Messenger middleware.** Tenant context is established and torn down per
  message by middleware, not by each handler. Handlers that forget would
  otherwise run unscoped.
- **Native SQL in reporting repositories.** CRM segmentation (BR-03-13/14,
  AND-combined filters), Top Players (BR-03-16, 90-day attendance), and the
  Epic-07 operational dashboards may use native SQL or DBAL query builders for
  aggregation. This is permitted **only because RLS covers native SQL**; it is
  the single largest reason RLS is in the MVP.
- **Voters call services.** `EventVoter` and `PlayerVoter` delegate coach reach
  to `CoachVisibilityService` and feature availability to `FeatureGate`. A voter
  restating a visibility rule is a defect, not a style choice.
- **Code-resolved public routes run without a user, or with one whose session
  context is a different trainer.** They resolve the tenant from the code in
  the route (see Tenancy, resolution source 5). This covers Epic-08's form
  routes and Epic-01's ShareLink acceptance routes; the allow-list is declared
  on the routes themselves.

---

## Tenancy enforcement

Five layers. Each one covers a hole the others leave open. None is a superset
of another, and that is why all five are present.

### Layer 1 — the trainer is the tenant key, denormalized

Every trainer-scoped entity carries the trainer directly, never by join. A SQL
predicate can only constrain a column on the table it filters, so this is the
enabling condition for layers 2 and 5. The trainer is the key rather than the
player-trainer membership because a camp form submission has a trainer and no
member (A3).

### Layer 2 — the Doctrine filter

A tenant filter on every trainer-scoped entity class, enabled by a
`kernel.request` listener and by the Messenger middleware. Covers DQL selects,
lazy proxy initialisation and joined classes. Kept even though layer 5 covers
the same ground, because the two fail in opposite directions: the filter still
works if the database role is misconfigured, and RLS still works if the listener
is forgotten.

### Layer 3 — the mandatory tenant context

`TenantContext` is required. **Unresolved means exception, never "all".** A
worker, console command or webhook handler with no resolved tenant crashes on
the first trainer-scoped query rather than silently reading the whole table.
This is what converts RLS's fail-closed *silence* into a loud failure: without
it, an unscoped batch job would quietly process zero rows forever.

Resolution sources, in order:

1. An active administrative tenant scope (Super Admin adopting a trainer).
2. An active impersonation session's subject.
3. The player's or coach's selected trainer context — held in the session, or
   supplied by the request on a route whose purpose is to change or target it —
   validated in either case against the global `AccountTrainerLink`. An
   unvalidated candidate resolves nothing.
4. The trainer's own tenant, for an account whose role is Trainer.
5. A **public code carried in the route**, on the routes explicitly
   allow-listed as code-resolved: the Epic-08 form's shareable code, and an
   `Identity` ShareLink code on the `/join/{code}` and `/invite/{code}` route
   group. The code is looked up in the global `PublicTenantCode` mapping, which
   holds the code, the owning trainer, and an opaque `(kind, referenceId)` pair
   — nothing else. Like `AccountTrainerLink`, this mapping is global because the
   resolver runs before a tenant does, and it is read for no purpose other than
   resolution. The allow-list is declared as a route attribute in one place so
   the surface is enumerable.
6. Otherwise: no tenant. Trainer-scoped access throws.

**Precedence.** Sources 1 and 2 outrank everything. On an allow-listed
code-resolved route, source 5 outranks sources 3 and 4: the tenant is the
code's trainer, never the session's selected context and never a trainer named
in the submitted payload. Elsewhere, 3 then 4. Without this precedence, an
existing player accepting a second trainer's ShareLink resolves to the trainer
already in their session and the membership INSERT is rejected by RLS.

**Resolution is not authorization.** A resolved tenant only makes the
trainer-scoped tables reachable. Whether the actor may do anything inside that
tenant is still a voter decision on the trainer-scoped row the code points at
(`ShareLinkVoter::SHARELINK_RESOLVE`,
`FormSubmissionVoter::FORM_SUBMISSION_CREATE`). A code absent from
`PublicTenantCode` resolves no tenant and the route 404s before any
trainer-scoped statement is issued; a code whose underlying `ShareLink` or
`Form` is missing, inactive, expired or exhausted is denied by that voter under
a fully established tenant context. A stale mapping row therefore grants
nothing. Source 5 is the platform's only tenant resolution from an
unauthenticated or bearer-code input, and code lookups are rate-limited.

*Why this exists (council verdict, `specs/council-sharelink-tenant-resolution.md`):
learning which trainer a ShareLink belongs to requires reading `ShareLink`,
which is itself trainer-scoped. Every alternative that resolves the tenant from
the link is therefore circular, and the mapping above is the precondition that
breaks the circle — not one option among several. Source 5 was already
performing this lookup for Epic-08's `Form` without naming a mechanism.*

### Layer 4 — the post-load ownership assertion

On hydration of any trainer-scoped entity: if the row's trainer is not the
active tenant and no crossing scope is open, throw. One comparison per hydrated
object. **This is the only layer that closes the Doctrine identity-map hole** —
see the RLS resolution below.

### Layer 5 — PostgreSQL Row-Level Security

**RLS is IN for MVP.** Rationale, mechanics and the exact division of labour
are in the next section, because this is the one point where the council and the
researcher differed.

### Crossing the boundary

Exactly one component reads across tenants: `CrossTenantReadService`. It uses a
**separate, read-only database connection** whose role carries `BYPASSRLS` and
holds `SELECT` only. Because that connection has no EntityManager behind it, it
is structurally incapable of returning a managed entity — it returns arrays.
The council's rule 5 ("projections, never managed entities") therefore stops
being a discipline and becomes a property of the wiring. Every crossing read
writes an audit entry.

The filter is never disabled inline anywhere else, and no other code opens a
second connection.

### Writing across tenants

Super Admin never writes cross-tenant. Two mechanisms, both of which set a
genuine tenant context before any write:

- **Impersonation** — acting as a specific user inside their tenant
  (BR-01-21/22). One hour, logged, cannot target another Super Admin.
- **Administrative tenant scope** — adopting a trainer's tenant without
  becoming a user, for the Event Master tool (US-07.07) and per-trainer fee
  edits (AC-05-27). Explicit, audited, and it sets the same tenant context and
  the same database session variable as any other resolution.

Net effect: exactly one code path can write a trainer-scoped row, and it always
has a genuine tenant set.

### Coach scoping

A second, narrower layer *inside* the tenant, and not the filter's job. Coach
reach is event-scoped (AC-03-64, AC-03-65) — a relationship traversal, not a
column comparison. `CoachVisibilityService` produces the reachable-event
constraint; every coach-facing read consumes it and every coach voter delegates
to it. A coach assigned to zero events sees zero players.

---

## The RLS disagreement, resolved

The council designed layers 3–5 in application code. The researcher documented
that Doctrine filters fail **open** — no injection into native SQL, no
re-evaluation on identity-map hits, no enablement in console commands or
Messenger workers — and recommended RLS as the actual boundary.

**Decision: RLS is in the MVP, as layer 5, additive to all four council
layers. Nothing the council decided is removed.**

### What each layer actually covers

| Hole | Filter (L2) | Context (L3) | Post-load (L4) | RLS (L5) |
|---|---|---|---|---|
| DQL select in a web request | covers | covers | covers | covers |
| Native SQL / raw DBAL | **no** | no | covers on hydration only | **covers** |
| Console command | **no** | throws | covers on hydration only | **covers** |
| Messenger worker | **no** unless middleware runs | throws | covers on hydration only | **covers** |
| Doctrine identity-map hit | **no** | no | **covers** | **no — no query is issued** |
| Misconfigured database role | covers | covers | covers | **no** |
| Forgotten `kernel.request` listener | **no** | throws | covers | **covers** |

Two corrections this table makes explicit:

- **RLS does not close the identity-map hole.** A `find()` that hits the
  identity map issues no SQL, so no policy is evaluated. Only layer 4, combined
  with the crossing service never producing managed entities, closes it. The
  researcher's framing implied RLS covered all three documented holes; it
  covers two of three.
- **RLS is not a superset of the Doctrine filter.** If the application connects
  as the table owner, or the bootstrap SQL fails, RLS silently stops applying.
  Layer 2 is what still works in that case.

### Why in, not out

- The two paths RLS uniquely covers are not hypothetical here. Epic-03
  segmentation, Epic-03 Top Players and the Epic-07 dashboards are
  aggregate-heavy and will be written as native SQL. The Stripe webhook worker
  runs outside `kernel.request` and processes money.
- The consequence of the failure it prevents is disclosure of minors' medical
  restrictions, injury and financial-aid flags (BR-03-6), coach notes and
  payment history — a breach with notification obligations, not a bug.
- The cost is bounded and container-native: bootstrap SQL creating three
  database roles, one policy per trainer-scoped table, and one session variable.
  No new container, no host dependency.
- Retrofitting it later is not free. The first RLS enablement on a populated
  database is exactly when you discover which code paths were relying on
  cross-tenant reads by accident, under production load.

### How it is wired

- **Three database roles.** An *owner* role that runs migrations and is not
  subject to RLS; an *application* role that is subject to RLS and holds
  `SELECT`/`INSERT`/`UPDATE`/`DELETE`; a *crossing* role with `BYPASSRLS` and
  `SELECT` only. `FORCE ROW LEVEL SECURITY` is deliberately **not** used, so the
  owner can migrate and backfill.
- **Policies use a session variable** holding the active trainer, compared to
  the tenant column. Global tables carry no policy.
- **`TenantContext` is the sole writer of that variable.** It issues the `SET`
  on connection establishment and again on every transition — resolution,
  impersonation start and end, administrative scope entry and exit, and per
  message in the worker. When no tenant is active it sets a sentinel that
  matches no trainer, so an unscoped query returns zero rows *and* layer 3
  throws. Two independent failures pointing the same way.
- **Session-scoped `SET`, not `SET LOCAL`**, because not every unit of work runs
  inside an explicit transaction. This is safe only if a database connection is
  never shared across requests, which imposes two hard constraints, recorded in
  Risks: no persistent PDO connections, and no connection pooler in the stack.
- **Append-only enforcement comes free.** The application role is granted
  `INSERT` and `SELECT` but not `UPDATE` or `DELETE` on the token entry table
  and the audit log. Immutability becomes a database privilege rather than a
  code convention.
- **A startup gate.** A console command asserts, before the container accepts
  traffic and in CI, that the application role is neither the table owner nor
  `BYPASSRLS`, that RLS is enabled on every table expected to carry it, and that
  no policy is missing. This converts "RLS silently stopped applying" — the one
  failure mode RLS cannot self-detect — into a container that refuses to start.
  It also asserts the converse for the resolver's own sources: `AccountTrainerLink`
  and `PublicTenantCode` must carry **no** RLS policy. A migration that enabled
  RLS on either would break every login and every public route, in a way that is
  hard to diagnose because the resolver would simply find nothing.

---

## Entity population

Every entity is classified. There is no unclassified default. `global` means no
tenant column, protected by role and voters only. `trainer-scoped` means the
trainer is denormalized onto it, it carries an RLS policy, and it is covered by
the Doctrine filter and the post-load assertion.

### Global

| Entity | Module | Why global |
|---|---|---|
| `Account` | Identity | One account per person, globally unique by email |
| `AccountProfile` | Identity | Personal details belong to the person, not a tenant |
| `ParentChildLink` | Identity | Family structure is not per trainer |
| `PlayerProfile` | Identity | The person. A child may train with several trainers; trainer-scoped state lives on the membership |
| `EmailVerificationToken`, `PasswordResetToken` | Identity | Issued before any tenant exists |
| `UserDeletionRecord` | Identity | GDPR compliance record about a person (BR-01-24) |
| `Trainer` | Platform | The tenant registry itself. Scoping the discriminator to itself is circular; also read cross-tenant for the switcher and for public content attribution (BR-04-11) |
| `TrainerBrandingSettings` | Platform | Rendered on public, unauthenticated pages (BR-08-6) before any tenant is authenticated. A logo and a hex colour are public by nature |
| `AccountTrainerLink` | Platform | **The tenant resolver's own source.** Minimal pairing of account, trainer, role-in-tenant and status. Cannot be trainer-scoped: the resolver reads it *before* a tenant exists |
| `PublicTenantCode` | Platform | **The tenant resolver's second source.** Minimal mapping of a public code to its owning trainer, with an opaque kind and reference. Cannot be trainer-scoped: the resolver reads it *before* a tenant exists. Carries no expiry, status or use count — those stay on the trainer-scoped `ShareLink`/`Form`, so the mapping cannot drift into an authority |
| `AuditLogEntry` | Platform | Council rule 2. Spans tenants by design; append-only at the database-privilege level |
| `ImpersonationSession` | Platform | Records a Super Admin acting across tenants |
| `PlatformConfiguration` | Platform | Referral ratio, default fee rate, default subscription price (BR-06-4) |
| `FeatureToggle` | Administration | Platform configuration *about* a trainer, owned and written by Super Admin. Global placement removes the need for an administrative tenant scope on the most-used admin screen. Not sensitive |
| `PlatformSubscription` | Billing | The platform's own billing relationship with a trainer (BR-05-13). Stores status and the Stripe identifier only; no money figures. Global so the Epic-07 "active trainers" count needs no crossing read |
| `StripeCustomerLink` | Billing | Cards are shared across all of a parent's trainers at the Stripe Customer level (BR-05-16) |
| `StripeEventReceipt` | Billing | Inserted before the event is parsed, when no trainer is yet known. The unique violation is the duplicate detection |
| HTTP session store, cache pool, lock store | — | DBAL-managed tables, not Doctrine entities. See Deployment |

### Trainer-scoped

| Entity | Module | Note |
|---|---|---|
| `TrainerBillingSettings` | Billing | Connect account, fee rate, token price, subscription price. Sensitive; never cross-tenant. Split out of `Trainer` for exactly this reason |
| `CoachMembership` | Identity | A coach works for exactly one trainer (BR-01-11) |
| `PlayerTrainerMembership` | Identity | Source (`sharelink`, `event_registration`, `coach_invite`, `camp_registration`), ShareLink used, joined-at, status, trainer-set skill level |
| `ShareLink` | Identity | Belongs to a trainer (BR-01-14/15/27) |
| `ShareLinkOpen` | Identity | Click log (BR-03-22) |
| `AvailabilityWindow` | Identity | Coach or player "Best Times". Scoped per trainer — see Decisions |
| `ChildApprovalRequest` | Identity | Covers RSVPs and purchases alike (BR-01-18/19) |
| `Event` | Scheduling | |
| `Rsvp` | Scheduling | |
| `AttendanceRecord` | Scheduling | |
| `AttendanceEdit` | Scheduling | Edit history (BR-02-18) |
| `CoachAssignment` | Scheduling | |
| `CoachAvailabilityOverride` | Scheduling | Kept separate from the assignment so a re-assignment does not erase the audited override (BR-01-26) |
| `EventInvitation` | Scheduling | Private events (BR-02-6) |
| `EventDuplicationRecord` | Scheduling | BR-02-19 |
| `Label`, `PlayerLabel` | Crm | Labels are trainer-specific; same name under two trainers is two labels (BR-03-4) |
| `PlayerFlag` | Crm | The 8 system flags (BR-03-6) |
| `PlayerNote` | Crm | General and per-session (BR-03-10/11) |
| `Playlist` | Content | Publication exception below |
| `ContentItem` | Content | Publication exception below |
| `DrillDetail` | Content | Extends `ContentItem`; publication exception below |
| `PlaylistItem` | Content | Ordering association. Scoped to the playlist's trainer |
| `PlaylistAssignment` | Content | BR-04-13/14/15 |
| `ContentProgress` | Content | BR-04-16/17 |
| `PlaylistAccessGrant` | Content | Per-playlist one-time purchase (A8). Records what was bought so a bundle or subscription model can be added without re-modelling |
| `ContentUsage` | Content | Scoped to the *reusing* trainer. The creator's "used by N trainers" figure is therefore a crossing read, which is correct: it is cross-tenant information |
| `TokenEntry` | Billing | Append-only. See Ledger |
| `TokenBalance` | Billing | Projection and concurrency anchor, per (parent account, trainer) |
| `PaymentRecord` | Billing | Mirror of a Stripe object. Nullable payer |
| `TokenPackage` | Billing | Trainer sets sizes and prices (BR-05-1, Q-05.03) |
| `SubscriptionEntitlement` | Billing | The "subscription token" (BR-05-14). See Ledger |
| `EntitlementCoverage` | Billing | Which entitlement covered which RSVP |
| `ReferralLink` | Growth | Per player, per trainer |
| `Referral` | Growth | |
| `ReferralAssistCount` | Growth | Per player-trainer pair, never global (BR-06-7) |
| `Coupon`, `CouponRedemption` | Growth | Trainer A's coupon never works for Trainer B (BR-06-11) |
| `Form` | Forms | Camp or evaluation; field definitions embedded, not an entity |
| `FormSubmission` | Forms | Public, unauthenticated write. Tenant resolved from the form's code |

### The publication exception

`Playlist`, `ContentItem` and `DrillDetail` are trainer-scoped **with one
declared widening of the read predicate**: a row that has ever been published is
readable by any tenant.

This is required by three business rules that are otherwise mutually
inconsistent: public content is discoverable by all trainers (BR-04-4); adding
it to another trainer's playlist stores a *reference*, not a copy (BR-04-12);
and reverting it to private must not break references other trainers already
hold (BR-04-5). A predicate of `trainer = tenant OR visibility = 'public'`
satisfies the first two and violates the third the moment the creator reverts.

Therefore **visibility governs discovery; publication governs readability.**
Discovery filters on visibility. The read predicate widens only on a
"has ever been published" fact, which is set once and never unset. Deletion —
not reversion — is what breaks references and shows "Unavailable", which is the
epic's own stated behaviour (BR-04-12). Rejected alternatives are in Decisions.

Playlist visibility is stored as **two orthogonal facts**: publication (in the
public library or not) and in-tenant audience (players and coaches, or coaches
only). Their three valid combinations are exactly A9's three states — public,
private, coach-only — so the vocabulary A9 settled is preserved unchanged; only
the storage stops conflating a cross-trainer axis with an in-tenant one.

---

## Cross-cutting services

Each has one responsibility. Each is named here so a later stage does not
invent a second one alongside it.

| Service | Single responsibility | Callers |
|---|---|---|
| `TenantContext` | Hold the active trainer; throw when unresolved; write the database session variable on every transition | Everything, indirectly |
| `TenantResolver` | Determine the tenant for a request, message or command from the six resolution sources, using the global `AccountTrainerLink` and `PublicTenantCode`. The only setter of `TenantContext` outside `AdministrativeScope` | Request listener, Messenger middleware, console base command, code-resolved public routes |
| `PublicTenantCodeRegistry` | Issue and revoke the global `code → trainer` mapping. The only writer of `PublicTenantCode` | `Identity` (ShareLink creation), `Forms` (form publication) |
| `CrossTenantReadService` | The only cross-tenant read. Returns arrays over the read-only `BYPASSRLS` connection; writes an audit entry per call | `Administration` (user lists, dashboards, trainer lists), `Content` (creator usage analytics) |
| `AdministrativeScope` | Enter and exit an audited tenant scope for a target trainer without becoming a user | `Administration` (Event Master, fee edits) |
| `CoachVisibilityService` | Produce the coach's reachable-event and reachable-player constraint from assignments | Every coach-facing read in `Crm`, `Scheduling`, `Content`; every coach voter |
| `MembershipService` | The **only** creator of a `PlayerTrainerMembership`. Requires an explicit association source | `Identity` (ShareLink), `Scheduling` (event registration), `Identity` (coach invite), `Forms` (camp conversion) |
| `TokenLedgerService` | The **only** writer of token entries and balances. Enforces lock order, sign-by-kind and every ledger invariant | `Scheduling`, `Content`, `Growth`, `Administration` |
| `StripeGateway` | The **only** component that calls the Stripe write API. Derives idempotency keys from a payment record persisted before the call | `Billing` services only |
| `StripeReportingReader` | The **only** source of any figure presented as earnings, payout or revenue. Reads live; persists nothing | `Billing` (trainer earnings, AC-05-25) |
| `BrandingProvider` | Resolve the active trainer's brand tokens for the current request and derive the shade ramp | Twig layout |
| `FeatureGate` | Evaluate a trainer's feature toggles (LPPP, Marketing, Camps) | Voters, navigation builder |
| `AuditLogger` | Append-only audit writer | Every admin-authority mutation, every crossing read, every impersonation transition |

---

## Authorization

**Roles are fixed constants** (A2): `ROLE_SUPER_ADMIN`, `ROLE_TRAINER`,
`ROLE_COACH`, `ROLE_PLAYER`. Each account holds exactly one (BR-01-7).

**No role hierarchy is configured.** `ROLE_SUPER_ADMIN` does not inherit the
others. Every Super Admin capability is an explicit clause in a voter. This is
deliberate: with a hierarchy, a voter written as "the subject's trainer is the
active tenant" would silently pass for a Super Admin with no tenant set, which
is precisely the cross-tenant write the council forbade.

- **Object-level decisions are always voters**: `EventVoter`, `RsvpVoter`,
  `PlayerVoter`, `PlaylistVoter`, `ContentItemVoter`, `TokenVoter`,
  `CouponVoter`, `FormVoter`, `FormSubmissionVoter`, `TrainerSettingsVoter`,
  `AuditVoter`.
- **`access_control` is coarse only** — firewall entry points and role gates on
  route prefixes. It never expresses ownership.
- **Feature toggles are a voter attribute**, not a controller `if`. A disabled
  feature is denied at the same place as any other permission (BR-07-1/3).
- **Child approval is a voter attribute** on the RSVP and purchase actions,
  consulting the approval state and the per-child "may spend tokens without
  approval" setting (BR-01-18/19).
- **A parent acting for a child** is authorized through `ParentChildLink`, in a
  voter, never by comparing identifiers in a controller.
- **Coach voters delegate** to `CoachVisibilityService`. A voter that restates
  event-scoped reach is a defect.

---

## The token and payment ledger

Append-only entry log as truth, balance as a locked projection, kept separate
from the payment mirror. Completing the council's verdict:

### Entry kinds and signs

| Kind | Sign | Notes |
|---|---|---|
| `purchase` | positive | Funded by a payment record |
| `gift` | positive | Trainer grants tokens |
| `referral_reward` | positive | Epic-06. References the triggering referral |
| `refund` | positive | Compensating entry referencing the original spend |
| `spend` | negative | Beneficiary player required (A7) |
| `adjustment` | signed | **Added beyond the council.** Super Admin correction for out-of-band Stripe activity (BR-05-12). Requires a reason and is audit-logged |

### Invariants, as enforceable rules

- **I1** For every (parent account, trainer): the balance projection equals the
  sum of that pair's entries. Self-policing; verified by a console command and
  by a test.
- **I2** A balance is never negative. A database check constraint is the
  backstop; a negative `adjustment` that would breach it is rejected loudly.
- **I3** Every `spend` records the beneficiary player. Every `refund` inherits
  the beneficiary of the spend it references.
- **I4** The sum of refunds referencing one spend never exceeds that spend.
  BR-05-5 permits partial token refunds, so this must be enforced rather than
  assumed.
- **I5** The sign matches the kind, per the table above.
- **I6** At most one entry exists per (payment record, purpose). Outbound
  idempotency at the ledger level.
- **I7** No entry is ever updated or deleted. Enforced by database privilege,
  not by convention: the application role holds `INSERT` and `SELECT` only.

### Lock ordering

A paid RSVP takes two locks — the token balance row and the event capacity row.
Two concurrent RSVPs can deadlock if the order varies.

**Fixed global order: the token balance row first, then the event row.** Every
path that takes both obeys it — paid RSVP, RSVP cancellation refund, content
purchase that also touches capacity.

Chosen this way because the event row is the hot row (many parents contending
for the last spots of one event) while the balance row is contended only within
one family. Acquiring the balance first means the hot row is held for the
minimum span — from acquisition to commit — instead of also spanning the
balance validation. An **unlocked capacity pre-check** gives fail-fast UX for
the common "event full" rejection; it is explicitly non-authoritative and the
locked check inside the transaction is the one that decides (AC-02-67).

**Trainer-initiated cancellation never holds both.** It locks the event, marks
it Cancelled and commits — closing the door on new RSVPs — and then refunds each
player in its own transaction taking only that player's balance lock. A single
transaction holding the event lock across N Stripe refund calls is not viable.
This is also why the fan-out is asynchronous.

### Subscriptions are entitlements, not ledger entries

`SubscriptionEntitlement` carries its own (parent account, trainer) subject, an
activation date chosen by the purchaser, a 30-day window and a state
(pending activation, active, expired). Spending against it decrements nothing
and writes no token entry; the RSVP records which entitlement covered it via
`EntitlementCoverage`. The user-facing name stays "subscription token".

**The "1 advance booking per day, unlimited same-day" rule** (BR-05-14,
AC-02-63) evaluates as: let `D` be the calendar date of the event's start in the
trainer's timezone and `T` today's date in that timezone. If `D == T` the
booking is same-day and unconstrained, subject only to capacity and eligibility.
If `D > T` it is an advance booking, permitted only when the player holds no
other active entitlement-covered RSVP for an event on date `D`. The limit counts
**entitlement-covered RSVPs only** — a subscriber may still pay tokens or money
for an additional advance event. Both readings of the source text are recorded
in Open architecture risks.

### Payment records

- **The fee rate in force is recorded on every payment at creation.** AC-05-27
  changes the rate for new transactions only; a rate read live would
  retroactively rewrite every historical figure.
- **The payer is nullable, with an always-present contact snapshot** taken from
  the form submission. A camp registrant who never converts has a payment and no
  account (A3). On conversion (A5) the account link is added — an additive link,
  not a re-modelling. Camp payments produce no token entry.
- **Refunds are new records, never mutations.** The charge record's *status* may
  still move because it mirrors Stripe's lifecycle, but "Refunded" as displayed
  in transaction history (AC-05-23) is derived from refund records. This is what
  makes BR-05-12's out-of-band Stripe Dashboard refunds reconcilable.
- **Refund direction, encoded exactly as the spec states**: cancelling **at
  least 24 hours** before the event start is a full refund; **under 24 hours**,
  none; a trainer-initiated cancellation always refunds in full regardless of
  timing (BR-05-10, AC-05-14/15, BR-02-11/12). A coded inversion would refund
  exactly the wrong population.
- **Idempotency is a database constraint.** Inbound: the Stripe event id is
  inserted into the uniquely-constrained `StripeEventReceipt` table first, in
  its own transaction; the unique violation *is* the duplicate detection.
  Handlers reconcile to the state Stripe reports for the payment intent rather
  than applying a delta, so out-of-order delivery cannot regress state. Outbound:
  the idempotency key is derived deterministically from a payment record
  persisted **before** the Stripe call, so a timeout retry reuses the key.

### The earnings boundary

Epic-07 AC-07-7 states four times that no revenue, payout or transaction data is
duplicated on the platform; AC-05-25 shows trainers their lifetime earnings.
Resolution: **the local payment record exists for linkage and correctness; any
figure presented as earnings, revenue or payout is read from Stripe.**

Structurally: `StripeReportingReader` is the only source of such figures, it
persists nothing, and no repository exposes a method that sums payment amounts
into a money figure for display. When Stripe is unavailable the earnings panel
shows an error — never a locally-summed fallback, which would silently violate
AC-07-7. The Epic-07 dashboard carries no money figures at all, only a link out.

---

## Synchronous and asynchronous work

Default transport is `sync`. One `doctrine://` transport, using the PostgreSQL
already in the stack, carries three things:

| Async work | Why |
|---|---|
| Stripe webhook processing | Stripe requires a 2xx before complex logic and retries for up to 3 days; slow synchronous handling is a correctness problem, not a latency one |
| Refund fan-out on trainer-initiated event cancellation | N Stripe refund calls in one request will time out (BR-02-12). One message per RSVP, idempotent per (RSVP, purpose) |
| Notification fan-out on event cancellation and price change | Same shape. A single transactional email stays synchronous |

Everything else is synchronous: RSVP, token spend, content purchase, form
submission, single emails, event duplication.

**Time-based state is derived at read time; the scheduler only materialises side
effects.** A 48-hour-old approval request is expired because its timestamp says
so, not because a job ran (BR-01-18). An impersonation session past one hour is
over. This removes the entire "the cron did not run" class of defects. Symfony
Scheduler, running inside the worker container, then handles only what genuinely
requires an actor: the auto-denial notification (US-01.05), the 7-day
pending-payment RSVP auto-cancellation (BR-05-9), and referral attribution
expiry housekeeping. No host cron.

---

## White-label branding as a runtime value

Branding is a value read per request, never a build input.

- `TrainerBrandingSettings` holds one logo reference and one primary colour
  (AC-01-60/61/63). MVP scope is exactly that; no fonts, no layout.
- `BrandingProvider` resolves it for the active trainer — or, on public
  unauthenticated pages, for the trainer that owns the form or portal being
  viewed — and derives the shade ramp server-side using the transformations
  `DESIGN_TOKENS.md` specifies (`lightenColor`, `darkenColor`, `hexToRgb`).
- The layout emits a small nonce-carrying inline `<style>` block setting
  `--brand-primary`, `--brand-primary-soft`, `--brand-primary-deep` and
  `--brand-primary-rgb` on `:root`. Every component consumes the custom
  properties. Saving branding applies immediately for everyone in that trainer's
  organization (AC-01-62) because nothing is cached and nothing is compiled.
- AssetMapper compiles the **static** token layer once — typography, spacing,
  radius, shadow scales from `DESIGN_TOKENS.md`. The brand layer never enters
  the pipeline, so no Node toolchain and no per-tenant build exists.
- Logos are stored on a Compose volume and rendered through `<img>`, never
  inlined, so an uploaded SVG cannot execute in the page's origin.

---

## Deployment shape

Docker Compose only. Five services, no host dependencies:

| Service | Purpose |
|---|---|
| `php` | php-fpm, PHP 8.3 or 8.4 |
| `web` | HTTP front end serving `public/` and proxying to `php` |
| `db` | PostgreSQL. Bootstrap SQL creates the owner, application and crossing roles |
| `worker` | `messenger:consume` plus Symfony Scheduler |
| `mail` | Mail catcher for development |

**All shared runtime state lives in the existing PostgreSQL** — HTTP sessions
via the PDO session handler, the cache pool backing the login rate limiter
(BR-01-6), and the lock store. No Redis, no filesystem affinity, no sticky
sessions, and therefore no sixth container.

The startup gate described under RLS runs in the `php` and `worker` entrypoints
before either accepts work.

---

## Stack

Only what this platform adds, pins or changes.

| Choice | Version | Over the alternative, because |
|---|---|---|
| Symfony | 7.4 LTS | Bugfixes to Nov 2028, security to Nov 2029, PHP >= 8.2 floor. 8.1 is not LTS, requires PHP >= 8.4 and ends Jan 2027 |
| PHP image | 8.3 or 8.4 | 8.2 security support ends 31 Dec 2026; both satisfy Symfony 7.4's floor |
| PostgreSQL | 16 or 17, one pinned tag | Row-Level Security and LISTEN/NOTIFY, both long-established. Same instance backs Messenger, sessions, cache and locks |
| Stripe | `stripe/stripe-php` v21.x, pinned in `composer.lock` | Official, ~164M installs. The one established Symfony bundle is abandoned and its own README recommends implementing directly |
| Asset pipeline | AssetMapper | Current `--webapp` default; no Node toolchain, which matters under a Compose-only constraint |
| Money | Integer minor units | Matches Stripe's wire format exactly. `brick/money` adds a mapping layer at a boundary that already speaks integers, at USD-only scope |
| Async | `doctrine://` transport | Uses the PostgreSQL already present; no broker container |
| Scheduling | Symfony Scheduler in the worker | No host cron, which a Compose-only constraint forbids |

---

## Decisions

| Decision | Chosen | Rejected | Because |
|---|---|---|---|
| Tenancy enforcement | Five layers: denormalized tenant key, Doctrine filter, mandatory context, post-load assertion, RLS | Filter alone; explicit predicates alone; schema-per-tenant; database-per-tenant | Filters fail open and are not closeable by discipline. Predicates give an unbounded review surface and silent failure. Physical separation forces cross-schema joins on the highest-traffic query, N-times migrations, runtime DDL on trainer creation, and connection multiplication against a single Compose PostgreSQL — and a shared schema is needed anyway for accounts, Stripe Customers and camp submissions |
| **PostgreSQL RLS** | **In for MVP**, as an additive fifth layer | Out for MVP, relying on application layers alone | It is the only layer covering native SQL, console commands and Messenger workers, all three of which this platform certainly has. The failure it prevents discloses minors' medical and financial-aid flags. Cost is bootstrap SQL plus one policy per table — container-native. Enabling it later, on populated data, is where you discover which paths accidentally relied on cross-tenant reads |
| Doctrine filter retained alongside RLS | Both | Dropping the filter as redundant | Neither is a superset. The filter survives a misconfigured database role; RLS survives a forgotten `kernel.request` listener |
| Identity-map coverage | Post-load assertion plus projection-only crossing | Relying on RLS to cover it | An identity-map hit issues no SQL, so no policy is evaluated. RLS covers two of the researcher's three documented holes, not three |
| Crossing implementation | Separate read-only `BYPASSRLS` connection returning arrays | Disabling the filter inline; a second EntityManager | A connection with no EntityManager is structurally incapable of producing a managed entity, so "projections, never entities" stops being a discipline |
| Cross-tenant writes | Impersonation **or** an audited administrative tenant scope | Impersonation only; unscoped Super Admin writes | The Event Master tool and per-trainer fee edits are legitimate admin writes that impersonating a user models badly. The scope sets the same genuine tenant context, preserving "exactly one code path writes trainer-scoped data" |
| **Tenant resolution source** | A global, minimal `AccountTrainerLink` | Reading the trainer-scoped membership; routing the trainer switcher through the crossing service | The resolver runs *before* a tenant exists, so it cannot read a trainer-scoped table — and the player's trainer switcher (AC-01-15) would otherwise show only the tenant already active. Routing every request's resolution through the crossing service would make it the hottest path in the application and drown the audit log |
| `Trainer` registry placement | Global, split from trainer-scoped billing settings | One global trainer entity holding Connect ids and fee rates; one fully scoped trainer entity | Display name and branding must be readable cross-tenant (switcher, public content attribution) and pre-authentication (public form pages). Connect account, fee rate and prices must not be. One entity cannot be both |
| Branding placement | Global | Trainer-scoped | Public, unauthenticated camp forms (BR-08-6) render trainer branding with no tenant authenticated. A logo and a hex colour carry no confidential content |
| Feature toggles placement | Global | Trainer-scoped | Council rule 2 classifies platform configuration as global. It is Super-Admin-owned, non-sensitive, and global placement removes an administrative tenant scope from the most-used admin screen |
| Player availability | Trainer-scoped | Global, one set of times per player | Every other player-facing surface — calendar, tokens, content, RSVPs — is separated per trainer (A6, BR-01-10). One global availability set would let one trainer infer a player's commitments to another. The cost is entering times once per trainer |
| Content readability | Trainer-scoped with a "has ever been published" widening | `trainer = tenant OR visibility = 'public'`; a subquery over the reusing trainer's playlists; copying content on reuse; routing every playlist render through the crossing service | The simple visibility predicate breaks BR-04-5 the moment a creator reverts to private. A subquery in a row policy is expensive and stops being a column comparison. Copying breaks BR-04-12's reference semantics. Crossing on a hot player-facing read is unacceptable. Publication is set once and never unset, so the widened predicate stays a column comparison |
| Playlist visibility storage | Two orthogonal facts — publication and in-tenant audience | One three-valued attribute | The three states A9 settled conflate a cross-trainer axis with an in-tenant one. Their three valid combinations are exactly A9's three states, so the settled vocabulary is unchanged |
| Ledger shape | Append-only entries, balance as a locked projection, payments kept separate | Stored balance as truth; pure derivation; one table with a unit discriminator; an Order/basket header | A drifted stored balance has no reconstruction path. Pure derivation does not prevent the last-token race. A unit discriminator makes the balance invariant conditional on a column, so one mis-typed row corrupts a balance silently. The MVP forbids split tender and multi-line purchases, but "what was this for" stays uniform across both record types so a header can be added later |
| Entry immutability | Database privilege — the application role holds `INSERT` and `SELECT` only on entries and the audit log | Code convention plus no setters | A convention survives exactly until someone adds a setter. The role separation RLS already requires makes this free |
| Subscriptions | Entitlement grants with their own window and state | A token entry with a special kind; a signed integer balance | Activation date, 30-day expiry and the 1-per-day advance rule are none of them a quantity, and its expiry contradicts BR-05-4's no-expiry rule for tokens |
| Referral rewards | A token entry of kind `referral_reward` | A separate `ReferralReward` entity | Epic-06's proposed reward entity duplicates the grant the ledger already records, creating a second record that can diverge from the balance |
| `adjustment` entry kind | Added, Super-Admin-only, reason required, audit-logged | The council's five kinds alone | BR-05-12 permits out-of-band Stripe Dashboard refunds with no platform-side counterpart. Without a correction kind the only reconciliation is a `gift`, which misreports what happened |
| Lock ordering | Token balance first, then the event row, on every path | Event first; no fixed order | A fixed order is what prevents the deadlock. Balance-first holds the hot, many-writer event row for the minimum span. Fail-fast comes from an unlocked, explicitly non-authoritative capacity pre-check |
| Trainer cancellation refunds | Commit the cancellation, then refund each player in its own transaction, asynchronously | One transaction covering the cancellation and all refunds | Holding the event lock across N Stripe calls is not viable, and a synchronous fan-out times out |
| Async scope | `doctrine://` for Stripe webhooks **plus** refund and notification fan-out | Webhooks only; async by default | The researcher's webhook case is unchanged. Fan-out has the same timeout shape and needs no new infrastructure — the worker container already exists for webhooks |
| Time-based state | Derived at read time; the scheduler only sends notifications and materialises side effects | A scheduled job that flips state | A job that did not run must never change what the system believes is true |
| Scheduling mechanism | Symfony Scheduler inside the worker container | Host cron; a dedicated cron container | Host cron violates the no-host-dependency constraint; a separate container adds nothing the worker cannot do |
| Shared runtime state | PostgreSQL — PDO sessions, PDO cache pool, PostgreSQL lock store | Filesystem sessions and cache; a Redis container | Filesystem state forces single-replica affinity. Redis is a sixth container for something the existing database does adequately at this scale |
| Role hierarchy | None configured | `ROLE_SUPER_ADMIN` inheriting the others | With a hierarchy, an ownership voter silently passes for a Super Admin holding no tenant — exactly the cross-tenant write the council forbade |
| ShareLink tenant resolution | A route-scoped code source (5) backed by a minimal global `code → trainer` mapping, resolved in `TenantResolver` | A scope opened by `MembershipService`; writing `AccountTrainerLink` first; moving membership creation to `Platform`; making `ShareLink` or the membership tables global; resolving via `CrossTenantReadService` | Learning a ShareLink's trainer requires reading a trainer-scoped row, so every alternative is circular without this mapping — it is the precondition, not an option. Once the tenant is genuinely resolved at `kernel.request`, a service-opened scope adds surface and buys nothing, and it would be a second setter of `TenantContext` outside the resolver. Source 5 already performed this lookup for `Form` without naming the mechanism |
| Code-lookup auditing | Audit the membership created and the stale-link denial; rate-limit and count unknown codes without auditing them | Auditing every lookup, e.g. by routing it through `CrossTenantReadService` | An unconditional audit write on an anonymous, scannable route makes the audit log the amplification target — the same objection that kept per-request resolution out of the crossing service |
| Membership creation | One service, explicit source required | Each entry point creating its own | AC-03-68 requires every association to record its source, now four of them including `camp_registration` (A3). One creator makes that structural |
| Branding delivery | Nonce-carrying inline `<style>` block on the layout | A `/branding.css` route; per-tenant compiled CSS | A stylesheet route needs a per-tenant cache key and a purge on save, and can serve stale CSS against AC-01-62's "applies immediately". Per-tenant compilation makes branding a deploy |
| Earnings figures | Read live from Stripe, never summed locally, no fallback | Summing local payment records; caching earnings in the database | AC-07-7 states four times that no revenue, payout or transaction data is duplicated. A local fallback would violate it silently, precisely when Stripe is unreachable and nobody is checking |
| Application shape | Modular monolith, module per epic, layered inside each | Flat `src/Service`; separate services per epic | Around 45 entities across 8 epics makes a flat structure unnavigable; separate services buy distribution problems this product does not have |

---

## Risks

- **RLS silently stops applying** if the application connects as the table owner
  or the bootstrap SQL fails. This is the one failure RLS cannot self-detect.
  *Cheapest early check*: the startup gate refuses to start the container, plus
  a functional test that queries a foreign-tenant row and asserts zero rows.
- **Fail-closed silence.** With a tenant unresolved, RLS returns zero rows
  rather than raising. A batch job would process nothing, forever, quietly.
  *Mitigated by* layer 3 throwing before the query is issued — which is why the
  mandatory context is not made redundant by RLS.
- **The session-variable model breaks under connection sharing.** Persistent PDO
  connections or any pooler in transaction, statement or session mode would leak
  one tenant's variable into another request. *Constraint*: no persistent
  connections, no pooler in the stack. The startup gate asserts the former.
- **The identity-map hole reopens** if the post-load assertion is ever removed
  as "redundant now that we have RLS". *Cheapest early check*: a test that
  hydrates a foreign-tenant entity through a permitted path and asserts the
  assertion throws, with a comment pointing at this document.
- **Lock-order violation** reintroduces the RSVP deadlock. *Cheapest early
  check*: a concurrency test driving a paid RSVP and a cancellation refund
  against the same parent and event simultaneously.
- **The publication predicate is the one widening of tenant isolation.** A bug
  in it leaks a trainer's private drills to every other trainer. *Cheapest early
  check*: a test asserting an unpublished item is invisible cross-tenant and a
  reverted item stays readable.
- **Doctrine ORM version is unverified.** Filter mechanics have been stable
  across 2.x and 3.x, but confirm what a Symfony 7.4 skeleton actually resolves
  before treating exact behaviour as settled.
- **Stripe SDK API-version drift.** Pin `stripe/stripe-php` in `composer.lock`;
  the SDK ships frequent API version bumps.
- **Integer-cents is scoped to USD-only with a flat fee.** Multi-currency, or
  partial-refund allocation beyond the token case, should revisit `brick/money`.
  An explicit rounding rule for the 5% fee must be fixed at the database-designer
  stage, since the trainer absorbs it and it is included in the listed price.
- **Segmentation and dashboard queries have no stated performance budget** while
  being the aggregate-heaviest reads in the product, and now also carry RLS
  predicates. *Cheapest early check*: seed a realistic tenant and measure before
  the CRM screens are considered done.
- **SVG logo upload.** Mitigated by rendering through `<img>` and never inlining,
  plus a strict CSP; revisit if inline rendering is ever wanted.
- **Single-replica assumptions elsewhere.** Sessions, cache and locks are on
  PostgreSQL precisely so replicas are possible, but uploaded logos live on a
  Compose volume and would need shared storage before the `php` service is
  scaled.

---

## Open architecture risks — assumptions I was forced into

Stated explicitly rather than buried. Each one is a place where the specs do not
settle something the architecture needed.

1. **No epic states a timezone model.** Both the 24-hour refund boundary
   (BR-05-10) and the "1 advance booking per day" rule (BR-05-14, AC-02-63)
   require one. *Assumed*: event times are stored as absolute instants and
   calendar-day questions are evaluated in the trainer's configured timezone.
   This adds a timezone to the trainer's settings that no epic asks for.
2. **"1 event per day in advance" is ambiguous.** *Assumed*: at most one
   entitlement-covered booking held for any future calendar day, unlimited for
   the current day. The alternative reading — at most one advance booking
   *made* per calendar day — is coherent too and would change the check
   entirely.
3. **Whether the advance limit constrains paid bookings too.** *Assumed*: it
   counts entitlement-covered RSVPs only, so a subscriber may still pay for an
   extra advance event. Blocking a paying customer seemed commercially
   implausible, but the source does not say.
4. **The 24-hour refund boundary is evaluated against the event's current start
   time**, not a snapshot taken at RSVP. A trainer editing an event's start
   therefore changes refund eligibility for existing RSVPs. No epic addresses
   this.
5. **Epic-07's dashboard metrics are stated both as live queries and as an
   hourly batch.** *Assumed*: live queries, consistent with "calculated, not
   stored", and consistent with deriving time-based state at read time. If they
   prove too slow this becomes the first place a materialised projection is
   needed.
6. **A coach viewing a player with no shared session history** has no stated
   expected behaviour (Epic-03 Testing Considerations). *Assumed*: not reachable
   at all — `CoachVisibilityService` yields no such player, so the route returns
   access denied rather than an empty profile.
7. **Camp form submissions and ShareLink acceptance are unauthenticated writes
   into trainer-scoped tables** — `FormSubmission`, `ShareLinkOpen`,
   `PlayerTrainerMembership`, `CoachMembership` — some carrying PII of minors,
   with the tenant resolved from a public code. Required by BR-08-6, BR-01-14/15
   and BR-03-22. Both need their own rate limiting and abuse review, neither of
   which any epic specifies.
8. **`FeatureToggle` and `PlatformSubscription` are classified global**, which
   means a trainer could read another trainer's feature set or subscription
   status if a voter were ever missing. Judged non-sensitive; if the client
   disagrees, both move to trainer-scoped and both admin screens gain an
   administrative tenant scope.
9. **`ContentUsage` makes the creator's "used by N trainers" figure a crossing
   read.** That is correct — it *is* cross-tenant information — but it means an
   Epic-04 analytics panel writes an audit entry on every view.
10. **Epic-05 AC-05-24's Family Overview is dropped** by A6. If it is ever
    reinstated, `AccountTrainerLink` is the only structure that could serve it,
    and it would need an explicit, audited carve-out rather than a quiet
    widening of any existing read.

---

*Written 2026-08-09. Inputs: Section A owner decisions, the tenancy council
verdict, the ledger council verdict, and the researcher's stack findings. The
Decisions table exists so a later session does not relitigate any of them.*
