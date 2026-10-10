# API Design: PracticePerfect platform

Route surface for a server-rendered Twig application with progressive
enhancement — **not a public REST API**. Symfony 7.4, one modular monolith,
nine modules. Built under `Task/app/`, which today is a bare
`composer create-project symfony/skeleton` checkout (`FrameworkBundle` only —
no Doctrine, no Security, no Twig, no API Platform installed yet). Nothing
here retrofits existing routes; this is the first route design for the
project.

**Inputs — read in full, not reopened.** `specs/architect-architecture.md`
(module map, five-layer tenancy, authorization, the ledger, async scope,
white-label branding, deployment shape, Decisions, Risks, Open architecture
risks). Section A of `specs/requirements-analyst-open-questions.md` (owner
decisions A1–A12 and the fee call). The eight epic specs
`specs/requirements-analyst-epic-0{1..8}-*-spec.md`, read for User scenarios,
Acceptance criteria, Business rules and Open questions per epic — 419
acceptance criteria total (78+70+70+43+38+33+41+46, matching
`architect-architecture.md` line 4).

**What this document does not do.** No schema — entities, columns, keys and
indexes are `database-designer`'s stage. No product decisions the epics leave
open — those are listed under `## Open questions` rather than settled here.
No client content is reproduced beyond the short acceptance-criterion
paraphrases the existing specs already use as their citation style.

---

## Conventions

Established once here and applied without restating on every row below.

### Path namespaces

Five role-scoped areas, chosen because the epics themselves describe five
distinct consoles/portals (Trainer Dashboard, Coach's "My Activities", the
Player/Parent portal, the Super Admin console, and the public,
un-authenticated surface) rather than one shared screen with a permission
toggle:

| Prefix | Coarse gate | Who |
|---|---|---|
| `/trainer/...` | `ROLE_TRAINER` | Trainer console: events, CRM, content, coaches, marketing, forms, billing settings |
| `/coach/...` | `ROLE_COACH` | Coach console: My Activities, assigned players, My Times |
| `/portal/...` | `ROLE_PLAYER` | Player/Parent portal. One role covers both — a parent account "is itself treated as a player account" (`requirements-analyst-epic-01-user-management-spec.md`, Data requirements note, line 302) |
| `/super-admin/...` | `ROLE_SUPER_ADMIN` | Administration module's console |
| `/account/...`, `/context/...` | `IS_AUTHENTICATED_FULLY` | Cross-role: own profile, trainer-context switch |
| no prefix | `PUBLIC_ACCESS` | `/login`, `/password/*`, `/join/{code}`, `/invite/{code}`, `/forms/{code}`, `/webhooks/stripe` |

**There is no generic, code-less public registration route.** Every account
creation path is either Super-Admin-initiated (trainer creation, BR-01-13 —
"no self-registration"), code-scoped via a ShareLink (`/join/{code}`,
`/invite/{code}`), or code-scoped via a camp/evaluation form (`/forms/{code}`).
A bare `/register` would contradict BR-01-13; it does not appear below.

### Route naming

`{module}_{resource}_{action}`, snake_case, module prefix is the lower-cased
module name from the architecture's module map (`platform`, `identity`,
`scheduling`, `crm`, `content`, `billing`, `growth`, `administration`,
`forms`). The prefix names which module's `Controller/` directory the route
lives in — see "Module boundary calls" below for the handful of cases where
that placement was not obvious from the architecture document and had to be
decided here.

### Gates vs. voters

