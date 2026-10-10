# Design: PracticePerfect HTTP surface (Laravel)

Route names, methods, URIs, middleware, Policy abilities, Form Requests and
response shapes for all eight epics, on top of the now-settled
`specs/architect-architecture.md` (module map, layering, tenancy, the token
ledger, sync/async split, presentation) and
`specs/database-designer-eloquent-schema.md` (table/model names). This
document designs neither of those — it assumes them and adds the one layer
they both deliberately deferred: **the routes themselves.**

**Provenance and a correction.** This task's own brief was written before
`architect-architecture.md` existed in this edition (`specs/MANIFEST.md` still
lists it "deliberately absent" as of this session's start). Both it and
`database-designer-eloquent-schema.md` appeared in `specs/` while this
document was being drafted, superseding this document's own from-scratch
reconstruction of the tenancy mechanism. Where the brief hypothesized a
design and the now-available architecture settles it differently, the
architecture wins and the correction is stated explicitly rather than
silently followed — most consequentially for "which routes are genuinely
JSON" (see "Response shapes" below): the brief expected the public form
submission to be JSON; the settled architecture commits it to plain Blade,
and this document follows the settled architecture.

**A sibling-document inconsistency, noted and resolved one way.**
`architect-architecture.md` and `database-designer-eloquent-schema.md` were
evidently drafted concurrently and disagree in three small places: the
session-variable statement (`set_config('app.current_trainer_id', ?, false)`
vs. a literal `SET app.current_trainer_id = ?`, the latter not a construct
PostgreSQL accepts a bind parameter for), the ledger service's name
(`TokenLedger` vs. `TokenLedgerService`), and the model namespace (per-module
vs. flat `App\Models\*` — the schema document itself defers this exact
question to `architect`, so this is not really a disagreement so much as the
architecture document answering a question the schema document left open).
This document follows `architect-architecture.md` throughout, since it is
the later, more complete, and specifically-scoped-to-this-question source,
and the schema document's own conventions section defers to it by name. Flagged
again in Decisions; not fixed here, since editing either sibling document is
out of this task's scope.

---

## Conventions

### Stack, in one line

Laravel 12, PHP 8.4, PostgreSQL 17, Eloquent, Blade (+ Livewire 3 on a named
short list of authenticated screens), session-guard auth (`web`, Breeze Blade
scaffold), `database` queue driver. No Sanctum, no Passport, no token API —
`architect-architecture.md` "Stack" (line 61) states plainly that no epic
names an API client or a mobile app. Every route below is either a Blade page,
a redirect, or (for exactly one route) a JSON acknowledgement to Stripe.

### Where the code lives

Module directories, not a flat `app/Http/Controllers`, per
`architect-architecture.md` "Where the code lives" (lines 104-134) and its
"Module layout" Decision (line 1041):

```
app/{Module}/Http/Controllers/{Controller}.php
app/{Module}/Http/Requests/{FormRequest}.php
app/{Module}/Policies/{Model}Policy.php
app/{Module}/Actions/{UseCase}.php
app/{Module}/Services/{Name}.php      (only the carried-over named services)
app/{Module}/Models/{Model}.php
app/{Module}/Jobs/{Job}.php
routes/web.php        authenticated + guest session routes
routes/public.php     the nine (this document: ten) code-resolved routes
routes/console.php    Artisan
```

`Module` is one of `Platform`, `Identity`, `Scheduling`, `Crm`, `Content`,
`Billing`, `Growth`, `Administration`, `Forms` — the module map at
`architect-architecture.md` lines 79-90, unchanged here. A controller lives in
the module that owns the model it primarily touches; where a route's work
spans modules (an RSVP touching `Scheduling` and `Billing`), the controller
lives with the route's own primary noun and calls the other module's named
service or Action, never its Eloquent model directly — the same boundary rule
`architect-architecture.md` states for `MembershipService`/`TokenLedger`/
`PublicTenantCodeRegistry` (lines 100-103).

Laravel's own policy auto-discovery resolves `App\Identity\Models\ShareLink`
to `App\Identity\Policies\ShareLinkPolicy` without any manual
`Policy::class => Model::class` registration — it substitutes `Policies` for
`Models` in the FQCN — so the module layout costs nothing in configuration.

### The route tables' columns

| Column | Meaning |
|---|---|
| Route name | Laravel route name, dot-namespaced by module |
| Method & URI | exactly as registered |
| Middleware | beyond the blanket `web`/`ResolveTenant` stack described below — only what a route adds |
| Policy : Model | the ability and the model instance (or model class, for `create`/`viewAny`) it is checked against |
| Form Request | validation + `authorize()`; "—" where a GET needs none; "Livewire" where the interaction is a component method, not a page submission |
| Response | see "Response shapes" below for the vocabulary |
| Serves | AC-NN-n / BR-NN-n |

Every authenticated, mutating route runs a Policy check. **Every** route,
including the public ones — resolving a tenant is not the same question as
authorizing an action, and the public routes each carry their own Policy call
on the specific row the code resolved (`ShareLinkPolicy::resolve`,
`FormPolicy::resolve`, `FormSubmissionPolicy::create`) exactly because
`architect-architecture.md` states this as a first-class rule, not a
footnote: "resolution is not authorization" (line 393, itself carried from
`council-sharelink-tenant-resolution.md:409-412`).

### Tenancy: how a route gets a resolved trainer, restated for routing purposes

Full mechanism in `architect-architecture.md` "Tenancy enforcement" (lines
184-635); restated here only as far as a route author needs it.

`ResolveTenant` middleware runs on every HTTP route except the Stripe webhook
(its own case, see "Webhook contract"). It never throws by itself — it
resolves a trainer id or it resolves nothing, and nothing is a perfectly
normal outcome for a Super Admin who is not impersonating. The **throw**
happens lazily, the first time some later code asks
`TenantContext::trainerId()` for a value that was never resolved — inside the
`BelongsToTenant` trait's `creating`/`retrieved` hooks, or an explicit call.
A route whose whole job is a cross-tenant read (`Administration`'s Users tool,
Event Master, CRM Master, Audit Log, LPPP Analytics) never asks, because it
queries through `CrossTenantReadService` or a global-table model
(`Account`, `Trainer`, `FeatureToggle`, `AuditLogEntry`) instead — so those
routes need no exemption from `ResolveTenant`; they simply never trigger the
part of it that can fail.

Five resolution sources, precedence and numbering exactly as
`architect-architecture.md` "Resolution sources, in order" (lines 397-442):

| # | Source | Outranks | Cited |
|---|---|---|---|
| 1 | Super Admin, not impersonating → **no tenant**. Reads go through `CrossTenantReadService` | — | line 403 |
| 2 | Active impersonation session → the impersonated account's tenant | everything | line 406 |
| 3 | Selected trainer context — session-held, or supplied by a request on a route whose purpose is to change or target it — validated against `account_trainer_links` | 4, unless overridden by 5 | line 410 |
| 4 | A Trainer or Coach account with exactly one membership → that trainer, no switcher | — | line 419 |
| 5 | A public code in the route, on an allow-listed route, looked up in `public_tenant_codes` | 3 and 4, **only on allow-listed routes** | line 422 |

This document's route tables mark which source resolves each authenticated
route only where it is not source 3-then-4 (the default for everything under
`/trainer`, `/coach`, and the unprefixed player routes) — i.e. only the
public group (source 5) and the administrative-scope routes (below).

**Two extensions this document adds, flagged as extensions, not settled
architecture:**

- **A tenth public route, `public.referral.show`.** US-06.01's referral link
  (`platform.com/join/{trainer-slug}/{player-id}`,
  `requirements-analyst-epic-06-marketing-growth-spec.md` line 112) resolves
  its tenant from `trainers.slug` directly — a global, RLS-free column
  indexed for exactly this ("public form/referral link resolution, the one
  unauthenticated tenant-resolution path",
  `database-designer-schema.md:334-336`) — not from `public_tenant_codes`,
  since no `share_link`/`form` row backs a referral link at all
  (`database-designer-schema.md:1643-1646`, "no separate stored code column
  is needed"). `architect-architecture.md`'s nine-route `routes/public.php`
  snippet (lines 476-485) does not include it. This document adds it as a
  tenth route in the same file, under the same middleware, resolved by a new
  `SlugTenantResolver` sitting beside `PublicCodeTenantResolver` rather than
  inside it, because the lookup target differs (`trainers.slug`, not
  `public_tenant_codes.code`) even though the shape — anonymous, route-scoped,
  audited the same way, rate-limited the same way — is identical. See
  Open questions.
- **`AdministrativeScope`, given a concrete shape.** The module map lists it
  as something `Administration` owns (`architect-architecture.md` line 88)
  but its mechanics are not defined in any input this document has read —
  the "Resolution sources" list stops at five, and "No role hierarchy"
  states only that Super Admin's two sanctioned tenant-touching mechanisms
  are impersonation and `CrossTenantReadService`, of which the second is
  `SELECT`-only (line 669). Neither covers a Super Admin editing one named
  trainer's pricing or overriding one named trainer's event without a full
  "viewing as" impersonation session. This document gives `AdministrativeScope`
  a concrete, narrow shape: a service, not a middleware, called explicitly
  from an `Administration` controller, wrapping `TenantContext::runFor()`
  (`architect-architecture.md` lines 264-271; stated there for jobs and
  commands, extended here to one more caller) around a call into the
  *owning* module's own Action — so `Administration` still never writes
  another module's model directly, satisfying its own "Must not" column
  (line 88) — behind a Super-Admin-only Policy check and an unconditional
  `AuditLogger` entry. Used by exactly the routes marked "AdministrativeScope"
  below: Event Master's edit/cancel/conflict-override, and per-trainer
  pricing. Flagged in Open questions as needing `architect`'s own ratification,
  since it is this document's synthesis, not a quotation.

```php
final class AdministrativeScope
{
    public function run(Trainer $trainer, \Closure $callback): mixed
    {
        // Gate::authorize() already ran in the controller/Form Request;
        // this is the tenancy half, not the authorization half.
        return app(TenantContext::class)->runFor(
            $trainer->id,
            function () use ($trainer, $callback) {
                $result = $callback();
                app(AuditLogger::class)->record(
                    actionType: 'administrative_scope_write',
                    relatedTrainer: $trainer,
                );
                return $result;
            },
        );
    }
}
```

### Impersonation, and staying attributable to the human who started it

`architect-architecture.md`'s "Impersonation" Decision (line 1035) settles
*that* it is first-party, session-key-based, and re-resolves a tenant via
source 2. It does not spell out how a write performed mid-impersonation still
attributes to the admin rather than the puppet, which
BR-01-22/AC-01-36 require ("all actions taken during the session are logged
with the admin's ID context",
`requirements-analyst-epic-01-user-management-spec.md` line 288). This
document's completion of that detail:

- `POST /admin/users/{account}/impersonate` (`Identity\Policies\AccountPolicy::impersonate`
  — Super Admin only, target `role !== super_admin`, AC-01-37) writes an
  `impersonation_sessions` row, then stores `session(['impersonation.session_id'
  => $session->id, 'impersonation.admin_account_id' => Auth::id(),
  'impersonation.started_at' => now()])` **before** calling
  `Auth::loginUsingId($target->id)`. Laravel's `login()` regenerates the
  session id for fixation-safety; it does not clear arbitrary session keys,
  so the three stashed values survive the switch.
- `Auth::user()` now returns the **target** for every ordinary read/write —
  the whole app behaves as that user, which is the point (AC-01-34: "all
  navigation, permissions, and data exactly match the impersonated user").
- `AuditLogger` (Platform-owned, sole writer of `audit_log_entries` per
  `architect-architecture.md` line 87) exposes `actorAccountId(): int`,
  resolving `session('impersonation.admin_account_id') ?? Auth::id()`. Every
  write path that populates `audit_log_entries.actor_account_id` calls this,
  never `Auth::id()` directly — an arch test (mirroring the ones already
  named for `withoutGlobalScope` and `Gate::before`) forbids
  `Auth::id()`/`Auth::user()->id` inside `AuditLogger::record()`'s callers
  from being used as the actor argument without going through this method.
- `POST /admin/impersonation/exit` reverses it: `Auth::loginUsingId($admin)`,
  `impersonation_sessions.ended_at/ended_reason='manual'`, session keys
  cleared.
- The 1-hour expiry (BR-01-22, AC-01-38) is evaluated at read time, not by a
  job, matching `architect-architecture.md` "Time-based state is derived at
  read time" (lines 752-767): a small `EnforceImpersonationExpiry` middleware
  checks `session('impersonation.started_at')` on every request while
  impersonating and force-exits (`ended_reason='expired'`) past one hour.

---

### Authorization: Policies, and the rule that never bends

Per `architect-architecture.md` "No role hierarchy" (lines 640-671) — **no
`Gate::before` Super-Admin bypass, anywhere, ever.** Every Super Admin
capability in the tables below is an explicit ability on an explicit Policy
method that checks `$user->role === Role::SuperAdmin` itself, alongside the
ordinary ownership check. An ownership check compares against
`TenantContext::trainerId()`, never against a column on the `$user`
model — the resolved tenant is the authority a Policy consults, exactly
because a Super Admin holding no tenant must fail an ownership check that
never even reaches the role branch, not skip it.

Public/guest abilities are typed `?Account $user` — `ShareLinkPolicy::resolve(?Account $user, ShareLink $link)`
and `FormSubmissionPolicy::create(?Account $user, Form $form)` are the two
Laravel invokes for an unauthenticated actor (`architect-architecture.md`
line 682).

Non-obvious authorization rules (full table at `architect-architecture.md`
lines 692-703) are called out per route below rather than restated as a
block.

### Form Requests, and what they must never accept

Every mutating route's Form Request validates field shape only. A business
rule the Action already enforces (capacity, eligibility, coach conflict) is
never re-validated in the Form Request — `architect-architecture.md`
"Layering" table, "Form Request... Never: Re-state a business rule the
Action owns" (line 150). `authorize()` delegates to the Policy; it is never
a second, looser copy of it.

**No Form Request accepts a price, a fee, a role, a trainer id, an
ownership/audit column, or a workflow status.** Every amount charged is
recomputed server-side from the current `events`/`playlists`/`token_packages`/
`trainer_billing_settings` row at submission time — this is what the
platform-fee snapshot design (`architect-architecture.md` lines 819-849)
already assumes, and a Form Request that accepted a client-supplied amount
would silently reopen it. Concretely: an RSVP/purchase Form Request's payload
is `{event_id | playlist_id | token_package_id, payment_method}` and nothing
priced; `trainer_id` is never a field anywhere, because no in-app route ever
carries `{trainer}` in its own path (see next section).

### No `{trainer}` in the URI, except where the URI's whole purpose is targeting one

Every ordinary authenticated route resolves its trainer from context (sources
3/4 above), never from a URL segment — a request cannot name a different
trainer than the one it is already scoped to, which is what makes "no
`{trainer}` in the route" a real security property and not just a style
choice. The only routes carrying an explicit trainer identifier are:

- the public/code-resolved group (`{code}`, `{trainerSlug}` — the identifier
  *is* the resolution mechanism, source 5);
- `Administration`'s single-trainer admin routes
  (`/admin/trainers/{trainer}/...`), where the named trainer becomes the
  tenant via `AdministrativeScope`, Super-Admin-gated and audited.

### Two context selectors

Both are `Identity`-owned, session-writing, and distinct from tenant
resolution's own sources — they are what *populates* source 3, and a
separate, family-scoped selector the epics also require:

| Route name | Method & URI | Form Request | Response | Serves |
|---|---|---|---|---|
| `identity.context.trainer` | `PATCH /context/trainer` | `SwitchTrainerContextRequest` — `trainer_id`, validated against the account's own active `account_trainer_links` rows | Redirect back + flash | AC-01-15, AC-01-32 |
| `identity.context.player` | `PATCH /context/player` | `SwitchPlayerContextRequest` — `player_id`, validated as "self" or one of the parent's own `parent_child_links` children, within the current trainer context | Redirect back + flash | AC-01-18, AC-01-44, AC-04-26 |

### Response shapes: HTML-first, and where "genuinely JSON" actually lands

This is the section the task brief's own hypothesis needs correcting most,
now that `architect-architecture.md`'s "Presentation" (lines 897-923) is
available:

- **The Stripe webhook is the only bespoke JSON route in this application.**
  It is server-to-server; there is no browser, no session, no CSRF token to
  supply, and nothing to render.
- **"Progressive-enhancement endpoints" are not a separate route category —
  they are Livewire, on a named short list of authenticated screens**: the
  CRM segmentation/player-list panel (BR-03-13), the event builder, the
  playlist/drill builder, and the Super Admin dashboards
  (`architect-architecture.md` lines 913-915). Livewire's reactivity POSTs to
  its own framework-managed `/livewire/update` endpoint, which this document
  does not register, name, or list a row for — it is automatic, and
  `architect-architecture.md` is explicit that it must **never** appear on
  the code-resolved allow-list (line 506, restated at line 900-907: the
  endpoint carries none of the original route's parameters, so it cannot be
  code-resolved, and would silently resolve from session context instead —
  correct on an authenticated trainer screen, wrong or absent on `/f/{code}`).
  Drag-and-drop reorder (playlist/drill ordering, AC-04-2), live
  player-search-as-you-type (AC-03-8), and the live "available players at
  this time" count (AC-02-12) are therefore Livewire component interactions
  on their host page's existing GET route, not additional named routes in
  the tables below — each is footnoted "Livewire" in its Form Request column,
  meaning: validated by the component itself
  (`#[Validate]`/`$this->validate()`), authorized by the same Policy call a
  Form Request would have made (`$this->authorize(...)` inside the component
  method), and ultimately calling the identical Action class a full-page
  submission would have called. A no-JS visitor on one of these four screens
  sees a plain, functional Blade form without the live reactivity — Livewire
  degrades to a normal POST-and-reload when its JS runtime is absent, which
  is why permitting it here does not conflict with "must work with
  JavaScript disabled" for the screens it is actually forbidden on (next
  point) and is an acceptable, explicit trade-off for the four screens it is
  permitted on (staff tools, not the public/sign-in surface the constraint
  is written for).
- **The public form submission is plain Blade, not JSON.** Epic-08's own UX
  assumptions — one page, not multi-step, sub-five-minute completion
  (`epic-08:259-260`) — make this free, and Livewire is explicitly forbidden
  on this route for the reason above. `public.form.submit` is a native
  `<form method="POST">`; a free submission redirects to
  `public.form.confirmation`; a paid one redirects to a server-created Stripe
  Checkout Session URL (`redirect()->away(...)`) — a 302 either way, which
  works identically with or without JavaScript. **This corrects the task
  brief's hypothesis** that this route would be genuinely JSON; the settled
  architecture commits it to HTML, and this document follows that.
- **Sign-in, and the whole public/code-resolved group, are plain Blade
  forms**, per the same constraint stated for camps: no Livewire, native
  `<form>` POSTs, session + CSRF, working with JavaScript disabled.
- **Everywhere else** (the overwhelming majority of routes below): a GET
  returns a Blade view; a mutating verb returns a redirect with a flashed
  message, per the next section.

### Error, redirect and flash conventions

- **Validation failure**, classic form POST: Laravel's own default —
  `back()->withErrors($validator)->withInput()`; Blade reads `$errors`/
  `@error('field')`, fields repopulate from `old()`. No custom envelope.
- **Validation failure**, a Livewire component: the component's own error
  bag, rendered inline — never a full-page redirect, since there was no
  full-page submission to redirect from.
- **Authorization failure** (`AuthorizationException` from a denied Policy
  ability): Laravel's default 403 page/response. Two distinct 403 shapes are
  used deliberately, not interchangeably:
  - **Existence-hiding** (`404`, not `403`) for an unknown or invalid
    public code — indistinguishable from any unknown URL, per
    `architect-architecture.md`'s failure-modes table (line 516) — because
    the tenant never resolved and the trainer-scoped row was never even
    queried.
  - **Explicit "Access Denied"** (`403`, with that exact copy) for a
    non-invited player hitting a private event's direct link — BR-02-6
    (`epic-02:317`) states this precisely: existence is *not* hidden, access
    is refused. The distinction matters: a private event's existence is not
    a secret from an ineligible player who somehow has the URL; a stranger's
    ShareLink code's validity is.
  - A **resolved-but-stale** code (expired/exhausted ShareLink, disabled
    camp) is neither of the above: the tenant resolved, the row loaded, and
    the Policy denied it with a specific, human message ("this invitation has
    expired", "Registration Closed") plus, for ShareLinks, the resend path
    (AC-01-42) — never a bare 403 or 404.
- **Flash keys**, used consistently across controllers: `status` (neutral
  confirmation, e.g. "Event created successfully" — AC-02-2 — or "Playlist
  updated!" — AC-04-31), `success`, `error`. Copy matches the AC's own
  quoted wording wherever an AC quotes one.
- **Not found** (`ModelNotFoundException`, ordinary route-model-binding miss
  on an authenticated route): Laravel's default 404.

### CSRF placement

The `web` middleware group (Laravel's default `VerifyCsrfToken`) covers
every route in `routes/web.php` and `routes/public.php` — public does not
mean CSRF-exempt; a guest session still gets a token, and every native
`<form>` on every public page carries `@csrf`. **The one exception is the
Stripe webhook**, excluded by URI in `bootstrap/app.php`:
`->withMiddleware(fn ($m) => $m->validateCsrfTokens(except: ['stripe/webhook']))`
— it cannot supply a token and is verified by its Stripe signature instead
(see "Webhook contract").

### Pagination defaults

**50 per page, everywhere a list paginates**, matching every number an AC
actually states rather than leaving it to vary by screen: AC-01-72 (Users
tool — no story gives a number, epic-level items elsewhere in the same file
do), AC-02-56 (Event Master, 50/page), AC-03-5 (player list, 50/page),
AC-03-33 (player detail's event history, 50/page), AC-07-12 (Users tool,
50/page), AC-07-26 (Event Master, 50/page). No AC states a different number
anywhere in the eight epics, so 50 is adopted platform-wide rather than
re-litigated per screen.

### Rate limits

BR-01-6 (`epic-01:272`) requires login throttling and names no threshold;
`architect-architecture.md` Open question 4 (line 1096) states the same gap
for the public-code limiter and leaves both unresolved. This document
proposes concrete numbers, flagged as proposals, not settled figures, so the
routes below have something enforceable rather than a citation to an
unresolved question:

| Named limiter | Proposed limit | Key | Used by |
|---|---|---|---|
| `login` | 5 / minute | `email` + IP | `platform.session.store` |
| `public-code` | 20 / minute | IP | the whole `routes/public.php` group (already named in `architect-architecture.md`'s own snippet, line 473 — this document supplies the number it left unstated) |
| `payment` | 10 / minute | account id, or IP if guest | every route that creates a Stripe Checkout Session or spends tokens (AC-05-36: "payment attempts are rate-limited to prevent brute force") |

---

## Route map

### Platform — session lifecycle

`Identity`-owned in the module map's terms (`Account` belongs to `Identity`,
not `Platform`), grouped here because it is the one surface every role
shares before any tenant exists.

| Route name | Method & URI | Middleware | Policy : Model | Form Request | Response | Serves |
|---|---|---|---|---|---|---|
| `platform.session.create` | `GET /login` | `guest` | — | — | Blade view | AC-01-65 |
| `platform.session.store` | `POST /login` | `guest`, `throttle:login` | — | `LoginRequest` (email, password) | Redirect to the role's dashboard (BR-01-8) / back + errors | AC-01-65, BR-01-1/2/3/6 |
| `platform.session.destroy` | `POST /logout` | `auth` | — | — | Redirect to `/login` | AC-01-69 |
| `platform.password.request` | `GET /forgot-password` | `guest` | — | — | Blade view | BR-01-4 |
| `platform.password.email` | `POST /forgot-password` | `guest`, `throttle:login` | — | `RequestPasswordResetRequest` | Redirect back + status (always the same message, whether or not the email exists) | BR-01-4 |
| `platform.password.reset` | `GET /reset-password/{token}` | `guest` | — | — | Blade view | BR-01-4 |
| `platform.password.update` | `POST /reset-password` | `guest` | — | `ResetPasswordRequest` — validated against `password_reset_tokens.token_hash`, 1h expiry | Redirect to `/login` + status | AC-01-66, BR-01-4 |
| `platform.email-verification.notice` | `GET /email/verify` | `auth` | — | — | Blade view | BR-01-5 |
| `platform.email-verification.send` | `POST /email/verification-notification` | `auth`, `throttle:login` | — | — | Redirect back + status | AC-01-67 |
| `platform.email-verification.confirm` | `GET /email/verify/{token}` | `auth` | — | — (consumes `email_verification_tokens.token_hash`, 24h expiry, sets `accounts.email_verified_at`) | Redirect to dashboard + status | AC-01-67, BR-01-5 |

There is no generic `POST /register`. BR-01-13 ("Only Super Admin can create
trainer accounts — no self-registration",
`requirements-analyst-epic-01-user-management-spec.md` line 279) closes the
only path that would otherwise justify one; every other account is created
either by Super Admin (`identity.trainers.store`, below), by a ShareLink
route (`public.join.register`, `public.invite.register`), or by camp
conversion (`public.form.convert`). Laravel's `verified` middleware — which
checks only `email_verified_at !== null` — is used as-is on every route that
needs it; only the mechanism that *sets* that column (the token table above,
not Breeze's stock signed-URL flow) is custom, matching the schema's own
`email_verification_tokens` design over Laravel's default. See Decisions.

---

### Identity (Epic-01)

#### Trainer creation

| Route name | Method & URI | Middleware | Policy : Model | Form Request | Response | Serves |
|---|---|---|---|---|---|---|
| `identity.trainers.create` | `GET /admin/trainers/create` | `auth`, `role:super_admin` | `TrainerPolicy::create : Trainer` | — | Blade view | AC-01-1 |
| `identity.trainers.store` | `POST /admin/trainers` | `auth`, `role:super_admin` | `TrainerPolicy::create : Trainer` | `StoreTrainerRequest` — business_name, trainer_name, email, phone, subscription tier (AC-07-16) | Redirect to `administration.trainers.show` + status | AC-01-1..8, AC-07-16/17, BR-01-13 |

Saving creates the `trainers` row, the seeded `feature_toggles` rows (all
enabled, BR-07-2), the trainer's static player `share_links` row plus its
`public_tenant_codes` mapping (so `public.join.show` works the instant the
trainer exists), a `platform_subscriptions` row, and queues the invitation
mail — five effects, one `DB::transaction()`, one Action
(`Identity\Actions\CreateTrainer`).

#### ShareLink issuance (trainer, coach)

| Route name | Method & URI | Middleware | Policy : Model | Form Request | Response | Serves |
|---|---|---|---|---|---|---|
| `identity.share-links.invite-players.show` | `GET /trainer/players/invite` | `auth`, `role:trainer` | `ShareLinkPolicy::viewAny : ShareLink` | — | Blade view (static link + optional unique links) | AC-03-59, AC-01-73 |
| `identity.share-links.store` | `POST /trainer/share-links` | `auth`, `role:trainer` | `ShareLinkPolicy::create : ShareLink` | `StoreShareLinkRequest` — optional unique per-player link (AC-03-61) | Redirect back + status | AC-03-61 |
| `identity.coach-invitations.create` | `GET /trainer/coaches/invite` | `auth`, `role:trainer` | `ShareLinkPolicy::create : ShareLink` | — | Blade view | AC-01-39 |
| `identity.coach-invitations.store` | `POST /trainer/coaches/invitations` | `auth`, `role:trainer`, `throttle:public-code` | `ShareLinkPolicy::create : ShareLink` | `InviteCoachRequest` — email required, optional name/message; issues `share_links(link_type='unique_coach')` + `public_tenant_codes` in one transaction | Redirect back + status | AC-01-39, BR-01-15 |
| `identity.coach-invitations.resend` | `POST /trainer/coaches/invitations/{shareLink}/resend` | `auth`, `role:trainer` | `ShareLinkPolicy::update : ShareLink` | — | Redirect back + status | AC-01-42 |
| `identity.coaches.index` | `GET /trainer/coaches` | `auth`, `role:trainer` | `CoachMembershipPolicy::viewAny : CoachMembership` | — | Blade view, 50/page (Pending/Accepted/Expired) | AC-01-40 |
| `crm.coach-invitations.create` | `GET /coach/players/invite` | `auth`, `role:coach` | `ShareLinkPolicy::create : ShareLink` | — | Blade view | AC-03-50 |
| `crm.coach-invitations.store` | `POST /coach/players/invitations` | `auth`, `role:coach`, `throttle:public-code` | `ShareLinkPolicy::create : ShareLink` | `InviteCoachIssuedPlayerLinkRequest` | Redirect back + status | AC-03-50/51/52 |

The coach-issued player invite (`crm.coach-invitations.store`) and the
trainer's static/unique player links both surface at `/join/{code}`; both
land on the same `public.join.show`/`register` pair below, differentiated
only by which `share_links` row the code resolves to — see the note on
`share_links.link_type` in Open questions for the schema-level gap this
glosses over (the enum only names `static_player`/`unique_coach`; neither
value cleanly fits a coach-issued player link, and this document does not
resolve which one it should reuse).

#### Public ShareLink and referral acceptance — the code-resolved group

`routes/public.php`, verbatim from `architect-architecture.md` lines 471-485
for the first nine, plus this document's tenth (see "Tenancy" above for why).
All ten share: `web` middleware (session + CSRF, per "CSRF placement"),
`throttle:public-code`, and `defaults('tenant_source', 'public_code')` for
the nine that resolve via `public_tenant_codes` (the tenth uses a distinct
`defaults('tenant_source', 'trainer_slug')`, called out where it differs).

| Route name | Method & URI | Policy : Model | Form Request | Response | Serves |
|---|---|---|---|---|---|
| `public.join.show` | `GET /join/{code}` | `ShareLinkPolicy::resolve : ShareLink` (nullable `$user`) | — | Blade view — anonymous: registration prompt; authenticated: instant-associate (single trainer) or the family-selection prompt "Who will train with [Trainer]?" (multi-child, AC-01-14) | AC-01-9..15 |
| `public.join.register` | `POST /join/{code}/register` | `ShareLinkPolicy::resolve : ShareLink` (nullable) | `RegisterViaShareLinkRequest` — name, email, password, parent phone, player name/age/gender | Redirect to trainer dashboard + status; confirmation mail queued | AC-01-10..12 |
| `public.join.associate` | `POST /join/{code}/associate` | `ShareLinkPolicy::resolve : ShareLink`, `PlayerTrainerMembershipPolicy::associate : PlayerTrainerMembership` — `auth`, `role:player` | `AssociateViaShareLinkRequest` — `member_ids[]` from the account's own "me" + children only | Redirect to trainer's events + status | AC-01-13/14/15 |
| `public.invite.show` | `GET /invite/{code}` | `ShareLinkPolicy::resolve : ShareLink` (nullable) | — | Blade view | AC-01-40, AC-03-50 |
| `public.invite.register` | `POST /invite/{code}/register` | `ShareLinkPolicy::resolve : ShareLink`, `CoachMembershipPolicy::accept : CoachMembership` | `RegisterViaCoachInviteRequest` | Redirect to coach dashboard + status | AC-01-40/41, BR-01-11 |
| `public.referral.show` (this document's addition) | `GET /join/{trainerSlug}/{playerId}` | `ShareLinkPolicy::resolve : ShareLink` (nullable — reuses the join landing once the tenant resolves) | — | Records `referrals.clicked_at` (30-day cookie, BR-06-1/2), then behaves exactly like `public.join.show` for its resolved tenant | AC-06-1..7 |
| `public.form.show` | `GET /f/{code}` | `FormPolicy::resolve : Form` (nullable) | — | Blade view — name, dates, description, price, spots remaining; "Registration Closed" if disabled; "Camp Full" if at capacity | AC-08-13/15 |
| `public.form.submit` | `POST /f/{code}` | `FormPolicy::resolve : Form`, `FormSubmissionPolicy::create : Form` (nullable — capacity/duplicate-email enforced under a row lock in the Action, not the Policy) | `SubmitPublicFormRequest` — the form's own `field_definitions`-driven shape, plus honeypot (risk 16, `architect-architecture.md` line 1069) | Free: redirect to `public.form.confirmation` + status. Paid: `redirect()->away($stripeCheckoutUrl)` | AC-08-14..16 |
| `public.form.confirmation` | `GET /f/{code}/confirmation` | `FormSubmissionPolicy::view : FormSubmission` | — | Blade view — reads the submission id from a session flash set by `submit`, never from the URL or query string (avoids a strangers-can-guess-another-registrant's-submission IDOR — see Decisions) | AC-08-16 |
| `public.form.convert` | `GET, POST /f/{code}/convert` | `FormSubmissionPolicy::convert : FormSubmission` | `ConvertSubmissionToAccountRequest` — password + terms; pre-filled name/email from the session-flashed submission | Existing email → redirect to `/login` + status, no duplicate account (AC-08-26). New → creates the account via `Identity`'s own creation path (never `Forms` writing an `accounts` row itself — module boundary), auto-assigns the trainer (BR-08-17), redirects to the player dashboard | AC-08-23..26 |

Failure modes for this whole group — 404 on an unknown code before any
trainer-scoped statement runs, resolved-but-stale denied by the Policy with
a specific message, a mapping row surviving a deleted `ShareLink`/`Form`
being benign — are exactly `architect-architecture.md`'s own table (lines
514-522); not restated per row here, only in the public-surface summary
table near the end of this document.

#### Family, child-trainer associations, availability, approvals

| Route name | Method & URI | Middleware | Policy : Model | Form Request | Response | Serves |
|---|---|---|---|---|---|---|
| `identity.family.index` | `GET /family` | `auth`, `role:player` | `ParentChildLinkPolicy::viewAny : ParentChildLink` | — | Blade view — every child, trainer associations, dates | AC-01-22 |
| `identity.children.store` | `POST /family/children` | `auth`, `role:player` | `ParentChildLinkPolicy::create : ParentChildLink` | `StoreChildProfileRequest` — name, age (1-18), gender, optional trainer-association prompt | Redirect to `identity.family.index` + status | AC-01-16/17/21 |
| `identity.children.edit` | `GET /family/children/{playerProfile}/edit` | `auth`, `role:player` | `PlayerProfilePolicy::update : PlayerProfile` | — | Blade view | AC-01-18 |
| `identity.children.update` | `PATCH /family/children/{playerProfile}` | `auth`, `role:player` | `PlayerProfilePolicy::update : PlayerProfile` | `UpdateChildProfileRequest` | Redirect back + status | AC-01-18/19 |
| `identity.children.trainers.add` | `POST /family/children/{playerProfile}/trainers` | `auth`, `role:player` | `PlayerTrainerMembershipPolicy::associate : PlayerTrainerMembership` | `AddChildToTrainerRequest` — the "select from My Trainers" branch only (source 3); the ShareLink-code branch on the same screen submits to `public.join.associate` instead, per `council-sharelink-tenant-resolution.md` line 271's own stated redirect, not a second resolution path | Redirect back + status | AC-01-23 |
| `identity.children.trainers.remove` | `DELETE /family/children/{playerProfile}/trainers/{trainer}` | `auth`, `role:player` | `PlayerTrainerMembershipPolicy::remove : PlayerTrainerMembership` | `RemoveChildFromTrainerRequest` — confirmation flag | Redirect back + status; warns of RSVP cancellation first (AC-01-24) | AC-01-24 |
| `identity.children.token-approval` | `PATCH /family/children/{playerProfile}/token-approval-setting` | `auth`, `role:player` | `ParentChildLinkPolicy::update : ParentChildLink` | `UpdateTokenApprovalSettingRequest` — boolean | Redirect back + status | AC-01-27/28 |
| `identity.availability.edit` | `GET /availability` | `auth` | `AvailabilityWindowPolicy::manage : AvailabilityWindow` | — | Blade view — player/parent weekly grid, or coach's recurring schedule, by role | AC-01-43, AC-01-46 |
| `identity.availability.update` | `PUT /availability` | `auth` | `AvailabilityWindowPolicy::manage : AvailabilityWindow` | `UpdatePlayerAvailabilityRequest` \| `UpdateCoachAvailabilityRequest` (role-selected) | Redirect back + status | AC-01-43/44, AC-01-46 |
| `identity.approvals.index` | `GET /approvals` | `auth`, `role:player` | `ChildApprovalRequestPolicy::viewAny : ChildApprovalRequest` | — | Blade view — pending inbox | AC-01-26 |
| `identity.approvals.approve` | `POST /approvals/{childApprovalRequest}/approve` | `auth`, `role:player` | `ChildApprovalRequestPolicy::respond : ChildApprovalRequest` — the specific `parent_account_id` only | `RespondToApprovalRequest` — optional `parent_note` (BR-01-20) | Free/token: redirect back + status. Payment-requiring (`action_type` needs a card charge, AC-05-13): `redirect()->away($stripeCheckoutUrl)` — the parent, not the child, completes the charge | AC-01-25/26, AC-05-13 |
| `identity.approvals.deny` | `POST /approvals/{childApprovalRequest}/deny` | `auth`, `role:player` | `ChildApprovalRequestPolicy::respond : ChildApprovalRequest` | `RespondToApprovalRequest` | Redirect back + status | AC-01-26 |

`identity.approvals.*` never surfaces an `expired` state to click — a
48-hour-old pending request is expired because its timestamp says so
(`architect-architecture.md` lines 758-760); the scheduled job named in
"Synchronous and asynchronous work" sends the auto-denial *notification*
only, and the inbox query itself excludes rows past the boundary rather than
showing a dead "Approve" button.

#### Profile and branding

| Route name | Method & URI | Middleware | Policy : Model | Form Request | Response | Serves |
|---|---|---|---|---|---|---|
| `identity.profile.edit` | `GET /profile` | `auth` | `AccountPolicy::update : Account` | — | Blade view, role-specific fields (AC-01-51) | AC-01-48 |
| `identity.profile.update` | `PATCH /profile` | `auth` | `AccountPolicy::update : Account` | `UpdateProfileRequest` — email, role, and (for players) skill level stay out of the field set entirely, not merely read-only in the view | Redirect back + status | AC-01-48..51 |
| `identity.branding.edit` | `GET /trainer/branding` | `auth`, `role:trainer` | `TrainerPolicy::updateBranding : Trainer` | — | Blade view, live preview | AC-01-60/61 |
| `identity.branding.update` | `PATCH /trainer/branding` | `auth`, `role:trainer` | `TrainerPolicy::updateBranding : Trainer` | `UpdateBrandingRequest` — logo (png/jpg/svg, ≤2MB), primary_color_hex | Redirect back + status — applies immediately, nothing cached (`architect-architecture.md` "White-label branding", line 857) | AC-01-60..63 |

---

### Scheduling (Epic-02)

#### Event Builder (trainer)

| Route name | Method & URI | Middleware | Policy : Model | Form Request | Response | Serves |
|---|---|---|---|---|---|---|
| `scheduling.events.index` | `GET /trainer/events` | `auth`, `role:trainer` | `EventPolicy::viewAny : Event` | — | Blade view, search/filter | — |
| `scheduling.events.create` | `GET /trainer/events/create` | `auth`, `role:trainer` | `EventPolicy::create : Event` | — | Blade view (Livewire: live availability count, AC-02-12 — degrades to a value recomputed on the validation-redisplay round trip with JS off) | AC-02-1 |
| `scheduling.events.store` | `POST /trainer/events` | `auth`, `role:trainer` | `EventPolicy::create : Event` | `StoreEventRequest` — title (≤100), type, start/end, location, capacity (1-999), optional eligibility/pricing/coach | Redirect to `scheduling.events.show` + "Event created successfully" | AC-02-1..3 |
| `scheduling.events.recurring.create` | `GET /trainer/events/recurring/create` | `auth`, `role:trainer` | `EventPolicy::create : Event` | — | Blade view | AC-02-61 |
| `scheduling.events.recurring.store` | `POST /trainer/events/recurring` | `auth`, `role:trainer` | `EventPolicy::create : Event` | `StoreRecurringEventsRequest` — pattern (day, count), base event fields | Redirect to `scheduling.events.index` + status — N independent `events` rows, no linking entity (`database-designer-schema.md` line 876) | AC-02-61/62 |
| `scheduling.events.show` | `GET /trainer/events/{event}` | `auth`, `role:trainer` | `EventPolicy::view : Event` | — | Blade view | — |
| `scheduling.events.edit` | `GET /trainer/events/{event}/edit` | `auth`, `role:trainer` | `EventPolicy::update : Event` | — | Blade view — not-past events only | AC-02-50 |
| `scheduling.events.update` | `PATCH /trainer/events/{event}` | `auth`, `role:trainer` | `EventPolicy::update : Event` | `UpdateEventRequest` — capacity-below-RSVP-count blocked (AC-02-53), triggers targeted notifications on date/time/location/coach/price change | Redirect back + status | AC-02-50..54 |
| `scheduling.events.duplicate` | `POST /trainer/events/{event}/duplicate` | `auth`, `role:trainer` | `EventPolicy::create : Event` | `DuplicateEventRequest` — new date/time required | Redirect to the new event's edit form + status | AC-02-14..17 |
| `scheduling.events.cancel` | `POST /trainer/events/{event}/cancel` | `auth`, `role:trainer` | `EventPolicy::cancel : Event` | `CancelEventRequest` — reason required | Redirect back + status; refund fan-out queued (BR-02-12) | AC-02-46/47/49 |
| `scheduling.events.invitees.update` | `PUT /trainer/events/{event}/invitees` | `auth`, `role:trainer` | `EventPolicy::update : Event` | `UpdateEventInviteesRequest` — `player_ids[]`, private events only | Redirect back + status | AC-02-4..7 |
| `scheduling.events.coach.assign` | `POST /trainer/events/{event}/coach` | `auth`, `role:trainer` | `CoachAssignmentPolicy::assign : CoachAssignment` | `AssignCoachRequest` — `coach_membership_id`; `override_reason` required only when the assignment conflicts with the coach's stated availability | Redirect back + status | AC-02-8..11 |
| `scheduling.rsvps.index` | `GET /trainer/events/{event}/rsvps` | `auth`, `role:trainer` | `RsvpPolicy::viewAny : Event` | — | Blade view — roster, availability-match badge | AC-02-43/44 |
| `scheduling.rsvps.export` | `GET /trainer/events/{event}/rsvps/export` | `auth`, `role:trainer` | `RsvpPolicy::viewAny : Event` | — | CSV download | AC-02-45 |
| `scheduling.rsvps.store` (trainer adds a player) | `POST /trainer/events/{event}/rsvps` | `auth`, `role:trainer` | `RsvpPolicy::createForOther : Event` | `TrainerAddsPlayerRequest` — `player_id`; capacity checked under the row lock | Redirect back + status; audit-logged (Q-02.09 default) | AC-02-45, Q-02.09 |
| `scheduling.rsvps.destroy` (trainer removes) | `DELETE /trainer/events/{event}/rsvps/{rsvp}` | `auth`, `role:trainer` | `RsvpPolicy::removeForOther : Rsvp` | `RemovePlayerRsvpRequest` | Redirect back + status; refund issued if paid | AC-02-45 |

#### Player-facing calendar and RSVP

| Route name | Method & URI | Middleware | Policy : Model | Form Request | Response | Serves |
|---|---|---|---|---|---|---|
| `scheduling.calendar` | `GET /calendar` | `auth` | `EventPolicy::viewAny : Event` (query-scoped, not a per-row check — a private-event visibility filter applied to the list query, not a Policy call repeated per event) | — | Blade view — month/week/day via plain `?view=` query string, no JS required | AC-02-18..22, AC-02-64/66 |
| `scheduling.events.public-show` | `GET /events/{event}` | `auth` | `EventPolicy::view : Event` — eligibility + private-invite check; non-invited on a private event gets 403 "Access Denied" per BR-02-6, not a 404 | — | Blade view / modal | AC-02-20 |
| `scheduling.rsvps.create` | `POST /events/{event}/rsvp` | `auth`, `throttle:payment` (only when `payment_method=usd`) | `RsvpPolicy::create : Event` | `CreateRsvpRequest` — `payment_method` (`free`\|`token`\|`usd`); no price field | Free/token: redirect to `scheduling.reservations.index` + status. USD: `redirect()->away($stripeCheckoutUrl)`. Child actor: creates a `child_approval_requests` row instead and redirects with "Sent to your parent for approval" | AC-02-23..28, BR-02-10 |
| `scheduling.reservations.index` | `GET /reservations` | `auth` | `RsvpPolicy::viewAny : Rsvp` (own) | — | Blade view | AC-02-24 |
| `scheduling.reservations.cancel` | `POST /reservations/{rsvp}/cancel` | `auth` | `RsvpPolicy::cancel : Rsvp` | `CancelRsvpRequest` — confirmation; <24h shows the no-refund warning first | Redirect back + status; refund per the 24h policy (BR-05-10) | AC-02-29..33 |

#### Coach activities and attendance

| Route name | Method & URI | Middleware | Policy : Model | Form Request | Response | Serves |
|---|---|---|---|---|---|---|
| `scheduling.coach.activities` | `GET /coach/activities` | `auth`, `role:coach` | `CoachAssignmentPolicy::viewAny : CoachAssignment` | — | Blade view — Events to Confirm, Assigned Sessions | AC-02-34 |
| `scheduling.coach.assignments.confirm` | `POST /coach/assignments/{coachAssignment}/confirm` | `auth`, `role:coach` | `CoachAssignmentPolicy::confirm : CoachAssignment` | — | Redirect back + status | AC-02-35 |
| `scheduling.coach.assignments.decline` | `POST /coach/assignments/{coachAssignment}/decline` | `auth`, `role:coach` | `CoachAssignmentPolicy::decline : CoachAssignment` | `DeclineAssignmentRequest` — optional reason | Redirect back + status | AC-02-36 |
| `scheduling.attendance.edit` | `GET /coach/events/{event}/attendance` | `auth`, `role:coach` | `AttendanceRecordPolicy::create : Event` — assigned coach, only after start (BR-02-16) | — | Blade view — roster, Present default | AC-02-38 |
| `scheduling.attendance.update` | `POST /coach/events/{event}/attendance` | `auth`, `role:coach` | `AttendanceRecordPolicy::update : Event` — same-day only for a coach (BR-02-18), a time comparison inside the Policy, not a scheduled lock | `RecordAttendanceRequest` — `{player_id: status}[]` | Redirect back + status | AC-02-38..42 |
| `administration.events.attendance` (trainer/Super Admin, any time) | `GET, POST /trainer/events/{event}/attendance` | `auth`, `role:trainer` | `AttendanceRecordPolicy::update : Event` — no same-day restriction for this role | `RecordAttendanceRequest` (same class) | Blade view / redirect + status | AC-02-41 |

---

### Crm (Epic-03)

| Route name | Method & URI | Middleware | Policy : Model | Form Request | Response | Serves |
|---|---|---|---|---|---|---|
| `crm.players.index` | `GET /trainer/players` | `auth`, `role:trainer` | `PlayerTrainerMembershipPolicy::viewAny : PlayerTrainerMembership` | — | Blade view (Livewire panel for search-as-you-type + AND-combined filters, BR-03-13); no-JS fallback is the same GET with a plain Search button and querystring filters | AC-03-1..10, AC-03-22..26 |
| `crm.players.show` | `GET /trainer/players/{playerProfile}` | `auth`, `role:trainer` | `PlayerTrainerMembershipPolicy::view : PlayerProfile` | — | Blade view — profile, labels, flags, notes, event history, read-only coach feedback | AC-03-27..33 |
| `crm.players.update` | `PATCH /trainer/players/{playerProfile}` | `auth`, `role:trainer` | `PlayerTrainerMembershipPolicy::update : PlayerProfile` | `UpdatePlayerLimitedFieldsRequest` — skill level only | Redirect back + status | AC-03-33 |
| `crm.labels.index` | `GET /trainer/labels` | `auth`, `role:trainer` | `LabelPolicy::viewAny : Label` | — | Blade view | AC-03-11 |
| `crm.labels.store` | `POST /trainer/labels` | `auth`, `role:trainer` | `LabelPolicy::create : Label` | `StoreLabelRequest` — name (≤50, unique per trainer, case-insensitive), color | Redirect back + status | AC-03-11 |
| `crm.labels.update` | `PATCH /trainer/labels/{label}` | `auth`, `role:trainer` | `LabelPolicy::update : Label` | `UpdateLabelRequest` | Redirect back + status | AC-03-13 |
| `crm.labels.destroy` | `DELETE /trainer/labels/{label}` | `auth`, `role:trainer` | `LabelPolicy::delete : Label` | `DestroyLabelRequest` — confirmation naming the affected player count | Redirect back + status — cascades off every player (BR-03-5) | AC-03-13/14 |
| `crm.players.labels.attach` | `POST /trainer/players/{playerProfile}/labels` | `auth`, `role:trainer` | `PlayerLabelPolicy::create : PlayerLabel` | `AttachLabelsRequest` — `label_ids[]` | Redirect back + status | AC-03-12 |
| `crm.players.labels.detach` | `DELETE /trainer/players/{playerProfile}/labels/{label}` | `auth`, `role:trainer` | `PlayerLabelPolicy::delete : PlayerLabel` | — | Redirect back + status | AC-03-12 |
| `crm.players.flags.store` | `POST /trainer/players/{playerProfile}/flags` | `auth`, `role:trainer` | `PlayerFlagPolicy::apply : PlayerFlag` — trainer and Super Admin only (Q-03.05 default; footnotes AC-03-18's own text, which the requirements analyst already flagged as self-contradicting) | `ApplyFlagRequest` — one of the 8 fixed types, optional note | Redirect back + status | AC-03-15/16 |
| `crm.players.flags.resolve` | `POST /trainer/players/{playerProfile}/flags/{playerFlag}/resolve` | `auth`, `role:trainer` | `PlayerFlagPolicy::resolve : PlayerFlag` | `ResolveFlagRequest` — "Mark as resolved?" confirmation | Redirect back + status — hidden from active view, kept in history | AC-03-17 |
| `crm.players.notes.store` | `POST /trainer/players/{playerProfile}/notes` | `auth`, `role:trainer` | `PlayerNotePolicy::createGeneral : PlayerNote` | `StoreGeneralNoteRequest` — text (≤1000), optional `event_id` for a per-event note | Redirect back + status | AC-03-19/20 |
| `crm.notes.update` | `PATCH /trainer/notes/{playerNote}` | `auth`, `role:trainer` | `PlayerNotePolicy::update : PlayerNote` — author-only, 24h window (BR-03-12) | `UpdateNoteRequest` | Redirect back + status | AC-03-21 |
| `crm.notes.destroy` | `DELETE /trainer/notes/{playerNote}` | `auth`, `role:trainer` | `PlayerNotePolicy::delete : PlayerNote` — author-only, any time; never a coach's note | — | Redirect back + status | AC-03-21 |
| `crm.dashboard` | `GET /trainer/dashboard` | `auth`, `role:trainer` | `PlayerTrainerMembershipPolicy::viewAny : PlayerTrainerMembership` | — | Blade view — Quick View: events this week, RSVPs by type, Top 10 Players, flag counts, ShareLink opens | AC-03-34..42 |
| `crm.coach.players.index` | `GET /coach/players` | `auth`, `role:coach` | `PlayerTrainerMembershipPolicy::viewAsCoach : PlayerTrainerMembership` — event-scoped via `CoachVisibilityService::reachablePlayerIds()` | — | Blade view — only players RSVP'd to this coach's assigned events | AC-03-43/64/65 |
| `crm.coach.players.show` | `GET /coach/players/{playerProfile}` | `auth`, `role:coach` | `PlayerTrainerMembershipPolicy::viewAsCoach : PlayerProfile` | — | Blade view — coach-scoped, read-only flags/labels | AC-03-44/45 |
| `crm.coach.feedback.store` | `POST /coach/players/{playerProfile}/feedback` | `auth`, `role:coach` | `PlayerNotePolicy::createSession : PlayerNote` — only for an event the coach is assigned to and shares with this player | `StoreSessionFeedbackRequest` — `event_id`, text (≤500) | Redirect back + status | AC-03-46/47/48 |
| `crm.coach.feedback.update` | `PATCH /coach/feedback/{playerNote}` | `auth`, `role:coach` | `PlayerNotePolicy::update : PlayerNote` — own, 24h | `UpdateFeedbackRequest` | Redirect back + status | AC-03-49 |
| `crm.coach.feedback.destroy` | `DELETE /coach/feedback/{playerNote}` | `auth`, `role:coach` | `PlayerNotePolicy::delete : PlayerNote` — own, 24h | — | Redirect back + status | AC-03-49 |

Coach session feedback and trainer general/per-event notes are the same
`player_notes` model (`note_type: general|session`) — two route groups, two
Form Requests, two Policy abilities, because the actors and screens are
genuinely distinct, but no second model.

---

### Content (Epic-04)

| Route name | Method & URI | Middleware | Policy : Model | Form Request | Response | Serves |
|---|---|---|---|---|---|---|
| `content.playlists.index` | `GET /trainer/content/{pillar}` | `auth`, `role:trainer` | `PlaylistPolicy::viewAny : Playlist` | — | Blade view — `{pillar}` is `learn`\|`practice` | — |
| `content.playlists.create` | `GET /trainer/content/{pillar}/create` | `auth`, `role:trainer` | `PlaylistPolicy::create : Playlist` | — | Blade view (Livewire: the builder, per `architect-architecture.md` line 914) | AC-04-1, AC-04-11 |
| `content.playlists.store` | `POST /trainer/content/{pillar}` | `auth`, `role:trainer` | `PlaylistPolicy::create : Playlist` | `StorePlaylistRequest` — title (≤100), optional filters, visibility, ≥1 video (learn) — the equivalent field set for practice's drill selection | Redirect to `content.playlists.show` + status | AC-04-1..3, AC-04-11/12 |
| `content.playlists.show` | `GET /trainer/content/{pillar}/{playlist}` | `auth`, `role:trainer` | `PlaylistPolicy::view : Playlist` | — | Blade view | — |
| `content.playlists.edit` | `GET /trainer/content/{pillar}/{playlist}/edit` | `auth`, `role:trainer` | `PlaylistPolicy::update : Playlist` | — | Blade view | AC-04-31 |
| `content.playlists.update` | `PATCH /trainer/content/{pillar}/{playlist}` | `auth`, `role:trainer` | `PlaylistPolicy::update : Playlist` | `UpdatePlaylistRequest` — title/description/filters/visibility/item order/add-remove, all in one submission (drag-and-drop is Livewire on this same page; the plain fallback is a numbered-position `<select>` per item, both writing the same `sequence_order` field set) | Redirect back + "Playlist updated!" — assigned players see it immediately, prior progress unaffected (AC-04-32/33) | AC-04-31..33 |
| `content.playlists.destroy` | `DELETE /trainer/content/{pillar}/{playlist}` | `auth`, `role:trainer` | `PlaylistPolicy::delete : Playlist` | `DestroyPlaylistRequest` — "this will unassign it from all players" confirmation | Redirect back + status — soft delete | AC-04-34 |
| `content.playlists.visibility` | `PATCH /trainer/content/{pillar}/{playlist}/visibility` | `auth`, `role:trainer` | `PlaylistPolicy::updateVisibility : Playlist` | `UpdateContentVisibilityRequest` — `is_public`, `audience` (`players_and_coaches`\|`coaches_only`, A9's three-state) | Redirect back + status | AC-04-17..19, A9 |
| `content.playlists.assignments.store` | `POST /trainer/content/{pillar}/{playlist}/assignments` | `auth`, `role:trainer` | `PlaylistAssignmentPolicy::create : PlaylistAssignment` | `StorePlaylistAssignmentRequest` — `target_type` (`player`\|`label`\|`skill_level`), optional due_date/note | Redirect back + status; a label/skill-level target's bulk fan-out is queued (BR-04-13) | AC-04-13..16 |
| `content.drills.index` | `GET /trainer/content/drills` | `auth`, `role:trainer` | `ContentItemPolicy::viewAny : ContentItem` | — | Blade view — "My Drills" | — |
| `content.drills.public` | `GET /trainer/content/drills/public` | `auth`, `role:trainer` | `ContentItemPolicy::viewAny : ContentItem` (widened publication scope) | — | Blade view — discovery, search/filter | AC-04-7..10 |
| `content.drills.create` | `GET /trainer/content/drills/create` | `auth`, `role:trainer` | `ContentItemPolicy::create : ContentItem` | — | Blade view | AC-04-4 |
| `content.drills.store` | `POST /trainer/content/drills` | `auth`, `role:trainer` | `ContentItemPolicy::create : ContentItem` | `StoreDrillRequest` — name (≤100), YouTube URL, ≥1 category | Redirect to `content.drills.show` + status | AC-04-4..6 |
| `content.drills.add-to-playlist` | `POST /trainer/content/drills/{contentItem}/add-to-playlist` | `auth`, `role:trainer` | `ContentItemPolicy::view : ContentItem`, `PlaylistItemPolicy::create : PlaylistItem` | `AddDrillToPlaylistRequest` — target playlist (existing or new) | Redirect + "Drill added to [Playlist Name]" — stores a reference, not a copy (BR-04-12) | AC-04-8 |
| `content.drills.visibility` | `PATCH /trainer/content/drills/{contentItem}/visibility` | `auth`, `role:trainer` | `ContentItemPolicy::updateVisibility : ContentItem` | `UpdateContentVisibilityRequest` (same class as playlists, binary here — drills carry no coach-only audience field) | Redirect back + status | AC-04-17/18 |
| `content.drills.destroy` | `DELETE /trainer/content/drills/{contentItem}` | `auth`, `role:trainer` | `ContentItemPolicy::delete : ContentItem` | `DestroyDrillRequest` — "used in N playlists" warning if in use | Redirect back + status — soft delete | AC-04-35/36 |
| `content.portal.index` | `GET /portal/content` | `auth` | `PlaylistPolicy::viewAny : Playlist` (paywall-aware — locked items show, purchase gates access) | — | Blade view — Learn / Practice / Progress tabs, scoped to the current trainer context only (BR-04-10, A6) | AC-04-20..23 |
| `content.portal.show` | `GET /portal/content/{contentItem}` | `auth` | `ContentItemPolicy::view : ContentItem` — locked → redirect to purchase (AC-04-22) | — | Blade view — embedded YouTube player + instructions; **the GET itself marks `content_progress` started** (BR-04-16's "engagement started" trigger, satisfied without any JS-only "on play" beacon — opening the page is the trigger) | AC-04-24..26 |
| `content.progress.index` | `GET /portal/progress` | `auth` | `ContentProgressPolicy::viewAny : ContentProgress` | — | Blade view — overall stats, recent activity, incomplete-by-due-date, per-playlist bars | AC-04-27..29 |

A trainer's view of a player's progress (AC-04-30) is a section on
`crm.players.show`, not a separate route. Super Admin's LPPP Analytics is
under Administration, below.

---

### Billing (Epic-05)

Every Form Request in this module accepts an identifying selection and a
payment method; never an amount. `payment_records.platform_fee_minor_units`
is computed once, server-side, at row creation, per
`architect-architecture.md` "Money and the platform fee" (lines 819-849) —
restated here only as the reason no route below has a price field.

| Route name | Method & URI | Middleware | Policy : Model | Form Request | Response | Serves |
|---|---|---|---|---|---|---|
| `billing.trainer.index` | `GET /trainer/billing` | `auth`, `role:trainer` | `TrainerBillingSettingPolicy::view : TrainerBillingSetting` | — | Blade view — earnings summary, Stripe Dashboard link | AC-05-25 |
| `billing.trainer.connect.start` | `GET /trainer/billing/stripe/connect` | `auth`, `role:trainer` | `TrainerBillingSettingPolicy::connect : TrainerBillingSetting` | — | `redirect()->away(...)` — a fresh Stripe Account Link, created idempotently on every hit (Account Links expire quickly; there is no separate "reconnect" route) | AC-05-1 |
| `billing.trainer.connect.return` | `GET /trainer/billing/stripe/connect/return` | `auth`, `role:trainer` | — | — | Redirect to `billing.trainer.index` + status | AC-05-1 (Stripe's own Account Link contract) |
| `billing.trainer.connect.refresh` | `GET /trainer/billing/stripe/connect/refresh` | `auth`, `role:trainer` | — | — | `redirect()->away(...)` — a new Account Link | AC-05-1 |
| `billing.trainer.dashboard-link` | `GET /trainer/billing/stripe/dashboard-link` | `auth`, `role:trainer` | `TrainerBillingSettingPolicy::view : TrainerBillingSetting` | — | `redirect()->away(...)` — Express Dashboard login link | AC-05-25/26 |
| `billing.trainer.token-packages.index` | `GET /trainer/billing/token-packages` | `auth`, `role:trainer` | `TokenPackagePolicy::viewAny : TokenPackage` | — | Blade view | schema note under `token_package` (`database-designer-schema.md` lines 1567-1571) — no AC states trainer editability directly; cited to the schema's own design intent, not invented from nothing |
| `billing.trainer.token-packages.update` | `PATCH /trainer/billing/token-packages/{tokenPackage}` | `auth`, `role:trainer` | `TokenPackagePolicy::update : TokenPackage` | `UpdateTokenPackageRequest` | Redirect back + status | same |
| `billing.trainer.pricing.update` | `PATCH /trainer/billing/pricing` | `auth`, `role:trainer` | `TrainerBillingSettingPolicy::update : TrainerBillingSetting` — trainer's own `token_price_minor_units`/`player_subscription_price_minor_units`, **not** the platform fee rate | `UpdateOwnPricingRequest` | Redirect back + status | BR-05-1, BR-05-14 |
| `billing.tokens.index` | `GET /tokens` | `auth` | `TokenBalancePolicy::view : TokenBalance` | — | Blade view — balance, Buy Tokens | AC-05-4 |
| `billing.tokens.checkout` | `POST /tokens/checkout` | `auth`, `throttle:payment` | `TokenBalancePolicy::purchase : TokenBalance` | `StoreTokenCheckoutRequest` — `token_package_id` or a custom amount | `redirect()->away($stripeCheckoutUrl)` | AC-05-4..6 |
| `billing.subscriptions.checkout` | `POST /tokens/subscription/checkout` | `auth`, `throttle:payment` | `SubscriptionEntitlementPolicy::purchase : SubscriptionEntitlement` | `StoreSubscriptionCheckoutRequest` — `activation_date`, may be future-dated | `redirect()->away($stripeCheckoutUrl)` | AC-05-29, BR-05-14 |
| `content.purchases.store` | `POST /content/playlists/{playlist}/purchase` | `auth`, `throttle:payment` | `PlaylistAccessGrantPolicy::create : Playlist` | `StoreContentPurchaseRequest` — `payment_method` (`token`\|`card`) | Token: instant, redirect to `content.portal.show` + status. Card: `redirect()->away($stripeCheckoutUrl)` | AC-05-18/19 |
| `billing.payment-methods.index` | `GET /billing/payment-methods` | `auth`, `role:player` | `StripeCustomerLinkPolicy::manage : StripeCustomerLink` | — | `redirect()->away(...)` — Stripe Customer Portal, no custom UI | AC-05-20/21 |
| `billing.transactions.index` | `GET /transactions` | `auth` | `PaymentRecordPolicy::viewAny : PaymentRecord` | — | Blade view — current trainer context only (BR-05-17, A6), filterable, 50/page | AC-05-22/23 |
| `crm.players.tokens.gift` | `POST /trainer/players/{playerProfile}/tokens/gift` | `auth`, `role:trainer` | `TokenBalancePolicy::gift : TokenBalance` — trainer only, never a coach | `GiftTokensRequest` — amount, note | Redirect back + status; audit-logged (AC-05-33, Epic-07 integration) | AC-05-33 |
| `administration.trainers.pricing.update` | `PATCH /admin/trainers/{trainer}/pricing` | `auth`, `role:super_admin` | `TrainerBillingSettingPolicy::adminUpdate : TrainerBillingSetting` — via `AdministrativeScope` | `UpdateTrainerFeeRateRequest` — subscription amount and/or fee rate, optional reason | Redirect to `administration.trainers.show` + status | AC-05-27/28, AC-07-40 |

`identity.approvals.approve` (above) is where a child's payment-requiring
request actually reaches Stripe — this module deliberately has no second
"approve and pay" route.

---

### Growth (Epic-06)

| Route name | Method & URI | Middleware | Policy : Model | Form Request | Response | Serves |
|---|---|---|---|---|---|---|
| `growth.referrals.index` | `GET /referrals` | `auth`, `role:player` | `ReferralLinkPolicy::view : ReferralLink` | — | Blade view — the link (never generated, always present), Share button, stats | AC-06-1/2/3 |
| `growth.trainer.referrals.index` | `GET /trainer/marketing/referrals` | `auth`, `role:trainer` | `ReferralPolicy::viewAny : Referral` | — | Blade view — overview metrics, Top Referrers, activity log | AC-06-13..16 |
| `growth.coupons.index` | `GET /trainer/marketing/coupons` | `auth`, `role:trainer` | `CouponPolicy::viewAny : Coupon` | — | Blade view | AC-06-26/28 |
| `growth.coupons.create` | `GET /trainer/marketing/coupons/create` | `auth`, `role:trainer` | `CouponPolicy::create : Coupon` | — | Blade view | AC-06-17 |
| `growth.coupons.store` | `POST /trainer/marketing/coupons` | `auth`, `role:trainer` | `CouponPolicy::create : Coupon` | `StoreCouponRequest` — code (4-20 chars, unique per trainer), discount type/value, applies_to, usage limit, optional expiry | Redirect to `growth.coupons.index` + status | AC-06-17/18/19 |
| `growth.coupons.show` | `GET /trainer/marketing/coupons/{coupon}` | `auth`, `role:trainer` | `CouponPolicy::view : Coupon` | — | Blade view — usage details, per-player redemptions | AC-06-26/27 |
| `growth.coupons.edit` | `GET /trainer/marketing/coupons/{coupon}/edit` | `auth`, `role:trainer` | `CouponPolicy::update : Coupon` | — | Blade view | AC-06-27 |
| `growth.coupons.update` | `PATCH /trainer/marketing/coupons/{coupon}` | `auth`, `role:trainer` | `CouponPolicy::update : Coupon` | `UpdateCouponRequest` — expiration, usage limit, status only | Redirect back + status | AC-06-27 |
| `growth.coupons.deactivate` | `POST /trainer/marketing/coupons/{coupon}/deactivate` | `auth`, `role:trainer` | `CouponPolicy::update : Coupon` | — | Redirect back + status | AC-06-27 |
| `growth.coupons.destroy` | `DELETE /trainer/marketing/coupons/{coupon}` | `auth`, `role:trainer` | `CouponPolicy::delete : Coupon` — blocked once used | — | Redirect back + status | AC-06-27 |
| `administration.settings.referral-program.edit` | `GET /admin/settings/referral-program` | `auth`, `role:super_admin` | `PlatformConfigurationPolicy::view : PlatformConfiguration` | — | Blade view | AC-06-29 |
| `administration.settings.referral-program.update` | `PATCH /admin/settings/referral-program` | `auth`, `role:super_admin` | `PlatformConfigurationPolicy::update : PlatformConfiguration` — global table, no tenant/`AdministrativeScope` involved | `UpdateReferralProgramRequest` — referrals_required, tokens_awarded, referee_welcome_bonus | Redirect back + status; applies to all trainers immediately, assist counts preserved (AC-06-30) | AC-06-29..31 |

Coupon "Apply" at checkout is a field on the RSVP/purchase Form Request
itself (`coupon_code`, optional) validated inline by the same request that
processes the payment — not a separate route. Where the checkout screen is
one of the four Livewire-eligible pages, applying a coupon can additionally
preview the discount live before submission; the no-JS path validates it as
part of the one full submission instead, with the same result either way.

---

### Administration (Epic-07)

Every route below except the two marked `AdministrativeScope` resolves
**no tenant at all** (source 1) and reads through `CrossTenantReadService` or
a global-table model — consistent with `architect-architecture.md`'s own
statement that `Administration` "May call: Every module, read-only, through
projections" (line 88).

| Route name | Method & URI | Middleware | Policy : Model | Form Request | Response | Serves |
|---|---|---|---|---|---|---|
| `administration.dashboard` | `GET /admin/dashboard` | `auth`, `role:super_admin` | `AdministrationPolicy::viewDashboard` | — | Blade view — operational metrics only, date-range preset, "View Financial Reports in Stripe" link; **no revenue/payout/transaction figure anywhere on it** (AC-07-7) | AC-07-1..7 |
| `administration.users.index` | `GET /admin/users` | `auth`, `role:super_admin` | `AccountPolicy::viewAny : Account` | — | Blade view — search, role/status filters, 50/page | AC-07-8..10, AC-07-12 |
| `administration.users.show` | `GET /admin/users/{account}` | `auth`, `role:super_admin` | `AccountPolicy::view : Account` | — | Blade view | AC-07-11 |
| `administration.users.edit` | `GET /admin/users/{account}/edit` | `auth`, `role:super_admin` | `AccountPolicy::update : Account` | — | Blade view — role/trainer view-only | AC-07-13 |
| `administration.users.update` | `PATCH /admin/users/{account}` | `auth`, `role:super_admin` | `AccountPolicy::update : Account` | `AdminUpdateAccountRequest` — name, email (re-verification warning), status; player-specific fields when applicable | Redirect back + "Super Admin edited user [Name]"; audit-logged | AC-07-13..15 |
| `administration.users.deactivate` | `POST /admin/users/{account}/deactivate` | `auth`, `role:super_admin` | `AccountPolicy::deactivate : Account` | `DeactivateAccountRequest` — confirmation | Redirect back + status | AC-01-52/53, AC-07-39 |
| `administration.users.reactivate` | `POST /admin/users/{account}/reactivate` | `auth`, `role:super_admin` | `AccountPolicy::reactivate : Account` | — | Redirect back + status | AC-01-54 |
| `administration.users.destroy` | `DELETE /admin/users/{account}` | `auth`, `role:super_admin` | `AccountPolicy::delete : Account` | `DeleteAccountRequest` — explicit "cannot be undone" confirmation, reason | Redirect back + status — anonymizes in place, never a hard delete (BR-01-24) | AC-01-55..59 |
| `administration.users.impersonate` | `POST /admin/users/{account}/impersonate` | `auth`, `role:super_admin` | `AccountPolicy::impersonate : Account` — target `role !== super_admin` (AC-01-37) | `ImpersonateAccountRequest` — confirmation naming the target and role | Redirect to the target's dashboard, sticky "Viewing as" banner | AC-01-33/34, AC-07-11 |
| `administration.impersonation.exit` | `POST /admin/impersonation/exit` | `auth` | `ImpersonationPolicy::exit` — whoever is currently impersonating | — | Redirect to the admin's own dashboard | AC-01-35 |
| `administration.trainers.index` | `GET /admin/trainers` | `auth`, `role:super_admin` | `TrainerPolicy::viewAny : Trainer` | — | Blade view | AC-07-38 |
| `administration.trainers.show` | `GET /admin/trainers/{trainer}` | `auth`, `role:super_admin` | `TrainerPolicy::view : Trainer` | — | Blade view — subscription status via Stripe | AC-07-38 |
| `administration.trainers.feature-toggles.edit` | `GET /admin/trainers/{trainer}/feature-settings` | `auth`, `role:super_admin` | `FeatureTogglePolicy::view : Trainer` | — | Blade view — 3 toggles, defaults on | AC-07-18/19 |
| `administration.trainers.feature-toggles.update` | `PATCH /admin/trainers/{trainer}/feature-settings` | `auth`, `role:super_admin` | `FeatureTogglePolicy::update : Trainer` — **no tenant resolution at all**: `feature_toggles` is global, referenced by `trainer_id`, carries no RLS (`architect-architecture.md` lines 684-690) | `UpdateFeatureTogglesRequest` — confirmation naming the trainer and effect | Redirect back + status; applies within seconds, hides rather than deletes (BR-07-3) | AC-07-19/20/21 |
| `administration.trainers.stripe-dashboard-link` | `GET /admin/trainers/{trainer}/stripe-dashboard-link` | `auth`, `role:super_admin` | `TrainerPolicy::view : Trainer` | — | `redirect()->away(...)` | AC-07-37 |
| `administration.events.index` | `GET /admin/events` | `auth`, `role:super_admin` | `EventPolicy::viewAnyAdmin : Event` | — | Blade view — every trainer's events, 50/page | AC-07-22..24, AC-07-26 |
| `administration.events.export` | `GET /admin/events/export` | `auth`, `role:super_admin` | `EventPolicy::viewAnyAdmin : Event` | — | CSV download | AC-02-57 |
| `administration.events.show` | `GET /admin/events/{event}` | `auth`, `role:super_admin` | `EventPolicy::viewAdmin : Event` | — | Blade view | AC-07-25 |
| `administration.events.update` | `PATCH /admin/events/{event}` | `auth`, `role:super_admin` | `EventPolicy::adminOverride : Event` — via `AdministrativeScope`, wrapping `Scheduling\Actions\UpdateEvent` | `AdminUpdateEventRequest` — no conflict warnings, overrides recorded regardless | Redirect back + status; audit-logged ("Super Admin overrode conflict: [details]") | AC-07-25/27/28 |
| `administration.events.cancel` | `POST /admin/events/{event}/cancel` | `auth`, `role:super_admin` | `EventPolicy::adminOverride : Event` — via `AdministrativeScope`, wrapping `Scheduling\Actions\CancelEvent` | `AdminCancelEventRequest` | Redirect back + status | AC-07-25, AC-02-48 |
| `administration.audit-log.index` | `GET /admin/audit-log` | `auth`, `role:super_admin` | `AuditLogEntryPolicy::viewAny : AuditLogEntry` | — | Blade view — date range, action type, subject search | AC-07-29..32 |
| `administration.audit-log.export` | `GET /admin/audit-log/export` | `auth`, `role:super_admin` | `AuditLogEntryPolicy::viewAny : AuditLogEntry` | — | CSV download | AC-07-33 |
| `administration.crm.index` | `GET /admin/crm` | `auth`, `role:super_admin` | `PlayerTrainerMembershipPolicy::viewAnyAdmin : PlayerTrainerMembership` | — | Blade view — CRM Master, cross-trainer flag application | AC-03-54..58 |
| `administration.content-analytics.index` | `GET /admin/content-analytics` | `auth`, `role:super_admin` | `ContentUsagePolicy::viewAnyAdmin : ContentUsage` | — | Blade view — content/engagement/growth stats, per-trainer drill-down | AC-04-37..39 |
| `administration.content-analytics.export` | `GET /admin/content-analytics/export` | `auth`, `role:super_admin` | `ContentUsagePolicy::viewAnyAdmin : ContentUsage` | — | CSV download | AC-04-40 |

There is no `administration.forms.*` — Super Admin reaches a trainer's camp
and evaluation forms exclusively by impersonating that trainer (BR-08-21,
`epic-08:218`), not through a dedicated cross-tenant tool. Once
impersonating, `Forms`' own routes below already apply unmodified, since
impersonation resolves a real tenant (source 2).

---

### Forms (Epic-08)

Coaches are denied every ability on `FormPolicy` outright (BR-08-20); the
route list below applies to trainers only among authenticated roles.

| Route name | Method & URI | Middleware | Policy : Model | Form Request | Response | Serves |
|---|---|---|---|---|---|---|
| `forms.index` | `GET /trainer/forms` | `auth`, `role:trainer` | `FormPolicy::viewAny : Form` | — | Blade view — camps and evaluations, submission counts, spots remaining | AC-08-27/32 |
| `forms.camps.create` | `GET /trainer/forms/camps/create` | `auth`, `role:trainer` | `FormPolicy::create : Form` | — | Blade view — pre-loaded template | AC-08-1 |
| `forms.camps.store` | `POST /trainer/forms/camps` | `auth`, `role:trainer` | `FormPolicy::create : Form` | `StoreCampFormRequest` — name, dates (display-only), description, capacity (1-1000), optional price ($1-$10,000 or free), field definitions | Redirect to `forms.show` + status; issues the `public_tenant_codes` mapping in the same transaction | AC-08-1..8 |
| `forms.evaluations.create` | `GET /trainer/forms/evaluations/create` | `auth`, `role:trainer` | `FormPolicy::create : Form` | — | Blade view | AC-08-9 |
| `forms.evaluations.store` | `POST /trainer/forms/evaluations` | `auth`, `role:trainer` | `FormPolicy::create : Form` | `StoreEvaluationFormRequest` — no capacity field at all (BR-08-3), not merely one left blank | Redirect to `forms.show` + status | AC-08-9..12 |
| `forms.show` | `GET /trainer/forms/{form}` | `auth`, `role:trainer` | `FormPolicy::view : Form` | — | Blade view — shareable link, submission count | AC-08-27/32 |
| `forms.edit` | `GET /trainer/forms/{form}/edit` | `auth`, `role:trainer` | `FormPolicy::update : Form` | — | Blade view — warns of existing submission count if any | AC-08-28 |
| `forms.update` | `PATCH /trainer/forms/{form}` | `auth`, `role:trainer` | `FormPolicy::update : Form` | `UpdateFormRequest` — capacity cannot drop below current registrations (AC-08-30) | Redirect back + status | AC-08-28/30 |
| `forms.toggle` | `POST /trainer/forms/{form}/toggle` | `auth`, `role:trainer` | `FormPolicy::update : Form` — camps only | — | Redirect back + status — disabled shows "Registration Closed" on `public.form.show` | AC-08-8/29 |
| `forms.destroy` | `DELETE /trainer/forms/{form}` | `auth`, `role:trainer` | `FormPolicy::delete : Form` — blocked while any unrefunded paid registration exists | `DestroyFormRequest` — confirmation | Redirect back + status | AC-08-31 |
| `forms.submissions.index` | `GET /trainer/forms/{form}/submissions` | `auth`, `role:trainer` | `FormSubmissionPolicy::viewAny : Form` | — | Blade view — filterable by payment/conversion status, 50/page | AC-08-17/18 |
| `forms.submissions.export` | `GET /trainer/forms/{form}/submissions/export` | `auth`, `role:trainer` | `FormSubmissionPolicy::viewAny : Form` | — | CSV download | AC-08-19 |
| `forms.submissions.attendance` | `PATCH /trainer/forms/{form}/submissions/{formSubmission}/attendance` | `auth`, `role:trainer` | `FormSubmissionPolicy::update : FormSubmission` | `MarkSubmissionAttendanceRequest` | Redirect back + status | AC-08-20 |
| `forms.message.store` | `POST /trainer/forms/{form}/message` | `auth`, `role:trainer` | `FormSubmissionPolicy::message : Form` | `SendBulkFormMessageRequest` | Redirect back + status; queued mail, one per participant | AC-08-21 |

The public half of this module (`public.form.*`) is listed under Identity's
"Public ShareLink and referral acceptance" table above, since all ten
code-resolved routes share one middleware group and one failure-mode table —
splitting them by owning module would duplicate that table for no benefit.

---

## Webhook contract — Stripe

**The one genuinely JSON route in the application**, and the one route that
never runs `ResolveTenant`'s session/CSRF half at all.

| Route name | Method & URI | Middleware | Response |
|---|---|---|---|
| `platform.stripe.webhook` | `POST /stripe/webhook` | none from `web` (no session, no `auth`); a dedicated `VerifyStripeWebhookSignature` middleware only | `200` JSON `{"received": true}` on every outcome that does not fail signature verification |

Registered in `routes/web.php` (matching `architect-architecture.md`'s
three-file layout, which names no fourth file for it), with its URI excluded
from CSRF in `bootstrap/app.php`:
`$middleware->validateCsrfTokens(except: ['stripe/webhook'])` — the standard
Laravel mechanism for a route that cannot supply a token, per "CSRF
placement" above.

**Two phases, exactly per `architect-architecture.md`'s "Synchronous and
asynchronous work"** (the receipt row is synchronous, line 722; processing is
queued on the `payments` connection, line 729):

1. **Signature verification** (`VerifyStripeWebhookSignature`) — `stripe-php`'s
   `Webhook::constructEvent()` against the raw body, the `Stripe-Signature`
   header, and the webhook secret. Failure: `400`, nothing written, nothing
   queued.
2. **Receipt insert, its own transaction, before the body is parsed** —
   `INSERT INTO stripe_event_receipts (stripe_event_id, event_type) VALUES
   (...)`. The unique constraint on `stripe_event_id` **is** the idempotency
   check (`architect-architecture.md` line 722, `database-designer-schema.md`
   lines 608-611): a unique-violation means this event was already seen,
   caught, and answered `200` immediately with nothing re-dispatched — a
   replay is indistinguishable from success to Stripe.
3. **On a first-seen event**, dispatch `Billing\Jobs\ProcessStripeWebhookEvent`
   on the `payments` queue and return `200` immediately, so the request never
   waits on the actual handling.

**Inside the job** — the tenant-resolution completion this document adds,
since neither carried-over document spells out the connecting step beyond
"the tenant is resolved from the `PaymentRecord` the event references"
(`architect-architecture.md` line 178): the event's metadata carries the
locally-assigned `payment_records.id` (set as Stripe metadata when the
`PaymentRecord` row and the Stripe object are created together, before the
Stripe call — `database-designer-schema.md` lines 1531-1540's own
idempotency-key design already requires this). The job reads that one row's
`trainer_id` via `CrossTenantReadService` (a new, narrow method,
`paymentRecordTrainerId(int $paymentRecordId): int`, alongside its existing
`contentItemUsageCount`), then wraps the actual handling in
`TenantContext::runFor($trainerId, ...)` — squarely within `runFor()`'s
already-stated scope for jobs (`architect-architecture.md` lines 264-271),
no extension needed here, unlike `AdministrativeScope` above.

| Stripe event | Handling, under the resolved tenant | Serves |
|---|---|---|
| `payment_intent.succeeded` | `payment_records.status = 'completed'`; confirm the related `rsvp`/unlock the `playlist_access_grant`/grant the `subscription_entitlement`, by calling into the owning module's own Action, never writing that model from `Billing` directly | AC-05-34 |
| `payment_intent.payment_failed` | `status = 'failed'`; notify the player; retry surfaced in-app | AC-05-34, BR-05-8 |
| `charge.refunded` | Insert the compensating `token_entries` row (if a token-funded item) or mark the `payment_records` refund row; update transaction history | AC-05-34 |
| `customer.subscription.created` | Grant subscription access (trainer's own platform subscription, `platform_subscriptions.status = 'active'`) | AC-05-34, BR-05-13 |
| `customer.subscription.deleted` | Revoke; `platform_subscriptions.status = 'canceled'`/`'suspended'` per BR-05-13's retry policy | AC-05-34, BR-05-13 |
| `account.updated` | `trainer_billing_settings.stripe_connect_onboarding_status` | AC-05-34 |

**Reliability**: `$tries = 3`, `retryUntil(): now()->addDay()`, exponential
backoff (`[60, 900, 14400]`, roughly matching "3 attempts over 24 hours",
AC-05-35). A scheduled sweep (`stripe_event_receipts.processed_at IS NULL`,
every 5 minutes, per `architect-architecture.md` line 748) catches anything
the queue itself dropped. Security requirements (idempotency keys, no card
data ever stored, encrypted Stripe API keys, rate-limited payment attempts)
are AC-05-36, already covered structurally: the idempotency key is derived
from `payment_records.id` (assigned before the Stripe call), card data never
enters this application at all (Checkout is Stripe-hosted), and
`throttle:payment` covers the attempt-rate half.

---

## Public unauthenticated surface — tenant resolution summary

All ten routes, one table, matching `architect-architecture.md`'s own
failure-mode table (lines 514-522) with this document's tenth row added and
marked.

| Route | Resolves via | Unknown/invalid identifier | Resolved but stale | Audited |
|---|---|---|---|---|
| `public.join.show` / `.register` / `.associate` | source 5, `public_tenant_codes` | 404, zero trainer-scoped statements | `ShareLinkPolicy::resolve` denies — "this invitation has expired" + resend path (AC-01-42) | Membership created: yes. Stale denial: yes. Unknown code: no — rate-limited and counted only |
| `public.invite.show` / `.register` | source 5, `public_tenant_codes` | 404 | Same Policy, same messaging | Same |
| `public.referral.show` *(this document's addition)* | direct `trainers.slug` lookup, no `public_tenant_codes` row involved | Unknown slug: 404 (no such trainer) | N/A — a referral link has no expiry/status of its own to go stale; it degrades to the ordinary `public.join.show` behavior once the tenant resolves | Click: `referrals.clicked_at`, not the audit log (analytics, not a security record — same reasoning as `share_link_opens`) |
| `public.form.show` / `.submit` / `.confirmation` / `.convert` | source 5, `public_tenant_codes` | 404 | `FormPolicy::resolve` denies — "Registration Closed" (disabled) / "Camp Full" (at capacity); neither is a 404, since the form itself is real | Submission created: yes (implicitly, via the `form_submissions` row itself). Unknown code: no |

A code absent from `public_tenant_codes`, or a slug absent from `trainers`,
never issues a trainer-scoped statement — the middleware resolves nothing and
the route fails before any query that RLS would even need to filter.

---

## Decisions

| Decision | Chosen | Rejected | Why |
|---|---|---|---|
| Where this document gets its tenancy mechanism | `architect-architecture.md`, followed verbatim for sources 1-5, precedence, `TenantContext`, `BelongsToTenant`, and the route allow-list mechanism | Re-deriving the mechanism from `council-sharelink-tenant-resolution.md`/`database-designer-schema.md` directly, as this document's own first draft did before the architecture document appeared in `specs/` | The architecture document is the later, more complete, specifically-scoped-to-this-question source, and it exists now — building an independent, only-partially-consistent reconstruction on top of the same Symfony-era inputs it already reconciled would create a second, competing tenancy design in the same repository |
| Which sibling document wins where they disagree | `architect-architecture.md`, on the session-variable mechanism, the ledger service's name (`TokenLedger`), and the module namespace | `database-designer-eloquent-schema.md`'s own choices on the same three points | The schema document's own Conventions section defers the namespace question to `architect` by name; its `SET app.current_trainer_id = ?` snippet binds a parameter PostgreSQL's `SET` statement does not accept placeholders for, which `architect-architecture.md`'s own Decisions table independently and correctly rules out. Flagged for the two documents' owners, not corrected in either — out of this task's scope |
| Public form submission's response shape | Plain Blade, native `<form>` POST, redirect on success (to a confirmation page or to Stripe) | JSON, as this task's own brief hypothesized | `architect-architecture.md` "Presentation" commits Epic-08 to plain Blade explicitly and forbids Livewire on the whole code-resolved group; a JSON-only response on a route that must also work with JavaScript disabled would need content negotiation the settled architecture never asks for and the epic's own UX (one page, sub-five-minute) never needs |
| "Progressive-enhancement" endpoints | Livewire components on four named authenticated screens, using Livewire's own framework-managed endpoint — no bespoke JSON routes | A dozen small bespoke JSON API routes (reorder, live search, coupon preview), each with a hand-built no-JS fallback route | `architect-architecture.md` already names exactly this shape and exactly these four screens (line 913-915) and explicitly forbids the alternative shape (a shared JSON endpoint) from ever reaching the public group. Building a second, parallel JSON-endpoint system for the same interactions the architecture already assigned to Livewire would be redundant and would not obviously degrade as cleanly with JavaScript off |
| `AdministrativeScope`'s shape | A service (not middleware), called explicitly from `Administration` controllers, wrapping `TenantContext::runFor()` around a call into the owning module's own Action, Policy-gated and audited | Requiring full impersonation for every single-trainer admin write (Event Master edit/cancel, pricing); a middleware-based admin-scope resolver | Full impersonation for a one-field pricing edit is real UX friction the epics don't ask for (AC-07-25's "as if they had created it" reads as a direct action, not a "become this user first" flow); a middleware can't cleanly express "wrap this one call into another module's Action" the way a service method can. Flagged as this document's own synthesis, not a quotation — see Open questions |
| Submission id on the camp confirmation/conversion routes | Carried via server-side session, flashed at successful-submission time; never in the URL or query string | `?submission=123` or `/f/{code}/confirmation/{submission}` | A sequential id in a public, unauthenticated URL lets a stranger increment it and see (or pre-fill a signup with) another registrant's name and email — a minor but avoidable PII leak on the platform's most public surface. Matches the two given routes' own URIs exactly (`architect-architecture.md`'s snippet carries no submission id on either), which is corroborating evidence, not just this document's own preference |
| Coupon "Apply" | A field on the same checkout Form Request that processes the RSVP/purchase, validated inline | A separate `/checkout/coupon/apply` endpoint | One request, one Policy check, one place the discount and the charge can never disagree about what was actually validated; a separate preview endpoint would need its own re-validation at charge time anyway, since a coupon's usage limit can be consumed between the preview and the charge |
| Email verification mechanism | The schema's own stateful `email_verification_tokens` table (hash + 1h... 24h expiry), not Laravel/Breeze's default signed-URL flow | Laravel's built-in signed-URL verification (stateless, no token row) | The schema (a settled, carried-over input) already models this table to mirror `password_reset_tokens` exactly; diverging would mean either overriding a settled schema decision or running two parallel verification mechanisms for no stated reason. Laravel's `verified` middleware is still used as-is, since it only reads `email_verified_at`, which either mechanism sets identically |
| Rate-limit thresholds | Proposed concrete numbers (5/min login, 20/min public-code, 10/min payment) | Leaving the limiters unconfigured pending a client answer | `architect-architecture.md` itself requires both limiters exist and states neither number is settled (Open question 4); shipping a named limiter with no threshold is not enforceable. Numbers are flagged as proposals, following the same "safe default, not a decision" pattern `requirements-analyst-open-questions.md` Section B already uses throughout this project |

---

## Open questions

Carried forward from `architect-architecture.md` where this document inherits
them unresolved (not re-litigated), plus this document's own:

1. **`AdministrativeScope`'s exact shape is this document's synthesis, not a
   citation.** `architect-architecture.md`'s module map names it; nothing
   this document has read defines its method signature, its exact audit
   payload, or which specific routes beyond Event Master and per-trainer
   pricing should use it (feature toggles do not need it — see the route
   table's own note). Needs `architect`'s ratification before
   `architecture-implementer` scaffolds it.
2. **The referral link's tenant resolution (`public.referral.show`) is not on
   `architect-architecture.md`'s own nine-route allow-list.** This document
   adds it as a tenth route in `routes/public.php`, resolved by a new
   `SlugTenantResolver` rather than `PublicCodeTenantResolver`, since it reads
   `trainers.slug` directly. The structural test
   `architect-architecture.md` names for the allow-list ("asserting the
   allow-list against the router... a reviewed act", line 502) needs to be
   written against ten names, not nine, once this is confirmed — flagged
   rather than silently assumed correct.
3. **`share_links.link_type`'s two-value enum (`static_player`\|`unique_coach`)
   does not cleanly cover two flows the epics describe**: a coach inviting a
   player (US-03.11/AC-03-50, "a unique ShareLink for this invitation") and a
   trainer's optional one-time per-player invite links (AC-03-61, "optional
   MVP"). This document routes both through `/join/{code}`
   (`public.join.show`) reusing the `static_player` shape, since neither AC
   states an expiry or a use-limit for them — this document's inference, not
   a settled fact, carried unresolved from the schema's own silence on the
   point.
4. **The sibling-document naming inconsistencies** (session-variable
   mechanism, `TokenLedger` vs. `TokenLedgerService`, module vs. flat
   namespace) — see Decisions. Worth a short reconciliation pass over
   `architect-architecture.md` and `database-designer-eloquent-schema.md`
   directly; out of this document's own scope to fix.
5. **Rate-limit thresholds** (`architect-architecture.md` Open question 4,
   BR-01-6) — this document's proposed numbers (5/min login, 20/min
   public-code, 10/min payment) need the client's or the team's confirmation,
   not just internal consistency.
6. **Whether the four Livewire-eligible screens actually need Livewire.**
   `architect-architecture.md` Open question 7 already asks this; this
   document's route table treats all four as Livewire-eligible per the
   architecture's permission, not as a requirement — `frontend-design` may
   reasonably build any of them in plain Blade instead, which would only
   shrink this document's Livewire footnotes, not change any route's name,
   method, URI, Policy or Form Request.
7. **`Administration`'s write into `platform_configurations`**
   (`administration.settings.referral-program.update`) is presented as a
   direct Policy-gated write to a global table with no `AdministrativeScope`
   involved, on the reasoning that `platform_configurations` carries no
   `trainer_id` and needs no tenant. Whether it should instead route through
   a `Platform`-owned settings-update service (mirroring
   `PublicTenantCodeRegistry`'s "only the owning module writes it" pattern,
   since `PlatformConfiguration` is `Platform`-owned per the module map, not
   `Administration`-owned) is not settled by anything this document has
   read — flagged rather than picked silently.