Matches `architect-architecture.md`, "Authorization" (line 413): `access_control`
is coarse-only (the prefixes above, plus `PUBLIC_ACCESS`/`IS_AUTHENTICATED_FULLY`
literals); every object-level decision is a voter. Table columns below write
this as `ATTRIBUTE → Subject`. `Subject: —` means the decision does not need
an object — either it is a pure class-level capability check (e.g. "can this
role create an Event at all") already covered by the coarse gate, or the
query itself is scoped to the current actor (a player viewing their **own**
progress needs no voter; a trainer viewing **a** player's progress does).

**No role hierarchy is configured** (architect-architecture.md line 418), so
every voter that gates a trainer-owned action carries its own explicit
`ROLE_SUPER_ADMIN` clause rather than inheriting one. Administration's routes
that reach the same underlying resource as a trainer-facing route (Event
Master editing an event, feature-toggle-gated content) **reuse the same
voter attribute** — `EVENT_EDIT`, not a separate `EVENT_EDIT_AS_ADMIN` — with
the Super Admin clause inside that one voter, exercised only once an
`AdministrativeScope` is open (see "Impersonation and administrative tenant
scope"). Table rows mark this `(+ SUPER_ADMIN via AdministrativeScope)` where
it applies, instead of duplicating a row per role.

Voter list used below, reusing the eleven the architecture already named
(`EventVoter`, `RsvpVoter`, `PlayerVoter`, `PlaylistVoter`, `ContentItemVoter`,
`TokenVoter`, `CouponVoter`, `FormVoter`, `FormSubmissionVoter`,
`TrainerSettingsVoter`, `AuditVoter`) and adding only what those eleven do not
cover: `AccountVoter`, `ChildProfileVoter`, `ShareLinkVoter`,
`AvailabilityVoter`, `ChildApprovalVoter`, `CoachMembershipVoter`,
`LabelVoter`, `PaymentMethodVoter`, `ImpersonationVoter`,
`PlatformConfigurationVoter`. Each new one is justified inline the first time
it appears. This is a larger set than the architecture's illustrative list of
eleven, which reads as "at least these," not "exactly these" — it names the
voters load-bearing enough to discuss (coach delegation, feature toggles) and
leaves the rest to this stage, which is exactly this document's job.

### Request contract

Matches `architect-architecture.md`, "Layering" table, "Input validation"
row: **Symfony Forms for every browser POST**, Request DTOs with
`#[MapRequestPayload]` only for the small set of genuinely-JSON endpoints
(next section). Form classes live at `App\{Module}\Form\{Name}Type`; JSON
request DTOs at `App\{Module}\Dto\Request\{Name}Request`. No route accepts a
client-supplied tenant id, role, price, fee rate, workflow state, or
ownership field — those are set server-side from `TenantContext` and the
authenticated actor, never bound from the request.

### Response contract

Four shapes, used consistently:

1. **Twig view** — `GET` routes rendering a page or a Turbo Frame fragment.
2. **Redirect-after-post** — every successful mutation. 303 to a `GET` route,
   flash message set. See "Error contract" below for the un-successful case.
3. **JSON** — only the routes named in "JSON vs. HTML," below.
4. **File download** — CSV exports (`Content-Type: text/csv`, streamed, not
   paginated — see Pagination).

### Module boundary calls

Four placements the architecture document does not spell out route-by-route,
decided here and listed again in `## Decisions`:

- **The Users tool** (list/search/view/edit-any-account/impersonate/
  deactivate/reactivate/delete-GDPR/create-trainer) is one screen described
  from two epics: Epic-01 states the mechanics (US-01.01, .07, .12, .13),
  Epic-07 states the console it lives in (AC-07-8..17, "In Scope (MVP)" §
  "Tools", `Epic-07_Super_Admin_System_Management_SPEC.md` line 65). Routed
  under **Administration** (`Epic-07`'s "Super Admin console" ownership),
  calling **Identity**'s account services underneath.
- **Per-trainer fee edits** (US-05.10, an Epic-05 story) route under
  **Administration**, not Billing — the architecture's own cross-cutting
  services table states it explicitly: `AdministrativeScope`'s callers are
  "Administration (Event Master, fee edits)" (line 401), and the Decisions
  table repeats it (line 654).
- Every **other** epic's own Super-Admin-facing analytics/config screen
  (CRM Master — US-03.12, LPPP Analytics — US-04.11, referral-ratio config —
  US-06.08) stays in **that epic's own module**, since the architecture names
  only the Users tool, Event Master, the dashboard and the audit log viewer
  under Administration's "owns" column (line 58) and these three are not
  among them.
- **Event Master Tool's list view** (US-07.06) uses `CrossTenantReadService`
  (its callers explicitly include "dashboards, trainer lists" — line 400);
  **editing or cancelling one event** (US-07.07) uses `AdministrativeScope`
  instead, because that is a genuine scoped write, not a read. Same split for
  the Users tool: list/search via `CrossTenantReadService`, per-account edits
  via calls into Identity's own services once `AdministrativeScope` (or an
  impersonation session) is active.

### AdministrativeScope is per-request, not per-session

Unlike impersonation, which is a whole browsing session with a persistent
banner (AC-01-34) and an explicit exit action, `AdministrativeScope` is
opened after voter authorization succeeds and closed at the end of the same
request/response cycle (`kernel.terminate`, or an explicit `finally`). Viewing
an event in Event Master and then editing it are two separate requests, each
independently opening and closing its own scope. This is not stated
explicitly in the architecture document; it is the most direct reading of
"adopting a trainer's tenant **without becoming a user**" (line 194) applied
to an HTTP request/response cycle, and is recorded in `## Decisions`.

### Rate limits, referenced once

| Limiter | Scope | Source |
|---|---|---|
| Login attempts | Per (IP, email) composite | BR-01-6 |
| Payment attempts | Per actor | AC-05-36 |
| Public form submission | Per IP, plus a honeypot field | `requirements-analyst-epic-08-forms-registration-spec.md`, Edge cases, "A bot or automated script submits the public form" |
| Password-reset request | Per (IP, email) composite | api-designer default — enumeration hardening, not epic-sourced |
| Coupon-code apply attempts | Per (actor, trainer) composite | api-designer default — brute-force hardening, not epic-sourced |

All limiters back onto the PostgreSQL-backed cache pool the architecture
already names for this exact purpose ("the cache pool backing the login rate
limiter (BR-01-6)" — Deployment shape, line 619) rather than a new store.

---

## JSON vs. HTML

Everything is HTML unless listed here. Three categories, each justified —
the task brief names these three as the candidates to examine, not as a
foregone conclusion, and one of them (the public form submission) is
deliberately **not** classified as JSON below, with reasoning.

### 1. The Stripe webhook receiver — JSON in, plain text out

`POST /webhooks/stripe`. Stripe sends JSON; the platform never renders
anything for it. See "Stripe webhook contract" for the full design. Not
CSRF-protected — there is no session, no cookie, no ambient browser
authority to forge, and the endpoint carries its own signature check.

### 2. Progressive-enhancement AJAX endpoints — genuinely JSON

Chosen because each one feeds a small, actively-manipulated widget rather
than re-rendering a page region — a Turbo Frame re-render is the better fit
for anything that is itself a list of rows (CRM search-as-you-type,
segmentation filtering, coupon-list refresh), so those stay HTML fragments,
not JSON:

| Route | Why JSON, not a Turbo Frame |
|---|---|
| `scheduling_trainer_event_availability_check` | Feeds a live-updating number ("15 of 20 eligible players available") next to a date/time picker while the trainer is still composing the Event form — AC-02-12, AC-01-45. A full-page or frame re-render would blow away the in-progress, unsaved form state around it. |
| `content_trainer_youtube_metadata` | Server-side proxy to avoid exposing a YouTube API key to the browser and avoid CORS; feeds one JS-populated field (duration, title) while the trainer is mid-edit on a playlist/drill form — AC-04-2. |
| `content_trainer_playlist_items_reorder` | The client already holds the correct DOM order after a drag-and-drop, for either a Learn playlist's videos or a Practice playlist's drills — one `PlaylistItem` ordering association covers both, so there is no second, separate drill-reorder route; the server only needs to persist an ordered id list and has nothing worth re-rendering — AC-04-2, AC-04-12. |
| `growth_portal_coupon_validate` | Recomputes a price inline at checkout ("$20 → $16") without reloading the whole checkout page or losing the payment-method selection already made — AC-06-20..22. |
| `content_portal_item_complete` | Fires from the YouTube IFrame API's `onStateChange` event the instant playback starts — a widget event, not a page transition, so there is nothing to render — AC-04-25. |

Each is authorized like any other mutation (voter, and CSRF via a header —
see "CSRF placement" — for the three that mutate). Response shape:

```json
{ "eligible": 20, "available": 15 }
```

closed, small, and named per-route below rather than through one generic
envelope, because these are four unrelated widgets, not one API surface.

### 3. The public form submission — deliberately kept HTML, not JSON

The brief lists this as a JSON candidate; this document disagrees, and
records why rather than silently picking one. A camp/evaluation registrant
is frequently a parent on a phone, sometimes on poor connectivity, filling
in the form exactly once (BR-08-6: no login; AC-08-40: mobile-friendly). A
standard HTML form POST with redirect-after-post — to a confirmation page
when free, to Stripe Checkout (a 303) when paid — works with zero
JavaScript and degrades no worse than every other mutation in this
application; a JSON submission would need client-side redirect handling to
reach Stripe Checkout and adds a failure mode (a JS error silently drops the
submission) that an HTML form's native submit does not have. Progressive
enhancement (Stimulus field-level validation, a loading spinner while
Checkout redirects) sits on top of the same HTML form, per the architecture's
own "progressive enhancement, not a SPA" framing. Recorded in `## Decisions`.

---

## Error contract, CSRF, redirects, pagination

### HTML routes

- **Validation failure** on a Form POST: re-render the **same** template,
  same URL, HTTP `422`, the bound Form carrying field-level errors — never a
  redirect, so input is not lost and focus can move to the first invalid
  field (a11y). Symfony's normal Form re-display behavior; `422` chosen over
  the common `200` because the request did fail and caching/monitoring
  should be able to tell.
- **Success**: `303 See Other` to a `GET` route. A flash message is set in
  one of four categories — `success`, `error`, `warning`, `info` — rendered
  once from the base layout.
- **Unauthenticated** access to a gated route: `302` to `/login` with
  `_target_path` preserved, not a bare `401` — this is an HTML application,
  not an API client.
- **Authorization denied** (`denyAccessUnlessGranted` throws): `403`, a
  static "Access Denied" page. Never names the resource, the trainer, or the
  reason.
- **Cross-tenant access is structurally a `404`, not a `403`.** This is the
  one place tenancy design changes the HTTP contract directly: a
  foreign-tenant entity is invisible to both the Doctrine filter (layer 2)
  and RLS (layer 5), so a `#[MapEntity]` lookup for another trainer's `Event`
  id returns nothing **before any voter runs** — the request 404s the same
  way a nonexistent id would. This is deliberate (architect-architecture.md,
  "Tenancy enforcement," lines 120–209): it prevents confirming that a
  resource exists in a trainer the actor cannot see. A same-tenant,
  wrong-owner or wrong-role request (a coach outside their assigned events,
  a player viewing another family's data) **is** found by the query and is
  the `403` case instead — the voter is what denies it.
- **Not found, in general**: `404`, generic template, no distinction between
  "never existed" and "existed but you can't see it" beyond the tenant case
  above.
- **Rate-limited**: `429`, generic "too many attempts, try again in N
  minutes" — never states whether the *specific* email/IP is the one
  throttled, to avoid confirming account existence via timing.

### JSON routes (§2 above) and the webhook

RFC 9457 problem details, per the skill's standard shape:

```json
{
  "type": "https://practiceperfect.example/problems/validation-failed",
  "title": "Validation failed",
  "status": 422,
  "code": "validation_failed",
  "violations": [{ "propertyPath": "code", "message": "This coupon code does not exist." }]
}
```

`401` (not `302`) for an unauthenticated JSON request — there is no page to
redirect to. `403`/`404` follow the same tenant-hiding rule as HTML. The
webhook returns `200` with an empty body on success (including on a detected
duplicate — Stripe must see `200`, or it retries a delivery already handled)
and a bare `400` on a signature failure, with no problem-details body, since
the caller is Stripe, not a browser or a documented API consumer.

### CSRF placement

- **Form POSTs**: Symfony's default per-form token (framework default,
  `csrf_protection: true`), token id `{action}-{resourceId}` matching the
  pattern already established in `examples/symfony-clean-code-patterns.md`
  §1 (`cancel-order-{id}`) — e.g. `cancel-rsvp-{rsvpId}`,
  `deactivate-user-{accountId}`.
- **JSON endpoints (§2)**: no session-rendered hidden field to carry a
  token, so the base layout emits one `<meta name="csrf-token">` per page
  load and the four JSON routes that mutate send it back as an
  `X-CSRF-Token` header, validated the same way (`isCsrfTokenValid`).
- **The webhook**: exempt. No session, no cookie, no ambient authority to
  forge — protected by Stripe's HMAC signature instead (see "Stripe webhook
  contract").

### Pagination

**50 per page, platform-wide**, matching the three places the epics state a
page size explicitly and agree with each other: AC-03-5 (player list),
AC-02-56 (Event Master), AC-07-12 (Users tool). Applied as the default for
every other list route the epics leave unstated, rather than inventing a
second number. `page` is the only client-controlled query parameter (1-
indexed); page size is fixed server-side, not client-adjustable, to bound
worst-case query cost. Sort and filter parameters are named and whitelisted
per route in the tables below — an unrecognized value is ignored, not a
`400`, since these are bookmarkable `GET` URLs a stale link should not break.
CSV exports (item 4 of the response contract) are explicitly **not**
paginated — the trainer expects the whole list — and stream via
`StreamedResponse` rather than buffering, since Epic-01's own 10,000-user
scale target (AC-01-77) makes an unstreamed export a realistic memory risk.

---

## Route map by module

Table columns: **Route** (name) · **Method** · **Path** · **Gate**
(`access_control`) · **Voter → Subject** · **Request** (Form/DTO) ·
**Response** · **ACs** (traceability). Unless a row says otherwise, the
workflow service is named `{Verb}{Resource}Service` after the route's own
action — only architecture-named services (`RsvpToEventService`,
`MembershipService`, `TokenLedgerService`, etc.) are called out by name.

Three names recur below as an informal shorthand for "the role's landing
page after login" rather than as routes of their own: `trainer_dashboard`
resolves to `crm_trainer_quick_view` (Epic-03's Quick View is the only
screen any epic describes as the trainer's landing page); `portal_dashboard`
resolves to `scheduling_portal_calendar` (no epic describes a distinct
player/parent dashboard separate from the Training Calendar); `coach_dashboard`
resolves to `scheduling_coach_activities` (My Activities). No epic names a
fourth, separate "dashboard" screen for either role that would need its own
route — these three aliases exist only so the redirect targets throughout
this document read as "go to your dashboard" rather than repeating the same
underlying route name every time.

### Platform module

Deliberately the smallest table here. Per the module map, Platform "owns...
Nothing [it] calls; must not... depend on any other module" — it is
infrastructure every other module's controllers sit on top of (tenant
resolution, the Doctrine filter, RLS session variable, branding runtime,
`CrossTenantReadService`, `AdministrativeScope`, the audit writer), not a
console of its own. Branding is rendered by `BrandingProvider` into every
page's `<head>` (a Twig global, not a route). Impersonation and
administrative-tenant-scope routes are triggered from **Administration**'s
console and are listed there, even though the entities and the session
mechanics they drive (`ImpersonationSession`, `AdministrativeScope`) are
Platform-owned — see "Impersonation and administrative tenant scope."

| Route | Method | Path | Gate | Voter → Subject | Request | Response | ACs |
|---|---|---|---|---|---|---|---|
| `platform_context_trainer_switch` | POST | `/context/trainer/{trainer}` | `ROLE_PLAYER` | `—` (validated against the actor's own `AccountTrainerLink` rows, not a voter — there is nothing to authorize beyond "is this my own link") | none (path param only) | Redirect to referer or `portal_dashboard`; sets session `current_trainer_id`, re-validated against `AccountTrainerLink` every request per Tenancy Layer 3 source 3 | AC-01-15 |

Coaches never switch context (BR-01-11: exactly one trainer), and trainers
are always their own tenant (resolution source 4) — so this is the only
context-switch route Platform needs. The **child**-context switch is a
separate, non-tenancy concern ("which of my children am I acting for") and
is listed under Identity.

### Identity module

Owns Epic-01: accounts, profiles, parent/child links, player profiles,
memberships, ShareLinks, availability, child approval, auth.

**Auth and account lifecycle** — all `PUBLIC_ACCESS` except where noted;
no voter needed, these gate on token/credential validity, not ownership.

| Route | Method | Path | Gate | Voter → Subject | Request | Response | ACs |
|---|---|---|---|---|---|---|---|
| `identity_auth_login` | GET, POST | `/login` | `PUBLIC_ACCESS` | `—` | `LoginType` (email, password) | Redirect to role dashboard on success; re-render with a generic "invalid credentials" on failure (never "wrong password" vs. "no such account") | AC-01-65, BR-01-1..3, BR-01-6 |
| `identity_auth_logout` | POST | `/logout` | `IS_AUTHENTICATED_FULLY` | `—` | none | Handled by the security component, no controller body | AC-01-69 |
| `identity_password_forgot` | GET, POST | `/password/forgot` | `PUBLIC_ACCESS` | `—` | `RequestPasswordResetType` (email) | Always the same "if that email exists, a reset link is on its way" — enumeration-safe by construction, api-designer default | BR-01-4 |
| `identity_password_reset` | GET, POST | `/password/reset/{token}` | `PUBLIC_ACCESS` | `—` | `ResetPasswordType` (new password) | Redirect to login with success flash; expired/used token → 404-style generic error page, not "token expired" (avoids confirming a token was ever valid) | AC-01-66, BR-01-4 |
| `identity_account_setup` | GET, POST | `/account/setup/{token}` | `PUBLIC_ACCESS` | `—` | `SetupPasswordType` | First-login credential setup after Super Admin creates a trainer (AC-01-3/4); redirect to `trainer_dashboard` | AC-01-3, AC-01-4, AC-01-5 |
| `identity_email_verify` | GET | `/email/verify/{token}` | `PUBLIC_ACCESS` | `—` | none | Marks verified, redirect to login with flash; see Open questions — whether login is blocked before verification is Q-01.05 (email-verification wording), unresolved | BR-01-5, AC-01-67 |
| `identity_email_verify_resend` | POST | `/email/verify/resend` | `IS_AUTHENTICATED_FULLY` | `AccountVoter::EMAIL_VERIFY_RESEND → Account` (new — self only) | none | Redirect back with flash | BR-01-5 |

**Registration via code** — the ShareLink landing pages. `/join/{code}` is
the static, unlimited-use player mass-invite (BR-01-14, AC-03-59);
`/invite/{code}` is the unique, limited-use link, reused for **both**
Epic-01's coach invitation (BR-01-15) and Epic-03's coach-issued player
invitation (AC-03-50), since both are stated with the identical URL shape
and only the stored `ShareLink.type` differs — see Decisions. **Neither
route resolves a tenant context on the anonymous GET** — see "Public
unauthenticated routes and tenant resolution" for why that is a genuine gap
in the architecture's resolution-source list, not settled here.

| Route | Method | Path | Gate | Voter → Subject | Request | Response | ACs |
|---|---|---|---|---|---|---|---|
| `identity_sharelink_join_show` | GET | `/join/{code}` | `PUBLIC_ACCESS` | `ShareLinkVoter::SHARELINK_RESOLVE → ShareLink` (new — checks the link is active, not expired/exhausted; nothing to check about the viewer) | none | Anonymous: register-or-login prompt. Authenticated non-child: redirect to `identity_sharelink_associate`. Authenticated child login: "ask your parent" page, triggers parent email, no association (AC-01-31) | AC-01-9, AC-01-31 |
| `identity_sharelink_join_register` | POST | `/join/{code}/register` | `PUBLIC_ACCESS` | `ShareLinkVoter::SHARELINK_RESOLVE → ShareLink` | `PlayerRegistrationType` (name, email, password, parent phone, player name/age/gender) | Redirect to `portal_dashboard`; creates `Account` + `PlayerProfile`, then `MembershipService::associate(..., source: 'sharelink')` | AC-01-10, AC-01-11, AC-01-12, BR-01-14, BR-01-28 |
| `identity_sharelink_associate` | GET, POST | `/join/{code}/associate` | `ROLE_PLAYER` | `ShareLinkVoter::SHARELINK_RESOLVE → ShareLink` | `FamilyMemberSelectionType` ("who trains with [Trainer]?" — self and/or children, multi-select) | Redirect to `portal_dashboard`; `MembershipService::associate` once per selected family member | AC-01-13, AC-01-14 |
| `identity_sharelink_invite_show` | GET | `/invite/{code}` | `PUBLIC_ACCESS` | `ShareLinkVoter::SHARELINK_RESOLVE → ShareLink` | none | Branches on `ShareLink.type`: coach invite → coach registration prompt; coach-issued player invite → same as `identity_sharelink_join_show`'s authenticated branch. Expired: "this invitation has expired," with a trainer/coach-facing resend action elsewhere, not on this page (BR-01-15, AC-01-42) | AC-03-50, AC-03-51 |
| `identity_sharelink_invite_register` | POST | `/invite/{code}/register` | `PUBLIC_ACCESS` | `ShareLinkVoter::SHARELINK_RESOLVE → ShareLink` | `CoachRegistrationType` | Redirect to `coach_dashboard`; creates `Account` + `CoachMembership` (status Pending or Active per AC-01-40) | AC-01-39, AC-01-40, AC-01-41 |

**Coach invitation (trainer-initiated)** and **ShareLink management**:

| Route | Method | Path | Gate | Voter → Subject | Request | Response | ACs |
|---|---|---|---|---|---|---|---|
| `identity_trainer_coaches_index` | GET | `/trainer/coaches` | `ROLE_TRAINER` | `—` (tenant-scoped query, own roster) | filters: status | Twig list | US-01.08 narrative |
| `identity_trainer_coach_invite` | GET, POST | `/trainer/coaches/invite` | `ROLE_TRAINER` | `CoachMembershipVoter::COACH_INVITE → —` (new — class-level: trainer must have Stripe-independent capacity, no other check) | `InviteCoachType` (email, optional name/message) | Redirect to `identity_trainer_coaches_index`; generates a 7-day `ShareLink` (type=coach-invite), emails it | AC-01-39, BR-01-15 |
| `identity_trainer_coach_resend` | POST | `/trainer/coaches/{coachMembership}/resend` | `ROLE_TRAINER` | `CoachMembershipVoter::COACH_INVITE → CoachMembership` | csrf only | Redirect back with flash; reissues a fresh 7-day link | AC-01-42 |
| `identity_trainer_sharelinks_index` | GET | `/trainer/sharelinks` | `ROLE_TRAINER` | `—` (own tenant) | none | Twig: static mass-invite link + optional unique per-player links, opens/joins counts (BR-01-27) | AC-01-73, AC-03-59, AC-03-61..63 |
| `identity_trainer_sharelinks_unique_create` | POST | `/trainer/sharelinks/unique` | `ROLE_TRAINER` | `ShareLinkVoter::SHARELINK_CREATE → —` | `UniqueShareLinkType` (recipient name/email) | Redirect back with the new link shown in a flash/panel | AC-03-61 (optional MVP) |
| `identity_coach_player_invite` | POST | `/coach/players/invite` | `ROLE_COACH` | `ShareLinkVoter::SHARELINK_CREATE → —` | `InvitePlayerType` (optional recipient email) | Redirect back with the new `/invite/{code}` link shown | AC-03-50, AC-03-52 |

**Family and child management** — parent-only; `ChildProfileVoter` denies a
child login on every one of these (AC-01-30's "cannot... change trainer
associations").

| Route | Method | Path | Gate | Voter → Subject | Request | Response | ACs |
|---|---|---|---|---|---|---|---|
| `identity_portal_family_index` | GET | `/portal/family` | `ROLE_PLAYER` | `—` (own children via `ParentChildLink`) | none | Twig: children + their trainer associations | AC-01-22 |
| `identity_portal_child_create` | GET, POST | `/portal/family/children/new` | `ROLE_PLAYER` | `ChildProfileVoter::CHILD_PROFILE_CREATE → —` (new) | `ChildProfileType` (name, age 1–18, gender, optional school/photo) | Redirect to `identity_portal_family_index`; duplicate-name/age warning is non-blocking (AC-01-21) | AC-01-16, AC-01-21 |
| `identity_portal_child_edit` | GET, POST | `/portal/family/children/{child}/edit` | `ROLE_PLAYER` | `ChildProfileVoter::CHILD_PROFILE_EDIT → PlayerProfile` | `ChildProfileType` | Redirect to `identity_portal_family_index` | AC-01-18, AC-01-19 |
| `identity_portal_child_trainer_add` | GET, POST | `/portal/family/children/{child}/trainers/add` | `ROLE_PLAYER` | `ChildProfileVoter::CHILD_TRAINER_ADD → PlayerProfile` | `AddTrainerType` (ShareLink code, or select from "My Trainers") | Redirect to `identity_portal_family_index` | AC-01-17, AC-01-23 |
| `identity_portal_child_trainer_remove` | POST | `/portal/family/children/{child}/trainers/{trainer}/remove` | `ROLE_PLAYER` | `ChildProfileVoter::CHILD_TRAINER_REMOVE → PlayerProfile` | csrf + confirmation checkbox | Redirect with flash; warns of upcoming-RSVP cancellation before this POST is reachable (confirmation step in the GET-rendered page) | AC-01-24 |
| `identity_portal_child_token_approval` | POST | `/portal/family/children/{child}/token-approval` | `ROLE_PLAYER` | `ChildProfileVoter::CHILD_TOKEN_APPROVAL_EDIT → PlayerProfile` | `TokenApprovalToggleType` (on/off) | Redirect back with flash | AC-01-27, AC-01-28 |
| `identity_portal_context_child_switch` | POST | `/portal/context/child/{child}` | `ROLE_PLAYER` | `ChildProfileVoter::CHILD_PROFILE_VIEW → PlayerProfile` | none | Redirect to referer; sets session `current_player_context_id` | AC-01-18, AC-04-26 |

**Child approval** (USD or token purchases pending parent sign-off):

| Route | Method | Path | Gate | Voter → Subject | Request | Response | ACs |
|---|---|---|---|---|---|---|---|
| `identity_portal_approvals_index` | GET | `/portal/approvals` | `ROLE_PLAYER` | `—` (own pending requests) | none | Twig list; expired-but-unprocessed rows show "Expired" — computed from the 48h timestamp at read time, not a job (architect-architecture.md, "Time-based state is derived at read time," line 572) | AC-01-25, BR-01-18 |
| `identity_portal_approval_approve` | POST | `/portal/approvals/{approval}/approve` | `ROLE_PLAYER` | `ChildApprovalVoter::CHILD_APPROVAL_DECIDE → ChildApprovalRequest` (new) | `ApprovalDecisionType` (optional note) | Redirect with flash; for a USD request this triggers Stripe Checkout (303 to Stripe) rather than an immediate confirmation — see Billing module | AC-01-26, BR-01-20 |
| `identity_portal_approval_deny` | POST | `/portal/approvals/{approval}/deny` | `ROLE_PLAYER` | `ChildApprovalVoter::CHILD_APPROVAL_DECIDE → ChildApprovalRequest` | `ApprovalDecisionType` (optional note) | Redirect with flash; child notified | AC-01-26, BR-01-20 |

**Availability ("Best Times")**:

| Route | Method | Path | Gate | Voter → Subject | Request | Response | ACs |
|---|---|---|---|---|---|---|---|
| `identity_portal_availability_edit` | GET, POST | `/portal/availability` | `ROLE_PLAYER` | `AvailabilityVoter::AVAILABILITY_EDIT → —` (new — self, or the child selected via the context switch above) | `AvailabilityGridType` (per-day toggle + custom ranges) | Redirect back with flash; per-child via the `current_player_context_id` session value | AC-01-43, AC-01-44 |
| `identity_coach_availability_edit` | GET, POST | `/coach/availability` | `ROLE_COACH` | `AvailabilityVoter::AVAILABILITY_EDIT → —` | `CoachAvailabilityType` (weekly recurring, multiple slots/day) | Redirect back with flash | AC-01-46 |

**Profile (any role) and portal branding (trainer)**:

| Route | Method | Path | Gate | Voter → Subject | Request | Response | ACs |
|---|---|---|---|---|---|---|---|
| `identity_account_profile_edit` | GET, POST | `/account/profile` | `IS_AUTHENTICATED_FULLY` | `AccountVoter::ACCOUNT_EDIT → Account` (self) | `EditProfileType` — common fields plus a role-specific field subset resolved server-side from the actor's role (AC-01-51); email/role/skill-level rendered read-only, never bound | Redirect back with flash; photo upload is a field on the same multipart form, not a separate endpoint | AC-01-48, AC-01-49, AC-01-50, AC-01-51 |
| `identity_trainer_branding_edit` | GET, POST | `/trainer/branding` | `ROLE_TRAINER` | `TrainerSettingsVoter::TRAINER_SETTINGS_EDIT → TrainerBrandingSettings` | `PortalBrandingType` (logo upload PNG/JPG/SVG ≤2MB, hex color) | Redirect back with flash; applies immediately for the whole org (AC-01-62) since branding is read per-request, never cached (architect-architecture.md line 597) | AC-01-60, AC-01-61, AC-01-62, AC-01-63 |

### Scheduling module

Owns Epic-02: events, RSVPs, attendance, coach assignments, invitations,
duplication. Calls Billing to take payment and Growth to price a coupon;
never writes a token entry or CRM state directly.

**Trainer console — Event Builder**:

| Route | Method | Path | Gate | Voter → Subject | Request | Response | ACs |
|---|---|---|---|---|---|---|---|
| `scheduling_trainer_event_index` | GET | `/trainer/events` | `ROLE_TRAINER` | `—` (own tenant) | filters: status, type, date range | Twig list | US-02.01 narrative |
| `scheduling_trainer_event_create` | GET, POST | `/trainer/events/new` | `ROLE_TRAINER` | `EventVoter::EVENT_CREATE → —` | `EventType` (title, type, start/end, location, capacity, description, eligibility, pricing, coach) | Redirect to `scheduling_trainer_event_show` | AC-02-1, AC-02-2, AC-02-3, BR-02-1..4 |
| `scheduling_trainer_event_availability_check` | GET | `/trainer/events/availability-check` | `ROLE_TRAINER` | `—` (read-only, own tenant) | query: `date`, `start`, `end` | **JSON** `{ "eligible": 20, "available": 15 }` — see JSON vs. HTML §2 | AC-02-12, AC-01-45 |
| `scheduling_trainer_event_show` | GET | `/trainer/events/{event}` | `ROLE_TRAINER` | `EventVoter::EVENT_VIEW → Event` | none | Twig: detail + tabs (RSVP List, Attendance) | US-02.12 narrative |
| `scheduling_trainer_event_edit` | GET, POST | `/trainer/events/{event}/edit` | `ROLE_TRAINER` | `EventVoter::EVENT_EDIT → Event` | `EventType` | Redirect to `scheduling_trainer_event_show`; past events unreachable (AC-02-54); capacity-below-RSVP-count blocks save (AC-02-53); date/time/location/coach/price changes fan out targeted notifications synchronously (single emails stay sync per architect-architecture.md line 570) | AC-02-50..54, BR-02-13, BR-02-15 |
| `scheduling_trainer_event_duplicate` | GET, POST | `/trainer/events/{event}/duplicate` | `ROLE_TRAINER` | `EventVoter::EVENT_DUPLICATE → Event` | `DuplicateEventType` (new date/time required, rest pre-filled and optional) | Redirect to the new event's `scheduling_trainer_event_show`; `EventDuplicationRecord` written for analytics | AC-02-14..17, BR-02-19 |
| `scheduling_trainer_event_cancel` | POST | `/trainer/events/{event}/cancel` | `ROLE_TRAINER` | `EventVoter::EVENT_CANCEL → Event` | `CancelEventType` (reason, required) | Redirect to `scheduling_trainer_event_index` with flash; commits the cancellation synchronously, fans out per-player refunds asynchronously (architect-architecture.md, "Trainer-initiated cancellation never holds both," line 493) | AC-02-46..49, AC-02-68, BR-02-12 |
| `scheduling_trainer_event_rsvps` | GET | `/trainer/events/{event}/rsvps` | `ROLE_TRAINER` | `EventVoter::EVENT_VIEW_RSVP_LIST → Event` | none | Twig list: name, age, skill, RSVP time, payment status, availability indicator | AC-02-43, AC-02-44 |
| `scheduling_trainer_event_rsvps_export` | GET | `/trainer/events/{event}/rsvps/export` | `ROLE_TRAINER` | `EventVoter::EVENT_EXPORT_RSVPS → Event` | none | CSV, streamed | AC-02-45 |
| `scheduling_trainer_event_rsvp_add` | POST | `/trainer/events/{event}/rsvps/add` | `ROLE_TRAINER` | `EventVoter::EVENT_MANUAL_ADD_PLAYER → Event` | `ManualAddPlayerType` (player select) | Redirect back with flash; allowed only while under capacity (Q-02.09's default — see Open questions) | AC-02-45, Q-02.09 |
| `scheduling_trainer_event_rsvp_remove` | POST | `/trainer/events/{event}/rsvps/{rsvp}/remove` | `ROLE_TRAINER` | `RsvpVoter::RSVP_REMOVE → Rsvp` (new — trainer-only removal, distinct from a player's own cancellation) | csrf only | Redirect back with flash; issues a refund if the RSVP was paid | AC-02-45 |
| `scheduling_trainer_event_attendance` | GET, POST | `/trainer/events/{event}/attendance` | `ROLE_TRAINER` | `AttendanceVoter::ATTENDANCE_RECORD → Event` (trainer branch: any time, overrides the coach's entries) | `TakeAttendanceType` (per-player status: Present/Absent/Late/Excused, collection form) | Redirect back with flash | AC-02-41, BR-02-18 |

**Coach console — assignments and attendance**:

| Route | Method | Path | Gate | Voter → Subject | Request | Response | ACs |
|---|---|---|---|---|---|---|---|
| `scheduling_coach_activities` | GET | `/coach/activities` | `ROLE_COACH` | `—` (scoped by `CoachVisibilityService`) | tab: `to-confirm` \| `assigned` | Twig list | US-02.10, US-02.11 narrative |
| `scheduling_coach_assignment_confirm` | POST | `/coach/assignments/{assignment}/confirm` | `ROLE_COACH` | `CoachAssignmentVoter::COACH_ASSIGNMENT_CONFIRM → CoachAssignment` (new) | csrf only | Redirect back with flash; notifies trainer | AC-02-35 |
| `scheduling_coach_assignment_decline` | POST | `/coach/assignments/{assignment}/decline` | `ROLE_COACH` | `CoachAssignmentVoter::COACH_ASSIGNMENT_DECLINE → CoachAssignment` | `DeclineAssignmentType` (optional reason) | Redirect back with flash; notifies trainer to find a replacement | AC-02-36 |
| `scheduling_coach_event_attendance` | GET, POST | `/coach/events/{event}/attendance` | `ROLE_COACH` | `AttendanceVoter::ATTENDANCE_RECORD → Event` (coach branch: assigned + event started + same-day-only once recorded, delegates reach to `CoachVisibilityService`) | `TakeAttendanceType` | Redirect back with flash | AC-02-38..42, BR-02-16..18 |

A trainer who assigns **themselves** as coach (AC-02-8) reaches attendance
through their own `scheduling_trainer_event_attendance` route above, which
already grants trainers any-time access regardless of who is assigned
(AC-02-41). Whether **assignment confirmation** itself applies to a
self-assigned trainer — i.e. does BR-02-13's "Pending until coach confirms"
require the trainer to click Confirm on their own assignment, or does
self-assignment skip that state entirely — is not stated anywhere in Epic-02
and is recorded under Open questions rather than decided here; no
trainer-facing confirm route is designed for it.

**Player/Parent portal — calendar and RSVP**:

| Route | Method | Path | Gate | Voter → Subject | Request | Response | ACs |
|---|---|---|---|---|---|---|---|
| `scheduling_portal_calendar` | GET | `/portal/calendar` | `ROLE_PLAYER` | `—` (eligible + invited events in the current trainer context) | query: `view` (month/week/day), `date`, filters | Twig; availability-match badge if the player set availability | AC-02-18..22, AC-02-64 |
| `scheduling_portal_event_show` | GET | `/portal/events/{event}` | `ROLE_PLAYER` | `EventVoter::EVENT_VIEW → Event` — private events deny (404-shaped "Access Denied," AC-02-7) for non-invited players even if otherwise eligible | none | Twig detail + RSVP action | BR-02-5, BR-02-6 |
| `scheduling_portal_event_rsvp` | POST | `/portal/events/{event}/rsvp` | `ROLE_PLAYER` | `RsvpVoter::RSVP_CREATE → Event` — denies on capacity/eligibility/duplicate/past-event (AC-02-28), and (via `ChildApprovalVoter` delegation) routes a child's request to Pending-Approval instead of confirming | `RsvpType` (`paymentMethod`: free \| token \| card; never a client-supplied price) | Free + adult: redirect to `scheduling_portal_reservations` with confirmation. Paid + card: 303 to Stripe Checkout (Billing). Paid + token: `TokenLedgerService::spend` inline, redirect with confirmation. Child: redirect to a "Pending Parent Approval" page | AC-02-23..28, BR-02-7..10 |
| `scheduling_portal_reservations` | GET | `/portal/reservations` | `ROLE_PLAYER` | `—` (own RSVPs in current trainer context) | filters: upcoming/past | Twig list | US-02.06/07 narrative |
| `scheduling_portal_rsvp_cancel` | POST | `/portal/rsvps/{rsvp}/cancel` | `ROLE_PLAYER` | `RsvpVoter::RSVP_CANCEL → Rsvp` — denies after event start (AC-02-30); a child's cancellation routes through `ChildApprovalVoter` the same as a purchase (AC-02-33) | csrf only | Redirect back with flash; refund per the 24-hour policy (BR-02-11), full refund unconditionally if the event itself was trainer-cancelled | AC-02-29..33, BR-02-11 |

### Crm module

Owns Epic-03: labels, flags, notes, segmentation, Quick View. May read
Scheduling and Content; writes memberships only through Identity, attendance
not at all.

| Route | Method | Path | Gate | Voter → Subject | Request | Response | ACs |
|---|---|---|---|---|---|---|---|
| `crm_trainer_players_index` | GET | `/trainer/players` | `ROLE_TRAINER` | `—` (own tenant) | `q` (search), filters (skill, age, gender, labels, flags, team, attendance bands, registration date, last-activity band — AC-03-22..24), `sort` | Twig list, live-updating via a Turbo Frame on `q`/filter change (not JSON — see JSON vs. HTML) | AC-03-1..10, AC-03-22..26, BR-03-13..15 |
| `crm_trainer_player_show` | GET | `/trainer/players/{membership}` | `ROLE_TRAINER` | `PlayerVoter::PLAYER_VIEW → PlayerTrainerMembership` | none | Twig: profile, labels, flags, notes, event history, coach feedback (read-only) | AC-03-27..33 |
| `crm_trainer_player_edit` | POST | `/trainer/players/{membership}/edit` | `ROLE_TRAINER` | `PlayerVoter::PLAYER_EDIT → PlayerTrainerMembership` | `PlayerCrmFieldsType` (skill level, and the free-text limited-field set AC-03-33 names) | Redirect back with flash | AC-03-33 |
| `crm_trainer_labels_index` | GET | `/trainer/labels` | `ROLE_TRAINER` | `—` | none | Twig: label list with per-label usage count | AC-03-14 |
| `crm_trainer_label_create` | GET, POST | `/trainer/labels/new` | `ROLE_TRAINER` | `LabelVoter::LABEL_MANAGE → —` (new) | `LabelType` (name ≤50 chars, color) — unique case-insensitively within the trainer, enforced as a custom Validator constraint plus a DB unique index | Redirect to `crm_trainer_labels_index` | AC-03-11, BR-03-3 |
| `crm_trainer_label_edit` | GET, POST | `/trainer/labels/{label}/edit` | `ROLE_TRAINER` | `LabelVoter::LABEL_MANAGE → Label` | `LabelType` | Redirect to `crm_trainer_labels_index` | AC-03-13 |
| `crm_trainer_label_delete` | POST | `/trainer/labels/{label}/delete` | `ROLE_TRAINER` | `LabelVoter::LABEL_MANAGE → Label` | csrf + confirmation ("Remove [Label] from [N] players?") | Redirect with flash; removes from every player | AC-03-13, BR-03-5 |
| `crm_trainer_player_label_add` | POST | `/trainer/players/{membership}/labels` | `ROLE_TRAINER` | `PlayerVoter::PLAYER_LABEL_MANAGE → PlayerTrainerMembership` | `ApplyLabelsType` (multi-select) | Redirect back with flash | AC-03-12 |
| `crm_trainer_player_label_remove` | POST | `/trainer/players/{membership}/labels/{label}/remove` | `ROLE_TRAINER` | `PlayerVoter::PLAYER_LABEL_MANAGE → PlayerTrainerMembership` | csrf only | Redirect back with flash | AC-03-28 |
| `crm_trainer_player_flag_add` | POST | `/trainer/players/{membership}/flags` | `ROLE_TRAINER` (+ `ROLE_COACH` — see note) | `PlayerVoter::PLAYER_FLAG_MANAGE → PlayerTrainerMembership` | `ApplyFlagType` (one of the 8 system flags, optional note) | Redirect back with flash | AC-03-15, BR-03-6, BR-03-7 |
| `crm_trainer_player_flag_resolve` | POST | `/trainer/players/{membership}/flags/{flag}/resolve` | `ROLE_TRAINER` | `PlayerVoter::PLAYER_FLAG_MANAGE → PlayerTrainerMembership` | csrf + "Mark as resolved?" confirmation | Redirect back with flash; hides from active view, keeps history | AC-03-17, BR-03-8 |
| `crm_trainer_player_note_add` | POST | `/trainer/players/{membership}/notes` | `ROLE_TRAINER` | `PlayerVoter::PLAYER_NOTE_MANAGE → PlayerTrainerMembership` | `AddNoteType` (text ≤1000 chars; general, or event-scoped via an optional event field) | Redirect back with flash | AC-03-19, AC-03-20 |
| `crm_trainer_player_note_edit` | POST | `/trainer/players/{membership}/notes/{note}/edit` | `ROLE_TRAINER` | `PlayerVoter::PLAYER_NOTE_MANAGE → PlayerTrainerMembership` — denies after 24h and denies on a coach-authored note (read-only to the trainer) | `AddNoteType` | Redirect back with flash | AC-03-21, BR-03-12 |
| `crm_trainer_player_note_delete` | POST | `/trainer/players/{membership}/notes/{note}/delete` | `ROLE_TRAINER` | `PlayerVoter::PLAYER_NOTE_MANAGE → PlayerTrainerMembership` | csrf only | Redirect back with flash | AC-03-21 |
| `crm_trainer_quick_view` | GET | `/trainer/dashboard` | `ROLE_TRAINER` | `—` | query: none (fixed windows per BR-03-16..19) | Twig dashboard: events this week, RSVPs by type, Top 10 Players, flag counts (clickable → `crm_trainer_players_index` pre-filtered), ShareLink opens/joins this week | AC-03-34..41, BR-03-16..19 |
| `crm_coach_players_index` | GET | `/coach/players` | `ROLE_COACH` | `—` (scoped by `CoachVisibilityService` — a coach assigned to zero events sees zero players) | `q` (scoped to assigned events only) | Twig list | AC-03-43, AC-03-64, AC-03-65 |
| `crm_coach_player_show` | GET | `/coach/players/{membership}` | `ROLE_COACH` | `PlayerVoter::PLAYER_VIEW → PlayerTrainerMembership` — delegates reach to `CoachVisibilityService`; a player with no shared session history is unreachable (404-shaped), per architect-architecture.md's Open architecture risk 6 | none | Twig: coach-scoped detail — attendance with this coach, read-only labels/flags, past feedback | AC-03-44, AC-03-45 |
| `crm_coach_player_feedback_add` | POST | `/coach/players/{membership}/feedback` | `ROLE_COACH` | `PlayerVoter::PLAYER_FEEDBACK_ADD → PlayerTrainerMembership` | `SessionFeedbackType` (session select from recent shared sessions, text ≤500 chars) | Redirect back with flash | AC-03-46..48 |
| `crm_coach_player_feedback_edit` | POST | `/coach/players/{membership}/feedback/{feedback}/edit` | `ROLE_COACH` | `PlayerVoter::PLAYER_FEEDBACK_EDIT → PlayerTrainerMembership` — denies after 24h, own feedback only | `SessionFeedbackType` | Redirect back with flash | AC-03-49 |

`crm_trainer_player_flag_add` lists both `ROLE_TRAINER` and `ROLE_COACH` at
the coarse gate because the source itself is unresolved here — US-03.04
"Permissions" states a coach can apply flags "(optional MVP)" as if decided,
while Q-03.05 asks the same question as open (see Open questions). The route
is designed to be reachable from both `/trainer/players/{id}` and
`/coach/players/{id}`; `PlayerVoter::PLAYER_FLAG_MANAGE`'s coach clause is
the actual on/off switch once Q-03.05 resolves, so no route change is needed
either way — only the voter's coach branch flips.

### Content module

Owns Epic-04: playlists, content items, drills, assignments, progress,
access grants, usage. Calls Billing to take payment; never writes a token
entry or CRM state.

Playlist visibility is two orthogonal facts, per architect-architecture.md
("The publication exception," line 364, and "Playlist visibility storage" in
Decisions, line 661): **publication** (public library or not) and
**in-tenant audience** (players+coaches, or coaches-only). One Form field
pair (`PlaylistVisibilityType`: `publication`, `audience`) covers both A9's
three states and Epic-04's under-specified Coach-Only Content bullet
(AC-04-41) without a third route.

| Route | Method | Path | Gate | Voter → Subject | Request | Response | ACs |
|---|---|---|---|---|---|---|---|
| `content_trainer_learn_index` | GET | `/trainer/content/learn` | `ROLE_TRAINER` | `—` (own tenant) | none | Twig list | US-04.01 narrative |
| `content_trainer_learn_create` | GET, POST | `/trainer/content/learn/new` | `ROLE_TRAINER` | `PlaylistVoter::PLAYLIST_CREATE → —` | `LearnPlaylistType` (title, description, filters, visibility, ordered video list) | Redirect to `content_trainer_playlist_show` | AC-04-1..3 |
| `content_trainer_practice_index` | GET | `/trainer/content/practice` | `ROLE_TRAINER` | `—` | none | Twig list | US-04.04 narrative |
| `content_trainer_practice_create` | GET, POST | `/trainer/content/practice/new` | `ROLE_TRAINER` | `PlaylistVoter::PLAYLIST_CREATE → —` | `PracticePlaylistType` (title, description, filters, visibility, ordered drill list — own, public, or inline-new) | Redirect to `content_trainer_playlist_show` | AC-04-11, AC-04-12 |
| `content_trainer_playlist_show` | GET | `/trainer/content/playlists/{playlist}` | `ROLE_TRAINER` | `PlaylistVoter::PLAYLIST_VIEW → Playlist` | none | Twig detail | US-04.09 narrative |
| `content_trainer_playlist_edit` | GET, POST | `/trainer/content/playlists/{playlist}/edit` | `ROLE_TRAINER` | `PlaylistVoter::PLAYLIST_EDIT → Playlist` | `LearnPlaylistType` or `PracticePlaylistType` | Redirect back with flash "Playlist updated!"; already-assigned players see it immediately, prior progress untouched, new items start "not started" | AC-04-31..33, BR-04-17 |
| `content_trainer_playlist_items_reorder` | POST | `/trainer/content/playlists/{playlist}/items/reorder` | `ROLE_TRAINER` | `PlaylistVoter::PLAYLIST_EDIT → Playlist` | **JSON** `{ "order": ["item-id", "item-id", ...] }` | **JSON** 204 — see JSON vs. HTML §2 | AC-04-2, AC-04-12 |
| `content_trainer_playlist_visibility` | POST | `/trainer/content/playlists/{playlist}/visibility` | `ROLE_TRAINER` | `PlaylistVoter::PLAYLIST_PUBLISH_TOGGLE → Playlist` | `PlaylistVisibilityType` (`publication`: public\|private, `audience`: players_and_coaches\|coaches_only) | Redirect back with flash; publication is a one-way "has ever been published" fact at the storage layer even when reverted to private display (architect-architecture.md line 377) | AC-04-17, AC-04-18, AC-04-41, BR-04-3..5 |
| `content_trainer_playlist_delete` | POST | `/trainer/content/playlists/{playlist}/delete` | `ROLE_TRAINER` | `PlaylistVoter::PLAYLIST_DELETE → Playlist` | csrf + confirmation ("...unassign it from all players") | Redirect with flash; soft delete (AC-04-36); other trainers' references show "Content unavailable" | AC-04-34, AC-04-36 |
| `content_trainer_playlist_assign` | GET, POST | `/trainer/content/playlists/{playlist}/assign` | `ROLE_TRAINER` | `PlaylistVoter::PLAYLIST_ASSIGN → Playlist` | `AssignPlaylistType` (players multi-select, or label group, or attribute filter; optional due date + note) | Redirect back with flash; notifies assignees | AC-04-13..16, BR-04-13..15 |
| `content_trainer_drills_index` | GET | `/trainer/content/drills` | `ROLE_TRAINER` | `—` | `scope`: mine \| public; `q`, filters (category, difficulty, equipment, duration) | Twig list, live-updating via Turbo Frame | AC-04-7, AC-04-9, BR-04-19 |
| `content_trainer_drill_create` | GET, POST | `/trainer/content/drills/new` | `ROLE_TRAINER` | `ContentItemVoter::CONTENT_ITEM_CREATE → —` | `DrillType` (name, instructions, YouTube URL, difficulty, equipment, space, player count, duration range, categories, visibility) | Redirect to drill show | AC-04-4..6 |
| `content_trainer_drill_show` | GET | `/trainer/content/drills/{drill}` | `ROLE_TRAINER` | `ContentItemVoter::CONTENT_ITEM_VIEW → ContentItem` — public drills are readable cross-tenant per the publication exception | none | Twig: preview + "Add to Playlist" | AC-04-8 |
| `content_trainer_drill_edit` | GET, POST | `/trainer/content/drills/{drill}/edit` | `ROLE_TRAINER` | `ContentItemVoter::CONTENT_ITEM_EDIT → ContentItem` | `DrillType` | Redirect back with flash | US-04.02 narrative |
| `content_trainer_drill_delete` | POST | `/trainer/content/drills/{drill}/delete` | `ROLE_TRAINER` | `ContentItemVoter::CONTENT_ITEM_DELETE → ContentItem` | csrf + confirmation naming affected playlist count | Redirect with flash; other trainers' references show "Drill unavailable" | AC-04-35 |
| `content_trainer_drill_visibility` | POST | `/trainer/content/drills/{drill}/visibility` | `ROLE_TRAINER` | `ContentItemVoter::CONTENT_ITEM_PUBLISH_TOGGLE → ContentItem` | `PublishToggleType` | Redirect back with flash | AC-04-17, AC-04-18 |
| `content_trainer_playlist_drill_add` | POST | `/trainer/content/playlists/{playlist}/drills` | `ROLE_TRAINER` | `PlaylistVoter::PLAYLIST_EDIT → Playlist` | `AddDrillType` (drill id — own or public) | Redirect back with flash "Drill added to [Playlist]"; stores a reference, not a copy (BR-04-12) | AC-04-8, BR-04-12 |
| `content_trainer_youtube_metadata` | GET | `/trainer/content/youtube-metadata` | `ROLE_TRAINER` | `—` | query: `url` | **JSON** `{ "title": "...", "durationSeconds": 245 }` — see JSON vs. HTML §2 | AC-04-2 |
| `content_portal_index` | GET | `/portal/content` | `ROLE_PLAYER` | `—` (current trainer context) | `tab`: learn \| practice \| progress | Twig | AC-04-20 — "Perfect" tab not carried forward, PERFECT pillar out of MVP (see Open questions) |
| `content_portal_playlist_show` | GET | `/portal/content/playlists/{playlist}` | `ROLE_PLAYER` | `PlaylistVoter::PLAYLIST_VIEW → Playlist` | none | Twig: locked (price + purchase CTA) or unlocked (item list with completion checkmarks) | AC-04-21..23 |
| `content_portal_playlist_checkout` | POST | `/portal/content/playlists/{playlist}/checkout` | `ROLE_PLAYER` | `PlaylistVoter::PLAYLIST_PURCHASE → Playlist` — child login routes through `ChildApprovalVoter` first | `PurchaseMethodType` (`method`: card \| token; optional `couponCode`) | Token: `PurchasePlaylistAccessService` (architecture-named) unlocks inline, redirect with confirmation. Card: 303 to Stripe Checkout | AC-05-18, AC-05-19, BR-04-6..9 |
| `content_portal_item_play` | GET | `/portal/content/items/{item}/play` | `ROLE_PLAYER` | `ContentItemVoter::CONTENT_ITEM_VIEW → ContentItem` — denies if the owning playlist is locked and unpurchased | none | Twig: embedded YouTube player + instructions | AC-04-24 |
| `content_portal_item_complete` | POST | `/portal/content/items/{item}/complete` | `ROLE_PLAYER` | `ContentItemVoter::CONTENT_ITEM_VIEW → ContentItem` | **JSON**, empty body — fired from the YouTube IFrame API's `onStateChange` (PLAYING), not a page navigation | **JSON** `{ "completed": true }` — see JSON vs. HTML §2 (extends the four listed there; same reasoning: a widget event, not a page) | AC-04-25, BR-04-16 |
| `content_portal_progress` | GET | `/portal/content/progress` | `ROLE_PLAYER` | `—` (own data) | none | Twig dashboard | AC-04-27..29 |
| `content_super_admin_analytics` | GET | `/super-admin/content/analytics` | `ROLE_SUPER_ADMIN` | `—` (`CrossTenantReadService`, audit-logged per call) | `trainer` (optional drill-down) | Twig dashboard; CSV export via `content_super_admin_analytics_export` | AC-04-37..40 |
| `content_super_admin_analytics_export` | GET | `/super-admin/content/analytics/export` | `ROLE_SUPER_ADMIN` | `—` | none | CSV, streamed | AC-04-40 |

A trainer viewing **a player's** content progress (AC-04-30) is a tab on
`crm_trainer_player_show` (Crm module), reading Content's repositories
through Content's own service — not a separate Content-module route, per the
"a module may read another module's entities through that module's
repository or service" boundary rule.

### Billing module

Owns Epic-05: the token ledger, balances, payment records, the Stripe
gateway, Connect settings, entitlements, webhook receipts. Called by
Scheduling, Content, Growth and Forms; never calls up to any of them. No
route here creates a Stripe Checkout Session as its own HTTP step — every
purchase-initiating route in Scheduling/Content/Billing itself calls
`StripeGateway` internally and performs the `303` redirect to the URL Stripe
returns in the same request; there is no separate "create checkout session"
endpoint for the browser to hit.

**Trainer console — Stripe Connect and pricing**:

| Route | Method | Path | Gate | Voter → Subject | Request | Response | ACs |
|---|---|---|---|---|---|---|---|
| `billing_trainer_settings` | GET | `/trainer/billing` | `ROLE_TRAINER` | `TrainerSettingsVoter::TRAINER_SETTINGS_VIEW → TrainerBillingSettings` | none | Twig: Connect status, token pricing, subscription price, earnings summary | US-05.01, US-05.09 narrative |
| `billing_trainer_stripe_connect` | POST | `/trainer/billing/stripe/connect` | `ROLE_TRAINER` | `TrainerSettingsVoter::TRAINER_SETTINGS_EDIT → TrainerBillingSettings` | none | 303 to Stripe Connect Express onboarding | AC-05-1, AC-05-2 |
| `billing_trainer_stripe_return` | GET | `/trainer/billing/stripe/return` | `ROLE_TRAINER` | `TrainerSettingsVoter::TRAINER_SETTINGS_EDIT → TrainerBillingSettings` | none | Verifies onboarding status against Stripe, redirects to `billing_trainer_settings` with flash | AC-05-1 |
| `billing_trainer_stripe_refresh` | GET | `/trainer/billing/stripe/refresh` | `ROLE_TRAINER` | `TrainerSettingsVoter::TRAINER_SETTINGS_EDIT → TrainerBillingSettings` | none | Regenerates an expired Account Link, 303 to Stripe | AC-05-1 |
| `billing_trainer_pricing_edit` | GET, POST | `/trainer/billing/pricing` | `ROLE_TRAINER` | `TrainerSettingsVoter::TRAINER_SETTINGS_EDIT → TrainerBillingSettings` | `TokenPricingType` (per-token price, package sizes/discounts) | Redirect back with flash; blocked ("Connect Stripe first") until Connect is complete (AC-05-3) | AC-05-1..2, AC-05-32, BR-05-1 |
| `billing_trainer_gift_tokens` | POST | `/trainer/players/{membership}/gift-tokens` | `ROLE_TRAINER` | `TokenVoter::TOKEN_GIFT → PlayerTrainerMembership` | `GiftTokensType` (amount, required note) | Redirect back with flash; `TokenLedgerService` writes a `gift` entry, audit-logged | AC-05-33 |
| `billing_trainer_earnings` | GET | `/trainer/billing/earnings` | `ROLE_TRAINER` | `TrainerSettingsVoter::TRAINER_SETTINGS_VIEW → TrainerBillingSettings` | none | Twig: `StripeReportingReader`-sourced figures only, link to Stripe Express Dashboard; on Stripe unavailability shows an error, never a locally-summed fallback (architect-architecture.md line 552) | AC-05-25, AC-05-26 |

**Player/Parent portal — tokens, subscription, payment methods, history**:

| Route | Method | Path | Gate | Voter → Subject | Request | Response | ACs |
|---|---|---|---|---|---|---|---|
| `billing_portal_tokens` | GET | `/portal/tokens` | `ROLE_PLAYER` | `—` (own balance, current trainer context) | none | Twig: balance + recent activity | AC-05-4, BR-05-2 |
| `billing_portal_tokens_purchase` | GET, POST | `/portal/tokens/purchase` | `ROLE_PLAYER` | `TokenVoter::TOKEN_PURCHASE → —` — a child login triggers `ChildApprovalVoter` instead of completing directly (AC-01-27) | `TokenPurchaseType` (package select or custom amount) | 303 to Stripe Checkout | AC-05-4..6 |
| `billing_portal_subscription_purchase` | GET, POST | `/portal/subscription/purchase` | `ROLE_PLAYER` | `TokenVoter::TOKEN_SUBSCRIPTION_PURCHASE → —` (new attribute — same voter, no separate `SubscriptionVoter`) | `SubscriptionPurchaseType` (activation date) | 303 to Stripe Checkout; entitlement granted only on webhook-confirmed payment, no grace period (BR-05-14) | AC-05-29, BR-05-14 |
| `billing_portal_checkout_success` | GET | `/portal/checkout/success` | `ROLE_PLAYER` | `—` | query: `context` (rsvp\|content\|tokens\|subscription) for the copy shown | Twig: "processing — we'll email you" (actual confirmation is the webhook, not this request) | BR-05-6 |
| `billing_portal_checkout_cancel` | GET | `/portal/checkout/cancel` | `ROLE_PLAYER` | `—` | query: `context` | Twig: "checkout cancelled," links back | BR-05-8 |
| `billing_portal_payment_methods_manage` | POST | `/portal/billing/payment-methods/manage` | `ROLE_PLAYER` | `PaymentMethodVoter::PAYMENT_METHOD_MANAGE → —` (new — denies a child login outright, AC-01-30) | none | 303 to the Stripe Customer Portal | AC-05-20, AC-05-21 |
| `billing_portal_transactions` | GET | `/portal/transactions` | `ROLE_PLAYER` | `—` (own, current trainer context only — BR-05-17) | filters: date range, type, payment method | Twig list, each row linking to a Stripe-hosted receipt | AC-05-22, AC-05-23 |

No route exists for Epic-05's optional "Family Overview" (AC-05-24) — **A6
drops it** ("No combined cross-trainer view on any player-facing screen,"
`requirements-analyst-open-questions.md` line 36). Its absence here is
deliberate, not an oversight.

**Stripe webhook**:

| Route | Method | Path | Gate | Voter → Subject | Request | Response | ACs |
|---|---|---|---|---|---|---|---|
| `billing_webhook_stripe_receive` | POST | `/webhooks/stripe` | `PUBLIC_ACCESS` on a dedicated **stateless, `security: false`** firewall (`^/webhooks/`) | none — signature verification stands in for authorization | Raw Stripe `Event` JSON + `Stripe-Signature` header | `200` empty body (success **and** detected duplicate); `400` empty body (bad signature) | AC-05-34..37, BR-08-14 |

Full mechanics in "Stripe webhook contract" below.

### Growth module

Owns Epic-06: referral links, referrals, assist counts, coupons,
redemptions. Grants a reward by calling Billing; never writes a token entry
itself, never owns the reward record (it is a `TokenEntry` of kind
`referral_reward`, per architect-architecture.md's ledger design, line 665).

| Route | Method | Path | Gate | Voter → Subject | Request | Response | ACs |
|---|---|---|---|---|---|---|---|
| `growth_portal_referrals` | GET | `/portal/referrals` | `ROLE_PLAYER` | `—` (own link, current trainer context) | none | Twig: link, Share button (client-side clipboard copy — no server route), referral-count stats | AC-06-1..3 |
| `growth_trainer_referrals_dashboard` | GET | `/trainer/marketing/referrals` | `ROLE_TRAINER` | `—` (own tenant; requires the Marketing feature toggle — `FeatureGate`) | date range preset | Twig: overview metrics, Top Referrers leaderboard, Activity Log | AC-06-13..16 |
| `growth_trainer_coupons_index` | GET | `/trainer/marketing/coupons` | `ROLE_TRAINER` | `—` (Marketing feature toggle) | none | Twig list + all-coupons summary | AC-06-26, AC-06-28 |
| `growth_trainer_coupon_create` | GET, POST | `/trainer/marketing/coupons/new` | `ROLE_TRAINER` | `CouponVoter::COUPON_CREATE → —` | `CouponType` (code 4–20 alphanumeric/hyphen, discount type/value, applies-to, usage limit, expiration, status) | Redirect to `growth_trainer_coupons_index` | AC-06-17, AC-06-18, BR-06-8, BR-06-9 |
| `growth_trainer_coupon_edit` | GET, POST | `/trainer/marketing/coupons/{coupon}/edit` | `ROLE_TRAINER` | `CouponVoter::COUPON_EDIT → Coupon` | `CouponType` | Redirect back with flash | AC-06-27 |
| `growth_trainer_coupon_deactivate` | POST | `/trainer/marketing/coupons/{coupon}/deactivate` | `ROLE_TRAINER` | `CouponVoter::COUPON_DEACTIVATE → Coupon` | csrf only | Redirect back with flash | AC-06-27 |
| `growth_trainer_coupon_delete` | POST | `/trainer/marketing/coupons/{coupon}/delete` | `ROLE_TRAINER` | `CouponVoter::COUPON_DELETE → Coupon` — denies if the coupon has ever been used | csrf + confirmation | Redirect back with flash | AC-06-27 |
| `growth_trainer_coupon_usage` | GET | `/trainer/marketing/coupons/{coupon}/usage` | `ROLE_TRAINER` | `CouponVoter::COUPON_VIEW_ANALYTICS → Coupon` | none | Twig: redemption list | AC-06-27, AC-06-28 |
| `growth_portal_coupon_validate` | POST | `/portal/checkout/coupon/validate` | `ROLE_PLAYER` | `CouponVoter::COUPON_APPLY → Coupon` (new — resolved by code + current trainer; checks active/unexpired/under-limit; eligibility itself is Q-06.10, unresolved — see Open questions) | **JSON** `{ "code": "SUMMER20", "purchaseType": "rsvp", "purchaseId": "..." }` | **JSON** `{ "valid": true, "discountedAmountCents": 1600, "message": "20% off with SUMMER20" }` or `{ "valid": false, "message": "Invalid or expired code" }` — see JSON vs. HTML §2 | AC-06-20..23, BR-06-9 |
| `growth_super_admin_referral_rules` | GET, POST | `/super-admin/growth/referral-rules` | `ROLE_SUPER_ADMIN` | `PlatformConfigurationVoter::PLATFORM_CONFIG_EDIT → PlatformConfiguration` (new) | `ReferralRuleType` (referrals required, tokens awarded, referee welcome bonus toggle) | Redirect back with flash; applies to every trainer immediately, existing assist counts preserved (AC-06-30); audit-logged | AC-06-29..31 |

The coupon code entered on `growth_portal_coupon_validate` is re-validated
**again** at final checkout submission inside the owning route
(`scheduling_portal_event_rsvp`'s or `content_portal_playlist_checkout`'s
`PurchaseMethodType.couponCode` field) — the JSON call is a UX convenience,
never the authoritative check, since the two requests are not atomic with
each other.

### Administration module

Owns Epic-07: the Super Admin console, feature toggles, operational
dashboards, the Event Master tool, the audit log viewer. May call every
module's services; must never write a trainer-scoped row without an active
`AdministrativeScope`, must never read another module's repositories
directly (it goes through that module's own service, same as any caller).

| Route | Method | Path | Gate | Voter → Subject | Request | Response | ACs |
|---|---|---|---|---|---|---|---|
| `administration_dashboard` | GET | `/super-admin/dashboard` | `ROLE_SUPER_ADMIN` | `—` | `range`: 7\|30\|90 days (default 30) | Twig: operational metrics only, "View Financial Reports in Stripe" link-out, **no revenue/payout/transaction figure anywhere on this page** (AC-07-7) | AC-07-1..7 |
| `administration_users_index` | GET | `/super-admin/users` | `ROLE_SUPER_ADMIN` | `—` (`CrossTenantReadService`) | `q`, `role`, `status` | Twig list | AC-01-72, AC-07-8..10, AC-07-12 |
| `administration_user_show` | GET | `/super-admin/users/{account}` | `ROLE_SUPER_ADMIN` | `AccountVoter::ACCOUNT_VIEW → Account` (+ SUPER_ADMIN) | none | Twig profile | AC-07-11 |
| `administration_user_edit` | GET, POST | `/super-admin/users/{account}/edit` | `ROLE_SUPER_ADMIN` | `AccountVoter::ACCOUNT_EDIT → Account` (+ SUPER_ADMIN) | `AdminEditAccountType` (name, email — with re-verification warning, status; role and trainer view-only; player-specific fields editable if the account is a player) | Redirect back with flash; audit-logged | AC-01-71, AC-07-13..15 |
| `administration_trainer_create` | GET, POST | `/super-admin/trainers/new` | `ROLE_SUPER_ADMIN` | `AccountVoter::ACCOUNT_CREATE_TRAINER → —` (new) | `CreateTrainerType` (business name, trainer name, email, phone, subscription tier) | Redirect to `administration_trainer_show`; creates the `Account`+`Trainer` (Identity) and the `PlatformSubscription`+Stripe subscription (Billing), sends the setup-link email | AC-01-1..8, AC-07-16, AC-07-17 |
| `administration_user_deactivate` | POST | `/super-admin/users/{account}/deactivate` | `ROLE_SUPER_ADMIN` | `AccountVoter::ACCOUNT_DEACTIVATE → Account` (new) | csrf + confirmation | Redirect back with flash; also the trainer-deactivation action — a Trainer is an Account with role Trainer, no separate route | AC-01-52, AC-01-53, AC-07-39 |
| `administration_user_reactivate` | POST | `/super-admin/users/{account}/reactivate` | `ROLE_SUPER_ADMIN` | `AccountVoter::ACCOUNT_REACTIVATE → Account` (new) | csrf only | Redirect back with flash | AC-01-54 |
| `administration_user_delete` | POST | `/super-admin/users/{account}/delete` | `ROLE_SUPER_ADMIN` | `AccountVoter::ACCOUNT_DELETE_GDPR → Account` (new) | csrf + explicit "cannot be undone" confirmation | Redirect back with flash; anonymizes personal fields, permanent, audit-logged with a reason | AC-01-55..59 |
| `administration_impersonation_start` | POST | `/super-admin/users/{account}/impersonate` | `ROLE_SUPER_ADMIN` | `ImpersonationVoter::IMPERSONATION_START → Account` (new — denies a Super-Admin target, BR-01-21) | csrf + confirmation modal naming the target | Redirect to the target's own dashboard (`trainer_dashboard`/`coach_dashboard`/`portal_dashboard`); see "Impersonation and administrative tenant scope" | AC-01-33, AC-01-34, AC-01-37 |
| `impersonation_exit` | POST | `/impersonation/exit` | `IS_IMPERSONATOR` (Symfony's `switch_user` marker role — reachable from **any** prefix, not just `/super-admin/...`, since the acting session is running under the target's own role by design) | `ImpersonationVoter::IMPERSONATION_END → —` | csrf only | Redirect to `administration_dashboard` | AC-01-35 |
| `administration_trainers_index` | GET | `/super-admin/trainers` | `ROLE_SUPER_ADMIN` | `—` (`CrossTenantReadService`) | `q` | Twig list | AC-07-38 |
| `administration_trainer_show` | GET | `/super-admin/trainers/{trainer}` | `ROLE_SUPER_ADMIN` | `—` (`CrossTenantReadService` for the summary; Stripe subscription status via `StripeReportingReader`) | none | Twig profile | AC-07-38 |
| `administration_trainer_features_edit` | GET, POST | `/super-admin/trainers/{trainer}/features` | `ROLE_SUPER_ADMIN` | `PlatformConfigurationVoter::PLATFORM_CONFIG_EDIT → FeatureToggle` — no `AdministrativeScope` needed, `FeatureToggle` is global (architect-architecture.md line 315) | `FeatureTogglesType` (LPPP, Marketing, Camps — three independent on/off) | Redirect back with flash; confirmation names the trainer + feature + effect; applies within seconds; audit-logged | AC-07-18..21, BR-07-1..3 |
| `administration_trainer_fee_edit` | GET, POST | `/super-admin/trainers/{trainer}/fees` | `ROLE_SUPER_ADMIN` | `TrainerSettingsVoter::TRAINER_FEE_EDIT → Trainer` (+ opens `AdministrativeScope` for `{trainer}` — `TrainerBillingSettings` **is** trainer-scoped) | `TrainerFeeType` (subscription amount, application fee %) | Redirect back with flash; subscription rate applies next billing cycle, fee rate applies to new transactions immediately; audit-logged with old/new values | AC-05-27, AC-05-28 |
| `administration_event_master_index` | GET | `/super-admin/events` | `ROLE_SUPER_ADMIN` | `—` (`CrossTenantReadService`) | `q`, `trainer`, `date range`, `type`, `status`; `sort`: date\|trainer\|capacity | Twig list | AC-07-22..24, AC-07-26 |
| `administration_event_master_show` | GET | `/super-admin/events/{event}` | `ROLE_SUPER_ADMIN` | `EventVoter::EVENT_VIEW → Event` (+ SUPER_ADMIN, opens `AdministrativeScope` for the event's trainer to load the real entity rather than a projection) | none | Twig detail; links to edit/cancel/RSVP-list below | AC-07-25 |
| `administration_event_master_edit` | GET, POST | `/super-admin/events/{event}/edit` | `ROLE_SUPER_ADMIN` | `EventVoter::EVENT_EDIT → Event` (+ SUPER_ADMIN via `AdministrativeScope`) | `EventType` | Redirect back with flash; scheduling-conflict warnings suppressed entirely (AC-07-27), the override itself is what gets audit-logged (AC-07-28) | AC-07-25, AC-07-27, AC-07-28 |
| `administration_event_master_cancel` | POST | `/super-admin/events/{event}/cancel` | `ROLE_SUPER_ADMIN` | `EventVoter::EVENT_CANCEL → Event` (+ SUPER_ADMIN via `AdministrativeScope`) | `CancelEventType` | Redirect back with flash; same refund fan-out as a trainer cancellation | AC-02-48, AC-07-25 |
| `administration_event_master_rsvps` | GET | `/super-admin/events/{event}/rsvps` | `ROLE_SUPER_ADMIN` | `EventVoter::EVENT_VIEW_RSVP_LIST → Event` (+ SUPER_ADMIN via `AdministrativeScope`) | none | Twig list | AC-07-25 |
| `administration_audit_log_index` | GET | `/super-admin/audit-log` | `ROLE_SUPER_ADMIN` | `AuditVoter::AUDIT_LOG_VIEW → —` | date range, action type, `q` (subject name) | Twig chronological list | AC-07-29..32, BR-07-4..6 |
| `administration_audit_log_export` | GET | `/super-admin/audit-log/export` | `ROLE_SUPER_ADMIN` | `AuditVoter::AUDIT_LOG_VIEW → —` | same filters as above | CSV, streamed | AC-07-33 |
| `administration_stripe_dashboard_link` | GET | `/super-admin/stripe` | `ROLE_SUPER_ADMIN` | `—` | none | 302 to the Stripe Express Dashboard, SSO'd | AC-07-6, AC-07-35, AC-07-36 |
| `administration_trainer_stripe_dashboard_link` | GET | `/super-admin/trainers/{trainer}/stripe` | `ROLE_SUPER_ADMIN` | `—` | none | 302 to that trainer's Connect account dashboard | AC-07-37 |

Viewing dashboards, searching Users, and viewing the audit log are
**explicitly not audit-logged themselves** — BR-07-5 states this as a
negative rule ("too verbose") — so `administration_dashboard`,
`administration_users_index` and `administration_audit_log_index` write no
audit entry on their own `GET`, even though the `CrossTenantReadService`
calls underneath the first two still write their own **crossing-read** audit
entry per the architecture's unconditional "every crossing read writes an
audit entry" rule (line 182) — two different audit obligations that happen
to overlap on the same request without contradicting each other.

### Forms module

Owns Epic-08: camp and evaluation forms, public submission, conversion to
account. Calls Identity to convert, Billing to take payment; must never
create a membership itself (that is `MembershipService`'s exclusive job,
source `camp_registration`) or write a payment record itself.

**Trainer console**:

| Route | Method | Path | Gate | Voter → Subject | Request | Response | ACs |
|---|---|---|---|---|---|---|---|
| `forms_trainer_index` | GET | `/trainer/forms` | `ROLE_TRAINER` | `—` (own tenant; requires the Camps feature toggle) | none | Twig list: camps + evaluations, submission counts, remaining spots, shareable links | AC-08-27, AC-08-32 |
| `forms_trainer_camp_create` | GET, POST | `/trainer/forms/camps/new` | `ROLE_TRAINER` | `FormVoter::FORM_CREATE → —` | `CampFormType` (name, display-only dates, description, capacity 1–1000, optional price $0 or $1–10,000, field builder — Text/Email/Dropdown/Multi-select only, ≥1 field) | Redirect to `forms_trainer_form_edit` | AC-08-1..5, BR-08-1..4 |
| `forms_trainer_evaluation_create` | GET, POST | `/trainer/forms/evaluations/new` | `ROLE_TRAINER` | `FormVoter::FORM_CREATE → —` | `EvaluationFormType` (name, description, optional price, field builder) | Redirect to `forms_trainer_form_edit` | AC-08-9, AC-08-10, BR-08-1..4 |
| `forms_trainer_form_preview` | GET | `/trainer/forms/{form}/preview` | `ROLE_TRAINER` | `FormVoter::FORM_EDIT → Form` | none | Twig: renders the same template `forms_public_show` uses, read-only, reachable pre-publish | AC-08-6, AC-08-11 |
| `forms_trainer_form_edit` | GET, POST | `/trainer/forms/{form}/edit` | `ROLE_TRAINER` | `FormVoter::FORM_EDIT → Form` | Same Form type as create | Redirect back with flash; warns of the current registration count if fields/pricing change and submissions already exist | AC-08-28 |
| `forms_trainer_form_publish` | POST | `/trainer/forms/{form}/publish` | `ROLE_TRAINER` | `FormVoter::FORM_EDIT → Form` | csrf only | Redirect back with flash; generates the shareable `/forms/{code}` link on first publish | AC-08-7 |
| `forms_trainer_form_toggle` | POST | `/trainer/forms/{form}/toggle` | `ROLE_TRAINER` | `FormVoter::FORM_TOGGLE → Form` — camps only, a no-op/hidden control for evaluations (BR-08-5) | csrf only | Redirect back with flash; disabled camps show "Registration Closed" on the public link | AC-08-8, AC-08-29 |
| `forms_trainer_form_delete` | POST | `/trainer/forms/{form}/delete` | `ROLE_TRAINER` | `FormVoter::FORM_DELETE → Form` — denies while any paid, unrefunded registration exists | csrf + confirmation | Redirect back with flash | AC-08-31 |
| `forms_trainer_submissions_index` | GET | `/trainer/forms/{form}/submissions` | `ROLE_TRAINER` | `FormVoter::FORM_VIEW_SUBMISSIONS → Form` | `paymentStatus`: paid\|free\|pending (see Open questions re: pending's own visibility); `conversionStatus`: converted\|not-converted | Twig list | AC-08-17, AC-08-18, AC-08-22 |
| `forms_trainer_submissions_export` | GET | `/trainer/forms/{form}/submissions/export` | `ROLE_TRAINER` | `FormVoter::FORM_EXPORT → Form` | same filters | CSV, streamed | AC-08-19 |
| `forms_trainer_submission_attendance` | POST | `/trainer/forms/{form}/submissions/attendance` | `ROLE_TRAINER` | `FormVoter::FORM_MARK_ATTENDANCE → Form` | `FormAttendanceType` (per-submission checkbox, collection form) | Redirect back with flash | AC-08-20 |
| `forms_trainer_submissions_bulk_email` | POST | `/trainer/forms/{form}/submissions/bulk-email` | `ROLE_TRAINER` | `FormVoter::FORM_SEND_BULK_EMAIL → Form` | `BulkEmailType` (subject, body) | Redirect back with flash | AC-08-21 |

Super Admin reaches any trainer's forms "via impersonation mode" per
BR-08-21's own wording, not a dedicated `/super-admin/forms/...` screen —
this is the one Epic-07/Epic-08 boundary the source itself resolves, so no
Administration-module route is designed for it.

**Public, unauthenticated** — full tenant-resolution treatment in "Public
unauthenticated routes and tenant resolution" below.

| Route | Method | Path | Gate | Voter → Subject | Request | Response | ACs |
|---|---|---|---|---|---|---|---|
| `forms_public_show` | GET | `/forms/{code}` | `PUBLIC_ACCESS` | `FormSubmissionVoter::FORM_SUBMISSION_CREATE → Form` — evaluates form state only (exists, active, under capacity), never actor identity, since there is no actor | none | `200`: full form (name, dates, description, price, spots remaining) if open; "Camp Full" if at capacity; "Registration Closed" if a disabled camp. `404`: code never resolves to a `Form` | AC-08-13, AC-08-15, BR-08-3, BR-08-5, BR-08-7 |
| `forms_public_submit` | POST | `/forms/{code}` | `PUBLIC_ACCESS` | `FormSubmissionVoter::FORM_SUBMISSION_CREATE → Form` | A **dynamically built** Symfony Form, assembled at runtime from `Form.fieldDefinitions` (Text/Email/Dropdown/Multi-select) via `FormFactoryInterface` — not a static class — plus one honeypot field (silently accepted-but-discarded if filled, never surfaced as a validation error, to avoid tipping off the bot) and a per-form uniqueness check on (form, email) (BR-08-10) | Free: `303` to `forms_public_confirmation`. Paid: `303` to Stripe Checkout, registration held un-confirmed until the webhook fires (BR-08-8, BR-08-14) | AC-08-14..16, BR-08-6, BR-08-9..11 |
| `forms_public_confirmation` | GET | `/forms/{code}/confirmation` | `PUBLIC_ACCESS` | `FormSubmissionVoter::FORM_SUBMISSION_CREATE → Form` (same attribute — this page is part of the same anonymous, code-scoped flow) | query: `submission` (opaque token, not the raw id — avoids enumerable submission ids appearing in a bookmarkable URL) | Twig: confirmation + "Create Your Account" CTA | AC-08-16 |
| `forms_public_convert_account` | GET, POST | `/forms/{code}/convert-account` | `PUBLIC_ACCESS` | `FormSubmissionVoter::FORM_SUBMISSION_CONVERT → FormSubmission` (new — checks the submission belongs to this form/code and is not already converted) | `ConvertSubmissionToAccountType` (password, terms acceptance; name/email pre-filled read-only from the submission) | Redirect to `portal_dashboard`, now authenticated. If the submission's email already has an `Account`: redirect to `identity_auth_login` with a "log in instead" flash (AC-08-26) rather than rendering the form at all | AC-08-23..26, BR-08-15..18 |

Declining conversion is not a route — it is simply not clicking the CTA. The
"ShareLink for later registration" that follows automatically (AC-08-25)
lands the registrant on `identity_sharelink_join_show` at some later,
**authenticated** moment — which is exactly the general ShareLink-acceptance
tenant-resolution gap flagged in the next section, not a new one specific to
Forms.

---

## Stripe webhook contract

One receiver (`billing_webhook_stripe_receive`, `POST /webhooks/stripe`)
for every Stripe event the platform consumes, including Epic-08's camp
payment confirmations (BR-08-14) — there is no second webhook endpoint for
Forms, since Epic-08's own payments already go through the same Stripe
Checkout mechanism (BR-08-11) as every other purchase (BR-05-6).

### Firewall

A dedicated firewall segment, ahead of `main` in the chain:

```yaml
webhook:
    pattern: ^/webhooks/
    stateless: true
    security: false
```

`stateless: true` — no session is started for this request at all, matching
"no persistent PDO connections" from the Risks section (line 692): a webhook
request must not pick up or leak a tenant session variable across requests.
`security: false` — Symfony's authentication layer is not involved; the
signature check inside the controller **is** the authorization.

### Step by step

1. **Read the raw body and the `Stripe-Signature` header.** Never
   `$request->toArray()` or anything that re-serializes the body — Stripe's
   signature is computed over the exact bytes received, and re-encoding
   before verifying breaks it.
2. **Verify the signature** via the Stripe SDK
   (`\Stripe\Webhook::constructEvent($payload, $sigHeader, $endpointSecret)`),
   with the SDK's default 5-minute timestamp tolerance against replay. A
   `SignatureVerificationException` → `400`, no body, nothing persisted, no
   message dispatched.
3. **Insert the idempotency row first, in its own transaction, before
   parsing the event's business meaning.** `StripeEventReceipt(eventId,
   eventType, rawPayload, receivedAt, status: 'pending')`, with a unique
   constraint on `eventId`. This is the architecture's own stated mechanism
   verbatim: "the Stripe event id is inserted into the uniquely-constrained
   `StripeEventReceipt` table first, in its own transaction; the unique
   violation *is* the duplicate detection" (line 536). `StripeEventReceipt`
   is global (no trainer known yet — architect-architecture.md line 318), so
   this insert needs no tenant context at all.
4. **On a unique-constraint violation** (a redelivery of an event already
   received): return `200` immediately. Nothing is re-dispatched. This is
   not an error case for Stripe — it is the expected shape of at-least-once
   delivery.
5. **On a successful insert**, dispatch `ProcessStripeWebhookEvent
   (stripeEventReceiptId: string)` on the `doctrine://` transport and return
   `200` immediately. The message carries **only the receipt row's id** —
   not the payload, not the event type — matching
   `examples/symfony-clean-code-patterns.md` §8's "the payload is stable"
   guidance; the handler re-reads `rawPayload` from the persisted row rather
   than trusting a payload that traveled through the queue.
6. **The async handler** loads the receipt, sets `TenantContext` from
   whatever the event's own metadata resolves to (the payment record's
   trainer, once that record is found — see below), dispatches to one
   per-event-type handler service, and updates the receipt's `status`
   (`processed` \| `failed`) and `processedAt`. A handler failure lets
   Messenger's retry policy run; Stripe's own delivery retry (up to 3 days)
   is the second, outer safety net, made redundant-but-harmless by step 4.

### Events consumed and effect (AC-05-34)

| Event | Effect |
|---|---|
| `payment_intent.succeeded` | Confirms the RSVP / unlocks the content / marks a camp registration Paid (BR-08-14), by looking up the `PaymentRecord` the `PaymentIntent` id was stored against at Checkout-session creation — never by trusting client-supplied metadata on the intent itself |
| `payment_intent.payment_failed` | Notifies the payer; RSVP/registration stays unconfirmed, spot stays open (BR-02-9, BR-05-8) |
| `charge.refunded` | Updates transaction history, restores tokens if the original spend was token-funded (BR-05-11); handlers reconcile to the state Stripe reports rather than applying a delta, so redelivery or out-of-order arrival cannot double-refund |
| `customer.subscription.created` | Grants the `SubscriptionEntitlement` — only from here, never from the purchase-initiating request itself, so a payment that never confirms grants nothing (BR-05-14's "no grace period") |
| `customer.subscription.deleted` | Revokes the entitlement |
| `account.updated` | Refreshes the trainer's Connect/onboarding status shown on `billing_trainer_settings` |

### Idempotency, outbound

Distinct from the inbound receipt check above: every Stripe **write** call
(`StripeGateway`, the platform's only caller of the Stripe write API) derives
its idempotency key deterministically from a `PaymentRecord` persisted
**before** the call, so a client-side timeout-and-retry reuses the same key
rather than double-charging (architect-architecture.md line 541).

### Reliability targets

99%+ webhook success rate, `<2s` processing per event, up to 50
webhooks/minute, 3 retry attempts over 24 hours on the handler side
(AC-05-35, AC-05-38) — the outer bound Stripe itself enforces (3 days of
delivery retry) is longer than anything this platform needs to guarantee on
its own.

---

## Public unauthenticated routes and tenant resolution

Every `PUBLIC_ACCESS` route in this document, and how each resolves a
tenant — or deliberately does not.

| Route(s) | Resolves a tenant on the anonymous request? | Mechanism |
|---|---|---|
| `identity_auth_login`, `identity_password_forgot`, `identity_password_reset`, `identity_account_setup`, `identity_email_verify` | No | These act on a global `Account` — no trainer-scoped row is touched before the user is authenticated |
| `identity_sharelink_join_show`, `identity_sharelink_invite_show` (`GET`) | **Yes — resolution source 5, code-resolved** | `ShareLink` is trainer-scoped, not global, so even rendering the landing page is a trainer-scoped *read*; and `ShareLinkOpen` (BR-03-22) makes this GET a trainer-scoped *write* as well |
| `identity_sharelink_join_register`, `identity_sharelink_invite_register` (`POST`) | **Yes — resolution source 5** | The code in the route, via the global `PublicTenantCode` mapping |
| `identity_sharelink_associate` (already-authenticated acceptance) | **Yes — resolution source 5, which outranks the session context** | Without the precedence rule an existing player accepting a second trainer's link would resolve to the trainer already in their session, and the membership INSERT would be rejected by RLS (AC-01-13/AC-01-14) |
| `identity_portal_child_trainer_add` | Yes — resolution source 3, as clarified | The target trainer is supplied by the request and validated against the global `AccountTrainerLink` |
| `forms_public_show`, `forms_public_submit`, `forms_public_confirmation`, `forms_public_convert_account` | **Yes — the same code-resolved source, shared with Epic-01's ShareLink routes** | Tenancy Layer 3, resolution source 5: a public code carried in the route, looked up in `PublicTenantCode` |
| `billing_webhook_stripe_receive` | No, at the HTTP boundary — resolved later, async, by the handler once it identifies which `PaymentRecord` the event belongs to | See "Stripe webhook contract," step 6 |

### The gap: creating the first membership from an anonymous ShareLink click

The architecture's resolution-source list (Tenancy enforcement, Layer 3,
lines 150–159) has no source covering the moment
`MembershipService::associate(...)` writes the **first**
`PlayerTrainerMembership` (or `CoachMembership`) row for a (player, trainer)
pair from a ShareLink acceptance:

- **Source 3** ("the player's or coach's selected trainer context... validated
  against the global `AccountTrainerLink`") cannot apply — that
  `AccountTrainerLink` row is exactly what this request is creating; nothing
  exists yet to validate against.
- **Source 4** ("the trainer's own tenant, for an account whose role is
  Trainer") does not apply — the actor here is a Player or a Coach, not the
  Trainer.
- **Source 5** is scoped explicitly to "the unauthenticated Epic-08 route"
  and states it is "the only unauthenticated tenant resolution in the
  platform" (line 158) — `identity_sharelink_join_register` and
  `identity_sharelink_invite_register` are unauthenticated too, and are not
  it.

`AccountTrainerLink` itself is global (no RLS policy — "Global tables carry
no policy," line 269), so writing **that** row needs no tenant context. But
`PlayerTrainerMembership` and `CoachMembership` **are** trainer-scoped
(architect-architecture.md's Entity population table, lines 326–328), and an
`INSERT` under RLS's `WITH CHECK` needs the session variable set to that
trainer *before* the write — which nothing in the resolution list grants the
newly-registering Player or Coach actor.

This is not invented here — it is recorded as a gap, per the hard
instruction not to settle what the specs leave open. A structurally
plausible fix (a narrow, code-authorized scope opened only for the
ShareLink's own trainer, only for the duration of `MembershipService`'s one
call — parallel in shape to `AdministrativeScope` but triggered by
`MembershipService` itself rather than reserved to Super Admin) is
suggested in `## Open questions`, not decided here.

**The one case that does not hit this gap**: camp-registration conversion
(`forms_public_convert_account`) stays entirely inside the code-resolved
Forms flow — the account and the membership are created in the same request
that is still running under source 5's tenant resolution, for the *same*
trainer the form belongs to. The gap resurfaces the moment a registrant
instead uses the **emailed ShareLink to register later** (AC-08-25) — at
that point they are back to the general ShareLink-acceptance case above,
now authenticated for the first time on that later request.

---

## Impersonation and administrative tenant scope

Two mechanisms, both named in `architect-architecture.md`'s "Cross-cutting
services" table and "Writing across tenants" section, with different HTTP
shapes and a different footprint in the audit log.

### Impersonation — a whole session, one identity swap

Recommended implementation vehicle: Symfony's native `switch_user` firewall
feature, not a bespoke session-swapping mechanism — it already provides
exactly this shape (the acting token becomes the target's, `getUser()`
returns the target, and the *original* authenticated token remains
recoverable via the `ROLE_PREVIOUS_ADMIN` role / `SwitchUserToken::
getOriginalToken()`), which is precisely BR-01-21/22's contract. The
`administration_impersonation_start` controller does its own authorization
first (`ImpersonationVoter::IMPERSONATION_START`, which denies a Super-Admin
target per BR-01-21/AC-01-37 — a check `switch_user`'s own
`ROLE_ALLOWED_TO_SWITCH` gate cannot express on its own) and writes the
`ImpersonationSession` row (who, whom, start time), then transitions the
token.

Once active:

- `getUser()` returns the **target**, so `/trainer/...`, `/coach/...`,
  `/portal/...` all become reachable exactly as that user would see them
  (AC-01-34) — and `/super-admin/...` becomes **un**reachable, since the
  effective roles are the target's, not `ROLE_SUPER_ADMIN`, and no role
  hierarchy is configured.
- Every page's layout renders the persistent "Viewing as [Name] | Exit
  Impersonation" banner from the presence of `ROLE_PREVIOUS_ADMIN` — not
  from a bespoke flag, so it cannot drift out of sync with the actual
  security token.
- `impersonation_exit` (`POST /impersonation/exit`, gated on the
  `IS_IMPERSONATOR` attribute rather than any specific role, so it is
  reachable regardless of which prefix the Super Admin is currently browsing
  under) restores the original token and writes the session's end time and
  duration.
- **1-hour expiry is enforced at read time, not by a job** — consistent with
  "time-based state is derived at read time" (architect-architecture.md line
  572): every request checks `now() - ImpersonationSession.startedAt`
  against 1 hour and force-exits if exceeded, rather than a scheduled task
  flipping a status.

### Administrative tenant scope — per-request, no identity change

Opened by the two named callers only — Event Master's per-event write
routes and the per-trainer fee edit — after the relevant voter already
allowed the action, and closed at the end of that same request
(`kernel.terminate`, or an explicit `finally`). The Super Admin's own
identity and roles never change; `TenantContext` alone gets a trainer set
for the request's duration, the same `SET` mechanism any other resolution
source uses (architect-architecture.md line 271). Unlike impersonation there
is no banner, no exit action, and no session-level state — entering and
leaving happen inside one HTTP request/response cycle, invisibly to the URL
bar.

### Distinguishing an impersonated request in the audit log

`AuditLogger`'s write call must be able to carry **two** identities, not
one, whenever either mechanism is active:

| Field | Impersonation | Administrative scope | Neither |
|---|---|---|---|
| `actorAccountId` | The **true**, originally-authenticated Super Admin — recovered from `SwitchUserToken::getOriginalToken()`, never from `getUser()` | The Super Admin performing the write | The authenticated actor |
| `actingAsAccountId` (nullable) | The impersonated target | `null` — no identity is assumed, only a tenant | `null` |
| `impersonationSessionId` (nullable FK) | The active `ImpersonationSession` | `null` | `null` |
| `administrativeScopeTrainerId` (nullable) | `null` | The adopted trainer | `null` |

This is an HTTP/service-contract requirement on every controller and service
that calls `AuditLogger`, not a schema (columns, types and indexes are
`database-designer`'s stage) — but the contract has to exist at this layer,
because `getUser()` alone cannot answer "who really did this" once
`switch_user` is active, and silently logging the impersonated target as the
actor would make the audit log actively misleading for exactly the
accountability purpose US-07.08 states it exists for.

---

## API Platform recommendation

**Against, for this codebase, as currently scoped.**

1. **The application is not an API.** The architecture chose server-rendered
   Twig with progressive enhancement explicitly ("Everything is
   server-rendered Twig with progressive enhancement. Not a SPA," line 40).
   `Task/app/` confirms this empirically rather than aspirationally: it is a
   bare `symfony/skeleton` checkout — `FrameworkBundle` only, no Doctrine, no
   Security, no Twig, no `api-platform/core` — so there is nothing to
   retrofit and no existing convention this recommendation would be
   overriding.
2. **The genuine JSON surface is six narrow, single-purpose endpoints**, not
   a resource collection: one webhook receiver (which API Platform does not
   model at all — a webhook is not a resource) and five widgets
   (availability count, YouTube metadata lookup, two drag-and-drop
   reorders, coupon validation, mark-content-complete). None filters, sorts,
   paginates, or needs content negotiation, hypermedia, or OpenAPI
   generation — the properties API Platform exists to automate. Every one of
   the six is more legibly a five-line controller action returning
   `JsonResponse` than an `ApiResource` with a custom provider/processor.
3. **Authorization here is voter-based and deep** — five tenancy layers,
   `CoachVisibilityService`-delegated coach reach, `AdministrativeScope`,
   impersonation, `FeatureGate` — and this document, following the
   architecture and the skill, commits to Symfony voters as the single place
   that logic lives. API Platform's `security`/`securityPostDenormalize`
   expression strings are a second, string-based authorization surface that
   would sit next to those voters rather than inside them. For a platform
   whose stated highest-risk defect class is tenant isolation
   (architect-architecture.md line 107), a second place authorization logic
   could be written — and drift from the first — is a cost with no
   offsetting benefit here.
4. **No abstraction here is substitution-justified.** Applying
   `examples/symfony-clean-code-patterns.md`'s own review question — "is
   every abstraction justified by substitution, volatility, ownership, or a
   narrower consumer contract?" — to API Platform's provider/processor
   layer: these six endpoints are never substituted, never consumed by
   anyone but this application's own Stimulus controllers, and owned end to
   end by one team. A new major dependency (which pulls in its own
   Serializer/normalizer pipeline, routing layer, and OpenAPI generation)
   for six single-purpose actions does not clear that bar.

**Reconsider when** — and only when — a genuine third-party or mobile-native
API consumer is actually scoped, not merely plausible in the abstract. Even
then, design it for the narrow resource set such a consumer would actually
want (Events and RSVPs are the likely candidates — a calendar integration),
not the whole domain, and re-run `api-platform-designer` against that slice
specifically, so the security-expression-vs-voter question above gets a
deliberate answer instead of an inherited default.

---

## Traceability

Every route table above cites its serving `AC-NN-n` inline; this section is
the module-level summary the task asks for, plus an honest accounting of the
419 acceptance criteria that are **not** one-to-one with a route — stating
that explicitly rather than implying full route-level coverage where none
was designed.

| Module | Epic | AC range | Route groups |
|---|---|---|---|
| Platform | — | AC-01-15 | 1 (trainer-context switch) |
| Identity | 01 | AC-01-1..67, AC-01-69, AC-01-71..73, AC-01-77 (many served by **Administration**'s routes — see below) | Auth/lifecycle (7), ShareLink landing/registration (7), coach invite/ShareLink mgmt (5), family/child (7), child approval (3), availability (2), profile/branding (2) |
| Scheduling | 02 | AC-02-1..70 | Event Builder (11), coach console (3), player portal (4) |
| Crm | 03 | AC-03-1..70 | Trainer CRM (13), coach CRM (3), Quick View dashboard (1) |
| Content | 04 | AC-04-1..43 | Trainer console (16), player portal (5), Super Admin analytics (2) |
| Billing | 05 | AC-05-1..38 | Trainer settings (7), player portal (6), webhook (1) |
| Growth | 06 | AC-06-1..33 | Player (1), trainer (7), checkout widget (1), Super Admin (1) |
| Administration | 07 (+ cross-epic: AC-01-1..8/33..38/52..59, AC-05-27/28) | AC-07-1..41 | Dashboard (1), Users tool (7), trainer mgmt/features/fees (3), Event Master (5), audit log (2), Stripe links (2) |
| Forms | 08 | AC-08-1..46 | Trainer console (10), public (4) |

### Coverage notes — acceptance criteria not tied to a single route

- **Performance and scale targets** (AC-01-77, AC-02-69, AC-03-7/42/69,
  AC-04-42, AC-05-38, AC-06-33, AC-07-41, AC-08-33/34/39/43..46) are budgets
  the routes above must meet, not endpoints of their own — no route table
  row cites them beyond the one or two placed where a design choice
  (streaming CSV, JSON widgets, pagination default) exists specifically
  because of one.
- **Process/approval gates** (AC-01-78, AC-02-70, AC-03-70, AC-04-43,
  AC-08's own success-criteria framing) describe when an epic is "done," not
  product behavior — not cited on any route, consistent with the epic specs'
  own footnote on each ("Process gate, not product behavior").
- **Structural/negative properties** (AC-01-74 tenancy isolation, AC-01-75
  hashed passwords and unique emails, AC-01-76 audit coverage, AC-07-7 "no
  revenue duplicated") are properties the whole design must hold, stated
  once each in Conventions/the module tables (the tenant-isolation 404 rule,
  the "no money figure on the Administration dashboard" row) rather than
  re-cited per route.
- **Cross-cutting behavioral constraints enforced by voter branches across
  many routes, not one route each**: AC-01-29/30 (what a child login can and
  cannot do) is enforced inside `ChildApprovalVoter`, `PaymentMethodVoter`,
  `RsvpVoter` and `identity_sharelink_join_show`'s own branch, not a
  dedicated "child constraints" endpoint. AC-01-32 (the child's own context
  selector shows only the child's trainers, no parent data) is a rendering
  difference inside `platform_context_trainer_switch`'s view, not a second
  route. AC-01-68/70 (RBAC reaches the correct dashboard; a role cannot
  reach another role's features) is the coarse-gate table itself.
- **Dropped by an owner decision, not an oversight**: AC-05-24 ("Family
  Overview") is deliberately absent — A6 drops it — and is called out by
  name in the Billing module table so its absence reads as intentional.
- **Folded into a Form's field list rather than cited per-field**: AC-01-20
  (a child may optionally have their own login) is carried as an optional
  email/password sub-section on `ChildProfileType`
  (`identity_portal_child_create`/`identity_portal_child_edit`), not a
  separate endpoint — granting login credentials is one more thing that Form
  can set, not a distinct workflow. AC-02-4..6 (private-event visibility and
  invitee selection) are carried inside `EventType`'s visibility and
  invitee-multiselect fields on `scheduling_trainer_event_create`/`_edit`,
  abbreviated out of the Request column alongside the rest of that Form's
  roughly dozen fields for table width, not omitted from the design. AC-01-36
  (every impersonation session and its actions are logged) is satisfied by
  the `AuditLogger` contract in "Impersonation and administrative tenant
  scope" rather than being its own endpoint.

---

## Decisions

| Decision | Chosen | Rejected | Because |
|---|---|---|---|
| Users tool placement | Administration module | Identity module (its own epic states the underlying mechanics) | Epic-07 explicitly owns "Super Admin console"; Identity supplies the account services Administration's controllers call |
| Per-trainer fee edit placement | Administration module | Billing module (its own epic, Epic-05, states the story) | architect-architecture.md names `AdministrativeScope`'s callers explicitly as "Administration (Event Master, fee edits)" — not a placement call this document invented |
| CRM Master / LPPP Analytics / referral-rule config placement | Each stays in its own epic's module, `/super-admin/...`-gated | Consolidating every Super-Admin-facing screen under Administration | The architecture's "owns" column names only the Users tool, Event Master, the dashboard and the audit log viewer under Administration — not these three |
| Event Master list vs. per-event write | List via `CrossTenantReadService`; edit/cancel via `AdministrativeScope` | `AdministrativeScope` for the list too | A list is a read across many tenants at once — exactly what `CrossTenantReadService`'s named callers ("dashboards, trainer lists") describe; a single event's edit is a genuine scoped write |
| `AdministrativeScope` lifetime | Opened and closed within one request/response cycle | A session-long "acting as trainer X" mode with its own banner, mirroring impersonation | The architecture frames it as adopting a tenant "without becoming a user" (line 194) — there is no identity to display a banner for, and no natural exit signal the way impersonation's identity swap has one |
| Impersonation implementation vehicle | Symfony's native `switch_user` firewall feature | A bespoke session-swap flag and a hand-rolled "acting as" check on every route | `switch_user` already provides the exact contract BR-01-21/22 needs — an original token recoverable via `ROLE_PREVIOUS_ADMIN`, and role-based reachability that changes with the identity — for free; reinventing it risks silently missing one of the places that reachability change needs to apply |
| `/invite/{code}` reuse | One route serving both Epic-01's coach invitation and Epic-03's coach-issued player invitation, branching on `ShareLink.type` | A third, distinct path for the coach-issued player invite | Both are stated in the source with the identical URL shape (`/invite/[unique-code]`); Epic-03's own Open questions flags this as unresolved, so a type-agnostic route that does not force a premature answer is the more conservative design |
| Public form submission response shape | HTML form POST, redirect-after-post | JSON, as the brief's candidate list suggested | Resilience on a phone with poor connectivity, a native redirect into Stripe Checkout with no client-side redirect handling to get wrong, and consistency with every other mutation in the app |
| Pagination default | 50/page platform-wide, server-fixed | Per-route bespoke sizes; a client-adjustable `per_page` parameter | Matches three explicit, mutually consistent 50/page statements in the source (AC-03-5, AC-02-56, AC-07-12); a client-adjustable size is an abuse vector no epic asks for |
| Cross-tenant error status | `404`, structurally, before any voter runs | `403` for every authorization denial, tenant-crossing included | RLS and the Doctrine filter make a foreign-tenant entity invisible to the lookup itself — this is a consequence of the tenancy design, not a controller-level style choice, and it hides existence as a side effect |
| Voter set | Reuse the architecture's 11 named voters; add 14 more where a genuinely distinct subject/attribute family exists | Forcing every new decision onto the 11 named voters (e.g. child-approval logic inside `RsvpVoter`) | The architecture's list reads as "at least these," and a voter whose `supports()` straddles unrelated subject types is a defect this document's own conventions section warns against |
| Attendance and coach-assignment confirmation | Two dedicated voters (`AttendanceVoter`, `CoachAssignmentVoter`) | Folding both into `EventVoter` | A Doctrine voter's `supports()` keys off one subject type; `CoachAssignment` and an in-progress attendance take are distinct subjects from `Event` even though both are Event-adjacent |
| Subscription purchase authorization | A `TokenVoter` attribute (`TOKEN_SUBSCRIPTION_PURCHASE`) | A dedicated `SubscriptionVoter` | The architecture treats subscriptions as ledger-adjacent entitlements (line 499), not a separate domain concern; one more attribute on the ledger's own voter matches that framing |
| API Platform | Not adopted | Adopting it for the six JSON endpoints, or for the whole domain | See "API Platform recommendation" above |
| Trainer self-assigned as coach | Reuses the trainer's own `/trainer/events/{event}/attendance` route | A parallel requirement to use `/coach/...` even when the "coach" is the trainer's own account | AC-02-41 already grants trainers any-time attendance access regardless of who is assigned — no separate route earns its keep |
| Child-context switch vs. trainer-context switch | Two separate routes, in two different modules (Identity vs. Platform) | One unified "context" concept and route | Different owning entities (`ParentChildLink` vs. `AccountTrainerLink`) and different tenancy consequences — only the trainer switch touches `TenantContext` at all |

---

## Open questions

Carried from the epic specs' own Open questions where a route design
decision depends on them (cited, not re-litigated), plus what this
route-design pass itself surfaced.

### Inherited from the epic specs, with the route-design consequence noted

- **Q-01.05 (email verification required before login?)** — unresolved in
  `requirements-analyst-epic-01-user-management-spec.md`. Consequence:
  whether `identity_auth_login` blocks an unverified account or only nudges
  it is undecided; the route is designed to support either behind one
  `AccountVoter`-adjacent check without a path change either way.
- **Q-03.05 (can coaches apply flags, or trainers only?)** — unresolved.
  Consequence: `crm_trainer_player_flag_add` is gated for both
  `ROLE_TRAINER` and `ROLE_COACH` at the coarse level; `PlayerVoter`'s coach
  branch is the actual switch once this resolves, so no route change is
  needed either way.
- **Q-06.10 (coupon eligibility: new players only, existing too, or
  per-coupon?)** — unresolved. Consequence: `growth_portal_coupon_validate`
  and the Growth module's `CouponVoter::COUPON_APPLY` are designed to accept
  an eligibility check without yet specifying its rule.
- **AC-08-18's "Pending" filter vs. BR from the same story's "pending
  payments excluded"** — the epic's own contradiction (Epic-08 Open
  questions, analyst-raised). Consequence: `forms_trainer_submissions_index`
  lists `paymentStatus=pending` as a filter value pending that resolution;
  if pending registrations are never shown, that value has nothing to
  filter and should be dropped from the Form's choice list.

### API-designer-raised

- **The ShareLink-acceptance tenant-resolution gap** (full treatment above,
  under "Public unauthenticated routes and tenant resolution"). The
  architecture's five tenant-resolution sources have no source covering the
  moment `MembershipService` writes the first trainer-scoped
  `PlayerTrainerMembership`/`CoachMembership` row for an actor who just
  registered or logged in via a ShareLink. A structurally plausible fix — a
  narrow, code-authorized scope opened only for the ShareLink's own trainer,
  only for the duration of that one `MembershipService` call, parallel in
  shape to `AdministrativeScope` but triggered by `MembershipService` itself
  rather than reserved to Super Admin — is suggested, not decided. This is
  the highest-value open item in this document: it affects the single most
  common user flow in the product (every player and coach registration) and
  is not mentioned anywhere in `architect-architecture.md`'s own Open
  architecture risks.
- **Does a trainer who self-assigns as an event's coach need to explicitly
  confirm the assignment?** BR-02-13 states assignments are "Pending until
  coach confirms" with no stated exception for a trainer assigning
  themselves. No confirm route is designed for this case — only the
  coach-console `scheduling_coach_assignment_confirm`, gated `ROLE_COACH`,
  which a trainer's own account cannot reach. Left open rather than assumed
  either way (auto-confirm, or a parallel trainer-facing confirm action).
- **What happens when a trainer navigates directly to a feature-toggled-off
  URL** (e.g. `/trainer/marketing/coupons` with the Marketing toggle off for
  that trainer)? BR-07-1/3 state the feature "disappears from the trainer's
  UI" (i.e. no nav link) but do not state the behavior of a direct hit on
  the URL itself. This document's existing-hiding convention would suggest
  `404`, consistent with treating a disabled feature like a nonexistent
  resource, but an upsell/"this feature isn't enabled for your account" page
  is equally defensible and not ruled out by any epic text. Not decided
  here.
- **Should every `AdministrativeScope` entry capture a reason, not only the
  fee edit?** AC-05-28 explicitly requires a reason for fee changes; AC-07-28
  requires only that an Event Master override is logged, with no stated
  reason field. Whether this asymmetry is intentional (a fee change needs
  business justification; an emergency schedule override does not) or an
  gap in Epic-07's own acceptance criteria is not addressed by either epic,
  and is not resolved here.

---

*Inputs: `architect-architecture.md` (read in full), Section A of
`requirements-analyst-open-questions.md`, and all eight epic specs' User
scenarios, Acceptance criteria, Business rules and Open questions sections.
`Task/app/` inspected directly (bare Symfony 7.4 skeleton, no bundles beyond
`FrameworkBundle`) to confirm this is a first design, not a retrofit. No
content from `Task/Epics/` or `Task/designs/` was read or modified.*
