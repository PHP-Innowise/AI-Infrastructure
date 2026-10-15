# Security voter design: PracticePerfect platform

The complete authorization layer for the PracticePerfect platform: roles,
the voter inventory, parent-child approval, coach-trainer exclusivity and
reach, Super Admin's two cross-tenant write mechanisms, `access_control`
versus voters, and the voter test matrix.

**Inputs already settled — do not reopen.** `specs/architect-architecture.md`
("Authorization," "Tenancy enforcement," "Module map," and the Decisions
table); `specs/council-sharelink-tenant-resolution.md` (the Recommendation);
`specs/api-designer-spec.md` ("Route map by module," "Gates vs. voters,"
"Public unauthenticated routes and tenant resolution," "Impersonation and
administrative tenant scope"); Section A of
`specs/requirements-analyst-open-questions.md` (owner decisions A1-A12). The
23 voter classes and their attributes already named across those three
documents are treated as fixed vocabulary. Where a name is missing — a
delegation attribute the architecture describes but does not spell out, a
module placement the api-designer spec left implicit — this document adds
it and says so. Where the vocabulary looks wrong, this document says so
instead of silently renaming it; none did.

**This document does not contain routes, entities, or schema.** Those
belong to `specs/api-designer-spec.md`, `specs/architect-architecture.md`,
and the database-designer stage, respectively. It also does not authorize
anything by hiding it in the UI — every capability below is enforced
server-side regardless of what the navigation shows.

---

## Roles

Four Symfony role strings, fixed constants, not runtime-editable data (A2,
`specs/requirements-analyst-open-questions.md:32`; BR-01-7,
`specs/requirements-analyst-epic-01-user-management-spec.md:273`). Each
account holds exactly one.

| Role | Means |
|---|---|
| `ROLE_SUPER_ADMIN` | Platform operator. Creates trainers, impersonates, edits any account, configures feature toggles and platform-wide referral rules, overrides scheduling conflicts, views the audit log. Holds no tenant of its own. |
| `ROLE_TRAINER` | The tenant owner. Is their own tenant (resolution source 4, `architect-architecture.md:161`). Manages their organization's players, coaches, events, content, forms and billing settings. |
| `ROLE_COACH` | Works for exactly one trainer at a time (BR-01-11). Reach inside that tenant is further narrowed to assigned events by `CoachVisibilityService`. |
| `ROLE_PLAYER` | Covers both an adult player and a parent account — "a parent account is itself treated as a player account" (`specs/api-designer-spec.md:45`, citing `requirements-analyst-epic-01-user-management-spec.md:302-308`). A child may additionally hold their own `ROLE_PLAYER` login (AC-01-20, line 172) distinguished from the parent's not by role but by which `Account` `getUser()` returns — see Parent-child approval, below. |

**No role hierarchy is configured** (`architect-architecture.md`,
"Authorization," lines 462-466; Decisions table row "Role hierarchy," line
717). `ROLE_SUPER_ADMIN` does not inherit `ROLE_TRAINER`, `ROLE_COACH` or
`ROLE_PLAYER`. The reason is stated once and is load-bearing for everything
below: with a hierarchy, a voter written as "the subject's trainer is the
active tenant" would silently **pass** for a Super Admin who holds no
tenant at all, because Symfony's hierarchy resolution grants every
inherited role unconditionally, and a Super Admin has no tenant to compare
against — the comparison degenerates into "true" instead of failing safely.
That is exactly the cross-tenant write the tenancy design forbids (lines
462-466).

**What it costs.** Every voter below that touches a trainer-scoped subject
must decide, for itself, whether `ROLE_SUPER_ADMIN` gets in and how,
because there is no shared default to inherit. This design finds five
different, already-implied answers, and a voter that picks the wrong one
either locks Super Admin out of a capability an epic grants, or opens a
cross-tenant write the architecture forbids:

1. **Bare role, no scope** — the subject is global, so there is no tenant
   boundary to cross (`AccountVoter`, `PlatformConfigurationVoter`,
   `ImpersonationVoter`).
2. **Role plus an open `AdministrativeScope`** — the subject is
   trainer-scoped and a named `/super-admin/...` route reaches it
   (`EventVoter` via Event Master, `TrainerSettingsVoter::TRAINER_FEE_EDIT`).
3. **Role plus an active impersonation session, no scope clause at all** —
   Epic-08 states the mechanism explicitly (`FormVoter` — BR-08-21).
4. **No clause, deliberately** — the source states Super Admin does *not*
   get this capability even though the subject is otherwise admin-reachable
   (`LabelVoter::LABEL_MANAGE` — AC-03-58).
5. **Capability settled, mechanism not** — an AC grants the capability but
   no route or scope mechanism is named anywhere (`PlayerVoter` on CRM
   Master's flag/note/feedback edits — see Open questions).

Each is written out with its citation in the Voter inventory, below.

---

## Voter inventory

23 voter classes: the 11 `architect-architecture.md`, "Authorization"
(lines 468-471) already names, the 10 `specs/api-designer-spec.md`, "Gates
vs. voters" (lines 88-99) adds, and the 2 its Decisions table splits out of
`EventVoter` (line 1131). One table per module, in module-map order. `—`
in the Subject column means a class-level check with nothing to load
(`specs/api-designer-spec.md:71-75`).

### Platform module

| Voter | Attributes | Subject | Enforces |
|---|---|---|---|
| `ImpersonationVoter` | `IMPERSONATION_START` (new), `IMPERSONATION_END` (new) | `Account` / `—` | BR-01-21/22, AC-01-33..38 |
| `PlatformConfigurationVoter` | `PLATFORM_CONFIG_EDIT` (new) | `PlatformConfiguration` \| `FeatureToggle` | BR-06-4, AC-06-29..31, BR-07-1..3, AC-07-18..21 |
| `AuditVoter` | `AUDIT_LOG_VIEW` | `—` | AC-07-29..34, BR-07-4..6 |

- `PlatformConfigurationVoter` supports two subject classes owned by two
  different modules in the Entity population table — `PlatformConfiguration`
  is `Platform` (`architect-architecture.md:357`), `FeatureToggle` is
  `Administration` (line 358). A voter class has one namespace; this design
  places the class in `Platform`, since `Platform` already owns one of the
  two entities outright and both subjects are identically shaped (global,
  Super-Admin-only, non-sensitive configuration). `specs/api-designer-spec.md`
  names the voter and the attribute but never a module for it — recorded in
  Decisions rather than silently resolved.
- Both voters here protect **global** subjects only — no tenant to leak, no
  `AdministrativeScope` needed for either, consistent with why
  `FeatureToggle`/`PlatformConfiguration` were placed global to begin with
  (Decisions row "Feature toggles placement," line 702).
- `ImpersonationVoter::IMPERSONATION_START`'s `Account` subject is the
  **target**, never the actor — the actor's `ROLE_SUPER_ADMIN` is already
  proven by the coarse gate on `administration_impersonation_start`.

### Identity module

| Voter | Attributes | Subject | Enforces |
|---|---|---|---|
| `AccountVoter` | `ACCOUNT_EDIT`, `ACCOUNT_VIEW`, `ACCOUNT_CREATE_TRAINER` (new), `ACCOUNT_DEACTIVATE` (new), `ACCOUNT_REACTIVATE` (new), `ACCOUNT_DELETE_GDPR` (new), `EMAIL_VERIFY_RESEND` | `Account` | BR-01-13, AC-01-48..59, AC-01-71/72, AC-07-13..17 |
| `ChildProfileVoter` | `CHILD_PROFILE_CREATE` (new), `CHILD_PROFILE_EDIT`, `CHILD_TRAINER_ADD`, `CHILD_TRAINER_REMOVE`, `CHILD_TOKEN_APPROVAL_EDIT`, `CHILD_PROFILE_VIEW` | `—` / `PlayerProfile` | AC-01-16..24, AC-01-27/28, AC-01-30 |
| `ShareLinkVoter` | `SHARELINK_RESOLVE` (new), `SHARELINK_CREATE` (new) | `ShareLink` / `—` | BR-01-14/15/27, AC-01-9..14, AC-01-31, AC-01-39..42, AC-03-50..52 |
| `AvailabilityVoter` | `AVAILABILITY_EDIT` (new) | `—` | BR-01-25/26, AC-01-43..47 |
| `ChildApprovalVoter` | `CHILD_APPROVAL_DECIDE` (new), `CHILD_APPROVAL_BYPASS` (new — this document) | `ChildApprovalRequest` / `ChildActionAttempt` (new — this document) | BR-01-18/19/20, BR-02-10, AC-01-25..29 |
| `CoachMembershipVoter` | `COACH_INVITE` (new) | `—` / `CoachMembership` | BR-01-15, AC-01-39, AC-01-42 |

- `AccountVoter::ACCOUNT_EDIT`/`ACCOUNT_VIEW` carry a bare `ROLE_SUPER_ADMIN`
  clause with no scope — `Account` is global (`architect-architecture.md:345`),
  so there is no tenant to adopt. `ACCOUNT_CREATE_TRAINER`,
  `ACCOUNT_DEACTIVATE`, `ACCOUNT_REACTIVATE`, `ACCOUNT_DELETE_GDPR` are
  `ROLE_SUPER_ADMIN`-only outright — no self-service branch exists.
- `ChildProfileVoter` denies a child login on **every** attribute in this
  row (`specs/api-designer-spec.md:423-425`) — this is the voter enforcing
  AC-01-30's "cannot... change trainer associations" from the child's side,
  distinct from `ChildApprovalVoter`, which governs the child's
  *purchase/RSVP* requests.
- `ShareLinkVoter::SHARELINK_RESOLVE` is the voter the council verdict
  names directly (`specs/council-sharelink-tenant-resolution.md:246-254`;
  `architect-architecture.md:183`): it runs only once `TenantResolver`
  source 5 has already set a genuine `TenantContext` from the code, and it
  denies an inactive/expired/exhausted/deactivated-trainer `ShareLink`
  under that fully-resolved tenant — it never decides tenancy itself.
- `CoachMembershipVoter::COACH_INVITE` governs the trainer's act of
  inviting; it does **not** enforce BR-01-11's exclusivity — see
  Coach-trainer exclusivity, below, for why that is `MembershipService`'s
  domain rule, not this voter's.

### Scheduling module

| Voter | Attributes | Subject | Enforces |
|---|---|---|---|
| `EventVoter` | `EVENT_CREATE`, `EVENT_VIEW`, `EVENT_EDIT`, `EVENT_DUPLICATE`, `EVENT_CANCEL`, `EVENT_VIEW_RSVP_LIST`, `EVENT_EXPORT_RSVPS`, `EVENT_MANUAL_ADD_PLAYER` | `—` / `Event` | BR-02-1..6/12/13/19, AC-02-1..17, AC-02-43..54 |
| `RsvpVoter` | `RSVP_CREATE`, `RSVP_CANCEL`, `RSVP_REMOVE` (new) | `Event` / `Rsvp` | BR-02-7..11, AC-02-23..33 |
| `AttendanceVoter` | `ATTENDANCE_RECORD` | `Event` | BR-02-16..18, AC-02-38..42 |
| `CoachAssignmentVoter` | `COACH_ASSIGNMENT_CONFIRM`, `COACH_ASSIGNMENT_DECLINE` | `CoachAssignment` | BR-02-13/14/15, AC-02-35/36 |

- `EventVoter::EVENT_EDIT`/`EVENT_CANCEL`/`EVENT_VIEW_RSVP_LIST` each carry
  `+ SUPER_ADMIN via AdministrativeScope` (`specs/api-designer-spec.md:698-700`)
  — Event Master "edit[s] the event as if they had created it" (AC-07-25,
  `specs/requirements-analyst-epic-07-super-admin-spec.md:261`) and
  silently overrides scheduling-conflict warnings (AC-07-27/28, lines
  270-276) — the override itself, not the conflict, is what gets
  audit-logged.
- `AttendanceVoter`/`CoachAssignmentVoter` were deliberately split out of
  `EventVoter` (`specs/api-designer-spec.md:1131`) because a
  `CoachAssignment` and an in-progress attendance take are distinct subject
  types from `Event`, even though both are event-adjacent — folding them
  in would widen `EventVoter::supports()` past one subject class.
- `AttendanceVoter`'s trainer branch is "any time, overrides the coach's
  entries" (BR-02-18); the coach branch adds a state check (event started,
  same-day-only until midnight) and a `CoachVisibilityService` reach check.
  No `AdministrativeScope` clause is named for Super Admin here — Epic-02
  states only that "the trainer and Super Admin can edit at any time"
  (BR-02-18, `specs/requirements-analyst-epic-02-event-management-spec.md:329`)
  without naming a mechanism; treated the same as the CRM Master gap below
  — see Open questions.

### Crm module

| Voter | Attributes | Subject | Enforces |
|---|---|---|---|
| `PlayerVoter` | `PLAYER_VIEW`, `PLAYER_EDIT`, `PLAYER_LABEL_MANAGE`, `PLAYER_FLAG_MANAGE`, `PLAYER_NOTE_MANAGE`, `PLAYER_FEEDBACK_ADD`, `PLAYER_FEEDBACK_EDIT` | `PlayerTrainerMembership` | BR-03-1/2/6/7/10/11/12, AC-03-1..49, AC-03-64/65 |
| `LabelVoter` | `LABEL_MANAGE` (new) | `—` / `Label` | BR-03-3/4/5, AC-03-11..13 |

- `PLAYER_VIEW`, `PLAYER_FLAG_MANAGE`, `PLAYER_NOTE_MANAGE`,
  `PLAYER_FEEDBACK_EDIT` each grant a Super Admin override the source
  states plainly: AC-03-56/58
  (`specs/requirements-analyst-epic-03-crm-players-spec.md:266-268`,
  "apply flags across trainers and view a player's history across
  trainers"); AC-03-49, line 255, "Super Admin can edit or delete any
  feedback at any time"; BR-03-12, line 319, "Super Admin can edit any note
  at any time." But **no route in `specs/api-designer-spec.md`'s Crm
  module table implements CRM Master**, despite its own Decisions table
  committing to one ("CRM Master... stays in its own epic's module,
  `/super-admin/...`-gated," line 1122). This design writes the Super
  Admin clause into each attribute's logic — the capability is settled —
  but cannot state whether it gates on `AdministrativeScope` or on
  impersonation, because no route names the mechanism the way Event Master
  and Forms each do. Carried to Open questions.
- `LABEL_MANAGE` gets **no** Super Admin clause anywhere, on any branch —
  the one deliberate, sourced *prohibition* in this inventory, not merely
  an unstated mechanism: AC-03-58 states Super Admin "cannot edit
  trainer-specific labels, respecting each trainer's own customization"
  (line 268). Contrast this against the CRM Master rows above, where the
  capability is granted but the mechanism is merely unstated — two
  different flavors of "no drawn route," kept distinct rather than
  conflated.
- `PLAYER_FLAG_MANAGE`'s coach branch is the literal on/off switch for
  Q-03.05 (`specs/requirements-analyst-open-questions.md`, Section B, line
  190); `specs/api-designer-spec.md:539-546` already designed the route to
  be reachable from both consoles for exactly this reason. This design
  keeps the branch present but denying-by-default until Q-03.05 resolves,
  matching Section B's default ("Trainers only").
- Both voters' `CoachVisibilityService` delegation is covered in full under
  Coach-trainer exclusivity and coach reach, below, not restated here.

### Content module

| Voter | Attributes | Subject | Enforces |
|---|---|---|---|
| `PlaylistVoter` | `PLAYLIST_CREATE`, `PLAYLIST_VIEW`, `PLAYLIST_EDIT`, `PLAYLIST_PUBLISH_TOGGLE`, `PLAYLIST_DELETE`, `PLAYLIST_ASSIGN`, `PLAYLIST_PURCHASE` | `—` / `Playlist` | BR-04-3..5/9/12..15/17, A8, A9, AC-04-1..23, AC-04-31..36, AC-04-41 |
| `ContentItemVoter` | `CONTENT_ITEM_CREATE`, `CONTENT_ITEM_VIEW`, `CONTENT_ITEM_EDIT`, `CONTENT_ITEM_DELETE`, `CONTENT_ITEM_PUBLISH_TOGGLE` | `—` / `ContentItem` | BR-04-1/2/18/19, AC-04-4..8, AC-04-24/25, AC-04-35 |

**The paywall:**

- `PLAYLIST_VIEW` always grants for a trainer-visible playlist — a player
  can see the locked teaser and price (AC-04-21, "unpurchased, with a
  locked indicator"). It is `CONTENT_ITEM_VIEW` that denies actual
  playback while the owning `Playlist` is locked and unpurchased
  (`specs/api-designer-spec.md:585`). The lock state is read from
  `PlaylistAccessGrant` (A8's one-time-purchase record,
  `architect-architecture.md:392`), never re-derived by the voter from
  `TokenEntry`/`PaymentRecord` directly.
- `PLAYLIST_PURCHASE`'s child-login branch routes through
  `ChildApprovalVoter::CHILD_APPROVAL_BYPASS` exactly as in Parent-child
  approval, below — a paywalled playlist is always paid, so every child
  attempt is either bypassed (token, toggle ON) or queued; the "free"
  carve-out never applies here.
- **Three-state visibility** (A9, `specs/requirements-analyst-open-questions.md:39`)
  is stored as two orthogonal facts — publication and in-tenant audience
  (`architect-architecture.md:426-430`) — and `PLAYLIST_PUBLISH_TOGGLE` is
  the single attribute covering both, taking a `PlaylistVisibilityType`
  with two fields rather than needing a third route or attribute
  (`specs/api-designer-spec.md:571`).
- **The publication exception is a read-predicate widening, never a write
  one.** `CONTENT_ITEM_VIEW`/`PLAYLIST_VIEW` grant cross-tenant for
  anything ever published (`architect-architecture.md:409-424`,
  "visibility governs discovery; publication governs readability");
  `_EDIT`/`_DELETE`/`_PUBLISH_TOGGLE`/`_ASSIGN` never do, regardless of
  publication state. Ownership of a public item never transfers, only its
  readability widens — see the test matrix, below.
- Neither voter carries a Super Admin clause on any attribute —
  `content_super_admin_analytics` (`specs/api-designer-spec.md:588`) reads
  through `CrossTenantReadService`, not through either voter, and no route
  edits an individual trainer's playlist or drill as Super Admin. Unlike
  the CRM Master gap, no AC in Epic-04 states such a capability should
  exist, so this is a natural absence, not an unresolved mechanism.
- **Feature gate:** `architect-architecture.md:452` names `FeatureGate` as
  evaluating "LPPP" alongside Marketing and Camps, and BR-07-1
  (`specs/requirements-analyst-epic-07-super-admin-spec.md:364-370`)
  states disabling LPPP "removes trainer access to the LPPP section and
  players see no content" — so every attribute on both voters here ANDs a
  `FeatureGate::isEnabled($trainer, 'lppp')` check onto its ownership/role
  logic. `specs/api-designer-spec.md`'s Content module route table does
  not repeat the "(feature toggle)" annotation the way its Growth and
  Forms tables do on their first row (contrast line 655 and line 727
  against Content's silence); this design treats that as the table's
  terseness, not as LPPP being exempt, since BR-07-1 states the opposite
  outcome explicitly. Recorded in Decisions.

### Billing module

| Voter | Attributes | Subject | Enforces |
|---|---|---|---|
| `TokenVoter` | `TOKEN_GIFT`, `TOKEN_PURCHASE`, `TOKEN_SUBSCRIPTION_PURCHASE` (new attribute, same voter) | `PlayerTrainerMembership` / `—` | BR-05-1..5/14/15, AC-05-4..9, AC-05-29/30/32/33 |
| `TrainerSettingsVoter` | `TRAINER_SETTINGS_VIEW`, `TRAINER_SETTINGS_EDIT`, `TRAINER_FEE_EDIT` (new) | `TrainerBillingSettings` / `TrainerBrandingSettings` / `Trainer` | AC-01-60..63, AC-05-1..3/25..28/32, BR-05-1 |
| `PaymentMethodVoter` | `PAYMENT_METHOD_MANAGE` (new) | `—` | AC-01-30, AC-05-20/21, BR-05-16 |

- `TOKEN_GIFT` denies `ROLE_COACH` outright, unconditionally — "this
  permission belongs to trainers only, not coaches" (AC-05-33,
  `specs/requirements-analyst-epic-05-payments-tokens-spec.md:219`). Stated
  as flatly as `LABEL_MANAGE`'s Super-Admin exclusion, just for a
  different role — both are reminders that "explicit" in this design's
  Super-Admin rule cuts for every role, not only the top one.
- `TOKEN_PURCHASE`/`TOKEN_SUBSCRIPTION_PURCHASE`'s child-login branch
  routes through `ChildApprovalVoter::CHILD_APPROVAL_BYPASS`. A
  subscription purchase is always Stripe/USD (BR-05-14, "payment is
  processed via Stripe"), so it always lands on the unconditional-deny USD
  row of that table when attempted by a child directly — no subscription
  bypass exists, by construction, not as a special case.
- `TrainerSettingsVoter` spans three subject classes across two module
  homes in the Entity population table — `TrainerBillingSettings` is
  `Billing` (`architect-architecture.md:368`); `TrainerBrandingSettings`
  and `Trainer` are `Platform` (lines 351-352). `specs/api-designer-spec.md`
  already committed to one voter class and one attribute
  (`TRAINER_SETTINGS_EDIT`) covering both the billing-settings and
  branding-settings routes (compare line 457 against line 616); this
  design keeps that vocabulary and places the class in `Billing`, since two
  of its three attributes are money-shaped — recorded in Decisions, not
  re-litigated as a rename.
- `TRAINER_FEE_EDIT` is the one attribute here that opens
  `AdministrativeScope` (`specs/api-designer-spec.md:695`) —
  `TrainerBillingSettings` **is** trainer-scoped, unlike the
  branding/global case, so the Super-Admin branch must open the scope for
  that specific trainer before the row is even reachable.
- `PAYMENT_METHOD_MANAGE` denies a child login "outright"
  (`specs/api-designer-spec.md:629`) — unlike `TOKEN_PURCHASE`, there is no
  bypass branch at all, for any funding method or toggle state, because
  AC-01-30 lists "add/remove payment methods" as something a child
  categorically cannot do (line 188), full stop, not something that
  becomes pending. `StripeCustomerLink` being global (shared across all of
  a parent's trainers, BR-05-16) means this voter's subject carries no
  tenant either — the interesting boundary here is the actor (never the
  child), not the trainer.

### Growth module

| Voter | Attributes | Subject | Enforces |
|---|---|---|---|
| `CouponVoter` | `COUPON_CREATE`, `COUPON_EDIT`, `COUPON_DEACTIVATE`, `COUPON_DELETE`, `COUPON_VIEW_ANALYTICS`, `COUPON_APPLY` (new) | `—` / `Coupon` | BR-06-8..11, AC-06-17..27 |

- `COUPON_APPLY`'s eligibility clause is designed to accept a rule without
  yet specifying it, pending Q-06.10
  (`specs/requirements-analyst-open-questions.md`, Section B, line 197),
  matching `specs/api-designer-spec.md:662`'s own framing. Section B's
  default is "per-coupon setting."
- BR-06-11 (`specs/requirements-analyst-epic-06-marketing-growth-spec.md:231`)
  states the cross-tenant rule in the source's own words: "A coupon
  created by Trainer A only works for Trainer A's own events/content;
  players cannot redeem Trainer A's coupon against Trainer B's events or
  content" — the most explicit cross-tenant citation of any business rule
  in this inventory, and the anchor for this voter's row in the test
  matrix, below.
- No route or AC names a Super Admin edit of an individual `Coupon` — the
  only Super-Admin-reachable Growth screen is the platform-wide referral
  rule, which is `PlatformConfigurationVoter::PLATFORM_CONFIG_EDIT`, a
  different voter over a different (global) subject entirely. `CouponVoter`
  carries no Super Admin clause on any attribute, for the same "natural
  absence" reason as `PlaylistVoter`/`ContentItemVoter`.

### Forms module

| Voter | Attributes | Subject | Enforces |
|---|---|---|---|
| `FormVoter` | `FORM_CREATE`, `FORM_EDIT`, `FORM_TOGGLE`, `FORM_DELETE`, `FORM_VIEW_SUBMISSIONS`, `FORM_EXPORT`, `FORM_MARK_ATTENDANCE`, `FORM_SEND_BULK_EMAIL` | `—` / `Form` | BR-08-1..5/19/20, AC-08-1..32 |
| `FormSubmissionVoter` | `FORM_SUBMISSION_CREATE` (new), `FORM_SUBMISSION_CONVERT` (new) | `Form` / `FormSubmission` | BR-08-6..18, AC-08-13..26 |

- `FormVoter` denies `ROLE_COACH` on every attribute, unconditionally —
  BR-08-20 (`specs/requirements-analyst-epic-08-forms-registration-spec.md:217`):
  "Coaches cannot create or manage forms; this is a trainer-only feature."
  No coach branch exists anywhere on this voter, unlike
  `PlayerVoter::PLAYER_FLAG_MANAGE`'s conditional one.
- `FormVoter` carries **no** `ROLE_SUPER_ADMIN` clause at all, on any
  attribute — the one voter in this inventory where that absence is fully
  settled rather than a gap. BR-08-21 (line 218) states the mechanism
  explicitly: "Super Admin can view all forms across all trainers, via
  impersonation mode." Under impersonation `getUser()` is the trainer, so
  the ordinary trainer-ownership check already grants — adding a
  `ROLE_SUPER_ADMIN` branch here would be redundant at best and, at worst,
  would accidentally grant `FORM_EDIT` to a Super Admin who merely opened
  an `AdministrativeScope` (fee edits, Event Master) without ever
  impersonating anyone, which BR-08-21 does not authorize. See Super
  Admin, below.
- `FORM_SUBMISSION_CREATE`'s subject is `Form`, and — because the actor is
  always anonymous on this attribute — the voter "evaluates form state
  only (exists, active, under capacity), never actor identity, since there
  is no actor" (`specs/api-designer-spec.md:750`). The one voter/attribute
  pair in this inventory whose `voteOnAttribute()` never inspects the
  token at all.
- `FORM_SUBMISSION_CONVERT`'s subject is `FormSubmission`; it "checks the
  submission belongs to this form/code and is not already converted"
  (`specs/api-designer-spec.md:753`) — an ownership check against the
  *code* still active in the request (source 5's tenant), not against any
  authenticated actor, since conversion can be the moment the actor is
  first authenticated.

---

## Parent-child approval

Governs A1 ("all under-18 players require a parent-managed account,"
`specs/requirements-analyst-open-questions.md:31`), BR-01-17/18/19 and
BR-02-10.

### Where the check lives

Two distinct voter-level questions, kept apart because they answer
different things:

1. **"May this actor attempt this action at all?"** — the ordinary
   object-level question, answered by the action's own voter:
   `RsvpVoter::RSVP_CREATE`/`RSVP_CANCEL`,
   `TokenVoter::TOKEN_PURCHASE`/`TOKEN_SUBSCRIPTION_PURCHASE`,
   `PlaylistVoter::PLAYLIST_PURCHASE`. These check capacity, eligibility,
   ownership, duplicate-RSVP, price rules — everything unrelated to who is
   a minor.
2. **"Does this specific attempt execute now, or wait for a parent?"** — a
   narrower question, not itself a grant/deny of the underlying action (a
   child genuinely **can** RSVP — AC-01-29 says so explicitly), but a fork
   in how it completes. This is `ChildApprovalVoter::CHILD_APPROVAL_BYPASS`,
   an attribute this document adds because the architecture names the
   *concept* without naming the attribute: "Child approval is a voter
   attribute on the RSVP and purchase actions, consulting the approval
   state and the per-child... setting" (`architect-architecture.md:476-478`).

`CHILD_APPROVAL_BYPASS` is consulted by the owning workflow service —
`RsvpToEventService`, the token/subscription purchase flow,
`PurchasePlaylistAccessService` — **after** its own voter has already
granted step 1, never before. Its subject is a small typed value object,
not the bare `PlayerProfile`, because the bypass rule depends on *how* the
action is funded, and that cannot be inferred from the profile alone:

```
final class ChildActionAttempt {
    public function __construct(
        public readonly Account $actor,
        public readonly PlayerProfile $beneficiary,
        public readonly FundingMethod $fundingMethod, // Free | Usd | Token
    ) {}
}
```

Vote logic, cited against the rule that produces each branch:

| Condition | Result | Source |
|---|---|---|
| `beneficiary` is not a child | Bypass **granted** — nothing to approve | BR-01-17 scopes the whole workflow to minors |
| `actor` is the beneficiary's own parent (via `ParentChildLink`, matched against `beneficiary`) | Bypass **granted** — the parent's own action already *is* the approval | `architect-architecture.md:479-480`, "A parent acting for a child is authorized through `ParentChildLink`, in a voter, never by comparing identifiers in a controller" |
| `actor` is the child's own `Account`, `fundingMethod` is `Usd` | Bypass **denied**, unconditionally — no toggle reaches USD | BR-01-18, "USD payments always require parent approval" |
| `actor` is the child's own `Account`, `fundingMethod` is `Free` | Bypass **denied** | BR-02-10, "Child accounts require parent approval for all RSVPs (free or paid)" — free RSVPs are not exempted |
| `actor` is the child's own `Account`, `fundingMethod` is `Token`, per-child toggle OFF (default) | Bypass **denied** | BR-01-19; AC-01-27, default OFF |
| `actor` is the child's own `Account`, `fundingMethod` is `Token`, toggle ON | Bypass **granted** — executes immediately, parent gets an informational notice only | AC-01-27, "when ON, the child's token purchase and event registration are processed immediately" |

Denial does not stop the request — it redirects the workflow to create a
`ChildApprovalRequest` (BR-01-18's "Pending Parent Approval" state)
instead of executing the spend/RSVP. This is the one place in this design
where a voter's DENY does not mean `403`; the calling service branches on
the grant rather than throwing on the deny, exactly as
`scheduling_portal_event_rsvp`'s own response contract states ("Child:
redirect to a 'Pending Parent Approval' page," `specs/api-designer-spec.md:507`).

### Composition with the RSVP and purchase voters

Inside `RsvpToEventService::rsvp()` (illustrative — not literal code the
epics specify):

1. Controller calls `denyAccessUnlessGranted(RsvpVoter::RSVP_CREATE, $event)`.
   Denied → `403`, the request never reaches the service.
2. Service asks `isGranted(ChildApprovalVoter::CHILD_APPROVAL_BYPASS, $attempt)`.
   - Granted → confirm the RSVP (or take payment) as normal.
   - Denied → create the `ChildApprovalRequest`, leave the RSVP
     unconfirmed, notify the parent.

The same two-step shape governs `scheduling_portal_rsvp_cancel` (AC-02-33,
"a child's cancellation routes through `ChildApprovalVoter` the same as a
purchase," `specs/api-designer-spec.md:509`), `content_portal_playlist_checkout`,
and `billing_portal_tokens_purchase`/`billing_portal_subscription_purchase`.

### `ChildApprovalVoter::CHILD_APPROVAL_DECIDE`

The voter's second, unrelated attribute: the parent's own act of approving
or denying a pending request (`identity_portal_approval_approve`/`_deny`,
`specs/api-designer-spec.md:442-443`). Subject: `ChildApprovalRequest`.

- **Grant**: actor is the parent named via `ParentChildLink` on the
  request's beneficiary, **and** the request is still within its 48-hour
  window.
- **Deny**: any other actor; or the right parent, but the 48-hour window
  has elapsed. The architecture computes time-based state at read time,
  never by a job (Decisions row "Time-based state," line 714; restated for
  this exact screen at `specs/api-designer-spec.md:441`, "expired rows show
  'Expired' — computed... at read time, not a job"). A decision voter that
  still granted after the 48-hour mark would let a late parent override an
  outcome BR-01-18 already auto-settled, so `CHILD_APPROVAL_DECIDE`
  independently applies the same cutoff, not only the read-side display —
  this document's own inference, not a verbatim rule (see Decisions).

### Distinguishing a child's own request from a parent acting for the child

Not by which `PlayerProfile` is the beneficiary — a parent and their child
can both target the same profile. By **which `Account` `getUser()`
returns**:

- `getUser()` returns the **child's own `Account`** (AC-01-20's "separate
  login") → the child's own request. `ChildApprovalVoter` always evaluates
  the child-branch rules above.
- `getUser()` returns the **parent's `Account`**, with the child selected
  through `identity_portal_context_child_switch`
  (`ChildProfileVoter::CHILD_PROFILE_VIEW`, `specs/api-designer-spec.md:435`)
  → a parent acting for the child. The `ParentChildLink` match in the
  bypass table's second row grants immediately; no `ChildApprovalRequest`
  is ever created, because there is no third party left to approve it.

This is exactly the distinction the architecture requires be drawn in a
voter and never "by comparing identifiers in a controller"
(`architect-architecture.md:479-480`): the comparison is `getUser()`'s
`Account` identity against the two sides of `ParentChildLink`, not a raw
`PlayerProfile` id equality check.

---

## Coach-trainer exclusivity and coach reach

Two different rules, easy to conflate: **exclusivity** is which trainer a
coach may belong to at all (a membership-uniqueness invariant); **reach**
is which players and events a coach — already legitimately at one trainer
— may see inside that tenant.

### Exclusivity (BR-01-11) is a domain rule, not a voter decision

"Coaches can work for only ONE trainer at a time (strictly enforced)"
(BR-01-11, `specs/requirements-analyst-epic-01-user-management-spec.md:277`;
AC-01-41, line 203, "if the coach already exists and is active elsewhere
the system shows an error rather than allowing a second active trainer").
This is enforced where the second `CoachMembership` would be created —
inside `MembershipService`, `Identity`'s own exclusive creator
(`architect-architecture.md:447`) — not inside a voter.

The council's own resolution of the adjacent ShareLink question states the
placement explicitly: "Coach already active under another trainer |
Tenant resolves; `CoachMembership` creation is refused by `Identity`'s own
rule (BR-01-11, AC-01-41...) — **a domain refusal, not a tenancy one**"
(`specs/council-sharelink-tenant-resolution.md:292`). No voter attribute in
this design is named for it, because there is nothing to authorize: a
coach accepting a second trainer's invite is not "unauthorized," they are
attempting an operation the domain forbids from existing at all —
symmetric with a duplicate email being rejected by a uniqueness rule, not
a voter. `CoachMembershipVoter::COACH_INVITE` (the trainer's side —
inviting a coach at all) is a genuine voter decision; the invited coach's
eventual acceptance is not.

### Reach: `CoachVisibilityService`, consumed, never restated

"A voter restating a visibility rule is a defect, not a style choice"
(`architect-architecture.md:112-114`). `CoachVisibilityService` "produce[s]
the coach's reachable-event and reachable-player constraint from
assignments" and is the required consumer for "every coach-facing read in
`Crm`, `Scheduling`, `Content`; every coach voter" (Cross-cutting services
table, line 446).

Concretely, in this inventory:

- `PlayerVoter::PLAYER_VIEW`, `PLAYER_FEEDBACK_ADD`, `PLAYER_FEEDBACK_EDIT`
  (coach branch) — grant only if the target `PlayerTrainerMembership` is
  inside `CoachVisibilityService`'s reachable-player set for the acting
  coach. They do **not** re-derive "has this coach shared a session with
  this player" from `AttendanceRecord`/`Rsvp` themselves.
- `AttendanceVoter::ATTENDANCE_RECORD` (coach branch) — grants only if the
  `Event` is inside the coach's reachable-event set, in addition to the
  same-day-only state check (BR-02-18).
- `EventVoter` — the coach-facing read (`scheduling_coach_activities`) is
  not gated by a voter at all (`—`, `specs/api-designer-spec.md:486`)
  precisely because the query itself is scoped by `CoachVisibilityService`
  — the pattern the architecture calls out for "Subject: —" rows
  (`specs/api-designer-spec.md:74-75`, "the query itself is scoped to the
  current actor").
- `CoachAssignmentVoter` — does not consult `CoachVisibilityService`; a
  coach confirming/declining their own assignment is a direct ownership
  check against `CoachAssignment.coach`, a stronger and simpler guarantee
  than reachability (the assignment names the coach directly).

The invariant this buys: "a coach assigned to zero events sees zero
players" (AC-03-64, `specs/requirements-analyst-epic-03-crm-players-spec.md:278`;
restated `architect-architecture.md:246`) is true **once**, inside
`CoachVisibilityService`, and every voter and read that needs it inherits
the same answer. If the reachability rule ever changes, it changes in one
service, not in four voters that would otherwise drift apart.

**The one deliberately unreachable case.** "A coach viewing a player with
no shared session history" has no stated expected behavior in Epic-03's
own Testing Considerations (`architect-architecture.md`, Open architecture
risk 6, lines 802-805). The architecture's own assumption — not reachable
at all, `CoachVisibilityService` yields no such player, so the route
denies rather than rendering an empty profile — is the one this design's
`PlayerVoter` coach branch follows, cited at
`specs/requirements-analyst-epic-03-crm-players-spec.md:248` (AC-03-44's
note) and `architect-architecture.md:802-805`.

**Coach-authored writes stay narrower than reach.** Reach answers "can the
coach see this player at all"; it does not answer "can the coach change
this player." AC-03-45
(`specs/requirements-analyst-epic-03-crm-players-spec.md:249`) is explicit
that even inside reach, a coach "cannot edit player profiles except
feedback... cannot remove labels or flags." `PlayerVoter::PLAYER_LABEL_MANAGE`
and `PLAYER_FLAG_MANAGE`'s coach branch (the latter conditional on
Q-03.05) therefore layers an *additional* role check on top of
`CoachVisibilityService`'s reach check — reach is necessary, never
sufficient, for a coach-authored write.

**Coach-invited players stay reach-gated too.** AC-03-53
(`specs/requirements-analyst-epic-03-crm-players-spec.md:261`): a coach
"can only view invited players once they appear in the coach's own
assigned sessions" — issuing a `ShareLink` invite
(`ShareLinkVoter::SHARELINK_CREATE`) does not itself grant `PLAYER_VIEW`;
the invited player becomes visible only once an assignment puts them
inside `CoachVisibilityService`'s reachable set. Two different voters, two
different subjects, no shortcut between them.

---

## Super Admin: impersonation and administrative tenant scope

Both are named in `architect-architecture.md`'s "Writing across tenants"
(lines 225-238) and "Cross-cutting services" (line 445): "Super Admin
never writes cross-tenant. Two mechanisms, both of which set a genuine
tenant context before any write" (line 227). They permit different things,
and a voter tells them apart by checking for structurally different
evidence — never by asking merely "is this Super Admin," which both
mechanisms share.

### What each permits

| | Impersonation | Administrative tenant scope |
|---|---|---|
| Duration | A whole browsing session, up to 1 hour, until exit (BR-01-22) | One HTTP request/response cycle only (`specs/api-designer-spec.md:154-164`) |
| Identity | `getUser()` returns the **target** — Super Admin's own roles are gone for the duration | `getUser()` still returns the **Super Admin** — no identity change |
| Reachability | Everything the target's own role reaches (`/trainer/...`, `/coach/...`, `/portal/...`), and **nothing** `/super-admin/...`-gated, since the effective role is the target's and no hierarchy restores `ROLE_SUPER_ADMIN` (`specs/api-designer-spec.md:943-947`) | Only the two named call sites: Event Master's per-event write routes, and the per-trainer fee edit (`architect-architecture.md:445`, "Administration (Event Master, fee edits)") |
| Named callers | `administration_impersonation_start` (BR-01-21/22) | `administration_event_master_edit`/`_cancel`, `administration_trainer_fee_edit` |
| What it can never do | Target another Super Admin (BR-01-21; `ImpersonationVoter::IMPERSONATION_START` denies — AC-01-37) | Reach a trainer-scoped table without the scope open first; write on behalf of a specific human identity — there is none to assume (`architect-architecture.md:232-233`, "adopting a trainer's tenant **without becoming a user**") |
| Forbidden under both | Any write outside the tenant the mechanism just set — both set the *same* `TenantContext` session variable through the *same* path (`architect-architecture.md:308-313`); there is exactly one way to hold a trainer-scoped write lock, no matter which mechanism opened it |

### How a voter tells them apart

Not every voter needs to — only the ones that reuse a trainer-facing
attribute for a Super-Admin-facing route need an explicit clause, and the
clause's *shape* differs by which mechanism reaches it:

- **Under impersonation, most voters need no special clause at all.**
  `getUser()` genuinely is the trainer (or coach, or player), so
  `FormVoter::FORM_EDIT`'s ordinary "is the acting trainer this form's
  owning trainer" check passes on its own merits. This is why `FormVoter`
  — reached only via impersonation per BR-08-21
  (`specs/requirements-analyst-epic-08-forms-registration-spec.md:218`,
  "Super Admin can view all forms across all trainers, via impersonation
  mode") — carries **no** `ROLE_SUPER_ADMIN` clause anywhere in this
  design. Impersonation's whole purpose is to make the ordinary check
  sufficient.
- **Under an administrative tenant scope, the ordinary check is never
  sufficient**, because `getUser()` is still the Super Admin, who owns
  nothing. The voter needs an explicit second branch: `ROLE_SUPER_ADMIN`
  **and** `AdministrativeScope::isOpenFor($subject->getTrainer())`.
  `EventVoter::EVENT_EDIT` carries exactly this
  (`specs/api-designer-spec.md:698`, "`+ SUPER_ADMIN via AdministrativeScope`"),
  as does `TrainerSettingsVoter::TRAINER_FEE_EDIT` (line 695). The check is
  on the *specific* adopted trainer, not merely "some scope is open
  somewhere" — a scope opened for Trainer A must not satisfy a voter check
  against Trainer B's event, which is what a parameterized
  `AdministrativeScope::isOpenFor()` is for.
- **A bare `ROLE_SUPER_ADMIN`, with neither an active impersonation
  session nor an open scope, must be insufficient for every trainer-scoped
  subject.** This is the negative space that makes the two mechanisms
  exhaustive: per `architect-architecture.md:227`, "Super Admin never
  writes cross-tenant," full stop — a voter clause of the shape "if
  `ROLE_SUPER_ADMIN`, grant," with no scope/impersonation check, is a
  defect in this design, not a shortcut. It is precisely the failure mode
  the missing role hierarchy exists to prevent (lines 462-466),
  reintroduced by hand inside a voter instead of by a hierarchy edge.

### Attributability: the human behind the write

`AuditLogger`'s contract carries **two** identities whenever either
mechanism is active (`specs/api-designer-spec.md:978-986`), and a voter's
job ends before this point — attributability is a calling-convention rule
every mutating service must follow, not something the voter itself
decides:

| Field | Impersonation | Administrative scope |
|---|---|---|
| `actorAccountId` | The true Super Admin, recovered from `SwitchUserToken::getOriginalToken()` — **never** from `getUser()`, which returns the target | The Super Admin performing the write (same as `getUser()`, since identity never changed) |
| `actingAsAccountId` | The impersonated target | `null` |
| `impersonationSessionId` | The active `ImpersonationSession` | `null` |
| `administrativeScopeTrainerId` | `null` | The adopted trainer |

Once `switch_user` is active, `getUser()` **returns the target**, so any
code that logs "who did this" from `getUser()` naively attributes an
impersonated write to the impersonated trainer, not the Super Admin who
chose to make it — defeating BR-01-22's entire purpose ("impersonation
exists for support/troubleshooting" and must stay auditable to the real
actor). This is a service/`AuditLogger`-contract concern, not a new voter
attribute; it is recorded here because a voter that grants `EVENT_EDIT`
under impersonation is exactly the moment this contract must already be
satisfied by the caller.

### What remains forbidden under both

- Writing a trainer-scoped row for a trainer neither mechanism has set as
  the active `TenantContext` (`architect-architecture.md:227`, "Super
  Admin never writes cross-tenant").
- Impersonating, or opening a scope for, another `ROLE_SUPER_ADMIN`
  account in a way that bypasses `ImpersonationVoter::IMPERSONATION_START`'s
  own BR-01-21 clause — administrative scope has no equivalent "target"
  concept (it adopts a trainer, not an account), so this specific
  prohibition is impersonation-only, but the general "no cross-tenant
  write" prohibition binds both.
- Financial figures: `StripeReportingReader` is "the only source of any
  figure presented as earnings, payout or revenue... no fallback"
  (`architect-architecture.md:450`; Decisions row "Earnings figures," line
  722), and AC-07-7
  (`specs/requirements-analyst-epic-07-super-admin-spec.md:165-172`)
  forbids duplicating revenue/payout/transaction data anywhere on the
  platform — neither mechanism creates an exception. Super Admin's own
  dashboard link-outs to Stripe (`administration_stripe_dashboard_link`)
  exist precisely so no voter ever needs to gate a locally-stored money
  figure for Super Admin, because none exists to gate.
- `LabelVoter::LABEL_MANAGE` — see the Voter inventory: **neither**
  mechanism grants access here, by explicit design (AC-03-58), which is
  worth restating as the sharpest counterexample to "Super Admin can
  always reach a trainer-scoped row through one of the two mechanisms."

---

## `access_control` versus voters

"`access_control` is coarse-only... every object-level decision is a
voter" (`specs/api-designer-spec.md:68-70`, matching
`architect-architecture.md:472-473`).

### Where `access_control` is used

Six shapes, all role-prefix or literal gates:

| Prefix | Gate | `specs/api-designer-spec.md` |
|---|---|---|
| `/trainer/...` | `ROLE_TRAINER` | line 43 |
| `/coach/...` | `ROLE_COACH` | line 44 |
| `/portal/...` | `ROLE_PLAYER` | line 45 |
| `/super-admin/...` | `ROLE_SUPER_ADMIN` | line 46 |
| `/account/...`, `/context/...` | `IS_AUTHENTICATED_FULLY` | line 47 |
| everything else | `PUBLIC_ACCESS` (named routes only) | line 48 |

Plus two attribute-shaped but still coarse gates that are not
prefix-based: `IS_IMPERSONATOR` (Symfony's `switch_user` marker role,
gating `impersonation_exit` — line 691) and the dedicated stateless
`security: false` firewall for `^/webhooks/` (line 641), where
authorization is replaced entirely by Stripe's signature.

### Why the coarse gate is never the real check

Three concrete reasons, each with a case in this design that would break
if the coarse gate were trusted alone:

1. **It cannot see the object.** `/trainer/players/{membership}` is
   `ROLE_TRAINER`-gated — that only proves the caller runs *some* trainer
   account, not that `{membership}` belongs to *their* tenant.
   `PlayerVoter::PLAYER_VIEW` (backed by tenancy layers 2-5) is what
   proves that. Without it, any trainer could view any other trainer's
   roster by guessing a membership id — precisely the class of defect this
   whole document exists to prevent.
2. **It cannot see per-tenant state.** `access_control` has no concept of
   "does *this* trainer have Marketing enabled" — that is `FeatureToggle`
   row state, evaluated by `FeatureGate`, consulted inside `CouponVoter`,
   the Growth voters, `FormVoter`, and the Content voters, never
   expressible as a route prefix rule (`architect-architecture.md:474-475`,
   "Feature toggles are a voter attribute, not a controller `if`. A
   disabled feature is denied at the same place as any other permission").
3. **It cannot narrow a role to a subset of its own reach.** `/coach/...`
   proves `ROLE_COACH`; it cannot prove "assigned to this event."
   `CoachVisibilityService`, consumed by voters (see Coach reach, above),
   narrows the coarse grant down to the coach's actual assignments — the
   gate would happily let every coach at every trainer hit
   `/coach/players/{id}` for every id.

### The HTTP-status contract this produces

Because the coarse gate and the voter answer different questions, denial
at each layer means something different, following the contract
`specs/api-designer-spec.md`, "Error contract, CSRF, redirects,
pagination" already fixes (lines 242-276):

- **No session at all**, hitting an authenticated-only prefix → `302` to
  `/login` (HTML) or `401` (the JSON routes) — the coarse gate, before any
  voter runs.
- **Authenticated, but the voter denies** (wrong role for this object,
  wrong owner, expired state) → `403`, a static "Access Denied" page that
  "never names the resource, the trainer, or the reason"
  (`specs/api-designer-spec.md:256-258`).
- **Authenticated, voter would deny, but the row is in a different
  tenant** → structurally `404`, not `403` — the one place tenancy design
  overrides the ordinary contract. The voter frequently never runs at all,
  because the Doctrine filter (layer 2) and RLS (layer 5) already make the
  row invisible to the `#[MapEntity]` lookup that would have supplied the
  voter's subject (`specs/api-designer-spec.md:259-269`;
  `architect-architecture.md`, Layers 2 and 5). A same-tenant wrong-owner
  or wrong-role case (a coach outside their assigned events, a trainer on
  another trainer's *reachable* row) **is** found by the query and **is**
  the `403` case — the distinction is exactly "was the row visible to the
  query" versus "was the actor allowed to act on the row once found,"
  which is the same distinction this whole document draws between
  resolution and authorization (`architect-architecture.md:180-190`,
  "Resolution is not authorization").
- **Rate-limited** → `429`, generic, never confirms which identifier was
  throttled (`specs/api-designer-spec.md:273-275`).

None of these four outcomes is decided by `access_control`. Its only job
is to keep an unauthenticated or wrong-role request from reaching a
controller at all — the cheapest possible rejection, before a database
connection or a voter is even involved. Every one of the 23 voters in this
inventory exists because that job stops at the role boundary, not the
object boundary.

---

## Voter test matrix

Every row states the allow case, the deny case, and — the highest-priority
case for this product — the cross-tenant (or, for global subjects, the
nearest equivalent boundary) deny. "Structural" means the tenancy layers
(Doctrine filter, RLS, post-load assertion) make the foreign row invisible
before the voter runs at all, surfacing as `404`; "voter" means both rows
are visible to the actor by construction and only the voter's own logic
draws the line.

| Voter (attribute tested) | Allow | Deny | Cross-tenant / boundary deny |
|---|---|---|---|
| `ImpersonationVoter::IMPERSONATION_START` | Super Admin impersonates a Trainer/Coach/Player account | Super Admin targets **another Super Admin** — denied (BR-01-21, AC-01-37) | N/A — global subject. Substitute: a non-Super-Admin actor attempts `IMPERSONATION_START` — denied defensively at the voter even though the coarse gate should already stop it (defense in depth) |
| `PlatformConfigurationVoter::PLATFORM_CONFIG_EDIT` | Super Admin edits Trainer X's `FeatureToggle` | A Trainer attempts to edit their **own** `FeatureToggle` row — denied; the route and voter are `ROLE_SUPER_ADMIN`-only, no self-service branch exists | N/A — both subjects are global by design (Decisions row "Feature toggles placement," line 702). Substitute: a `ROLE_TRAINER` actor must be denied regardless of *which* trainer's `FeatureToggle` id is passed — Open architecture risk 8 (`architect-architecture.md:812-816`) names exactly this as the risk global placement carries: only the voter, not RLS, stands between a trainer and every other trainer's toggle state |
| `AuditVoter::AUDIT_LOG_VIEW` | Super Admin views the audit log | A Trainer, Coach or Player attempts `AUDIT_LOG_VIEW` — denied | N/A — `AuditLogEntry` is explicitly global and "spans tenants by design" (`architect-architecture.md:355`); this is deliberate (the log's purpose is platform-wide accountability), not an oversight, and is noted here so it is not mistaken for a missed case |
| `AccountVoter::ACCOUNT_EDIT` | Actor edits their own `Account` | Actor attempts to edit a different `Account` with no `ROLE_SUPER_ADMIN` — denied | N/A — `Account` is global. Substitute: a Trainer attempts `ACCOUNT_EDIT` on a Coach's `Account` (no ownership, no Super Admin) — denied |
| `ChildProfileVoter::CHILD_TRAINER_ADD` | Parent adds their own child to a new trainer via ShareLink code | The **child's own login** attempts `CHILD_TRAINER_ADD` — denied outright (AC-01-30) | Substitute (family boundary, `PlayerProfile`/`ParentChildLink` are global): Parent A attempts `CHILD_TRAINER_ADD` on a `PlayerProfile` id that is Parent B's child (no `ParentChildLink` edge) — denied |
| `ShareLinkVoter::SHARELINK_RESOLVE` | Code resolves to an active `ShareLink` under its `PublicTenantCode`-named trainer | Code resolves to an expired/exhausted/inactive `ShareLink` — denied, "this invitation has expired" | Structural, by drift: if a `PublicTenantCode` row ever pointed at a trainer other than its `ShareLink`'s own trainer, layer 4's post-load assertion throws on hydration before the voter runs — "a test that hydrates a foreign-tenant entity through a permitted path and asserts the assertion throws" (`architect-architecture.md`, Risks, "identity-map hole reopens," ~line 741); mirrors the council's own "mapping row survives a deleted `ShareLink`" failure mode (`council-sharelink-tenant-resolution.md:289`) |
| `AvailabilityVoter::AVAILABILITY_EDIT` | Player edits their own Best Times | A Coach attempts to edit a **player's** availability — denied, wrong subject/role entirely | `AvailabilityWindow` is trainer-scoped (Decisions row "Player availability," line 703) — a player's availability under Trainer A must be invisible/unwritable while the player's active tenant context is Trainer B |
| `ChildApprovalVoter::CHILD_APPROVAL_BYPASS` | Parent acting for their own child (context switch) — bypass granted, executes immediately | Child's own login, token spend, per-child toggle OFF — bypass denied, `ChildApprovalRequest` created instead | N/A — subject is global. Note: a stranger acting on a `PlayerProfile` they neither own nor parent is stopped earlier, by the base `RsvpVoter`/`TokenVoter` ownership check, before `CHILD_APPROVAL_BYPASS` is ever consulted |
| `ChildApprovalVoter::CHILD_APPROVAL_DECIDE` | The parent named on the request approves/denies it inside the 48-hour window | A different parent (no `ParentChildLink` to this child) attempts to decide it — denied | `ChildApprovalRequest` is trainer-scoped (`architect-architecture.md:374`) — parent's session context is Trainer A, request belongs to Trainer B (a different trainer relationship for the same child) — structurally invisible, `404` |
| `CoachMembershipVoter::COACH_INVITE` | Trainer invites a new coach | Super Admin, with no `AdministrativeScope` open, attempts `COACH_INVITE` — denied; no admin path is designed for coach invitation | `CoachMembership` (for the resend attribute) is trainer-scoped (`architect-architecture.md:369`) — Trainer A attempts to resend Trainer B's coach invite — structurally invisible, `404` |
| `EventVoter::EVENT_EDIT` | Trainer edits their own `Event` | A Coach assigned to the event attempts `EVENT_EDIT` — denied; editing is trainer/Super-Admin-only, never a coach capability | Trainer A attempts `EVENT_EDIT` on Trainer B's `Event` id — structurally invisible, `404`. Separately: Super Admin **without** an open `AdministrativeScope` attempts it — denied; the bare role is insufficient (see Super Admin, above) |
| `RsvpVoter::RSVP_CREATE` | Eligible adult player RSVPs to an eligible public `Event` within capacity | Player attempts to RSVP twice to the same event — denied, "already registered" (AC-02-28) | Player's active tenant context is Trainer A; `Event` id belongs to Trainer B (e.g. a stale link from a prior trainer context) — structurally invisible, `404` |
| `AttendanceVoter::ATTENDANCE_RECORD` | Coach assigned to the event records attendance same-day | The same coach attempts to edit attendance **the next day** — denied, read-only after midnight (BR-02-18); separately, a coach not assigned to the event — denied via `CoachVisibilityService` yielding no reachable event | A coach's own `CoachMembership` ties them to exactly one trainer (BR-01-11), so their tenant context can only ever be that trainer; a foreign-tenant `Event` id is structurally invisible regardless, `404` |
| `CoachAssignmentVoter::COACH_ASSIGNMENT_CONFIRM` | The assigned coach confirms their own Pending `CoachAssignment` | A different coach (not named on the assignment) attempts to confirm it — denied | `CoachAssignment` is trainer-scoped — a `CoachAssignment` id from a different trainer than the coach's own is structurally invisible, `404` |
| `PlayerVoter::PLAYER_VIEW` | Trainer views a `PlayerTrainerMembership` in their own roster | Coach attempts to view a player who has never shared a session with them — denied per Open architecture risk 6 (`architect-architecture.md:802-805`) | **The canonical case.** Trainer A attempts `PLAYER_VIEW` on a `PlayerTrainerMembership` id belonging to Trainer B's roster, for the *same underlying player* associated with both trainers (BR-03-2, multi-trainer model) — structurally invisible, `404`, exactly the shape BR-03-2 itself names: "each trainer sees only their own association with the player" |
| `LabelVoter::LABEL_MANAGE` | Trainer manages their own `Label` | **Super Admin** attempts `LABEL_MANAGE` — denied, even via `AdministrativeScope` (AC-03-58, "respecting each trainer's own customization") | Trainer A attempts `LABEL_MANAGE` on Trainer B's `Label` id — structurally invisible, `404` |
| `PlaylistVoter::PLAYLIST_EDIT` | Trainer edits their own `Playlist` | Player attempts `PLAYLIST_EDIT` (never a player capability at all) — denied | Trainer A attempts `PLAYLIST_EDIT` on a **private, never-published** `Playlist` owned by Trainer B — structurally invisible, `404`. Contrast: the same `Playlist`, once published, becomes `PLAYLIST_VIEW`-able cross-tenant by the publication exception, but **never** `PLAYLIST_EDIT`-able — the read predicate widens, the write predicate never does |
| `ContentItemVoter::CONTENT_ITEM_VIEW` | Player plays a `ContentItem` in an unlocked/purchased `Playlist` | Player attempts to play a `ContentItem` whose owning `Playlist` is locked/unpurchased — denied | Trainer A attempts `CONTENT_ITEM_EDIT` on a private `Drill` owned by Trainer B — structurally invisible, `404`. Contrast, as with Playlist: a published `Drill` is cross-tenant `_VIEW`-able, never cross-tenant `_EDIT`-able |
| `TokenVoter::TOKEN_GIFT` | Trainer gifts tokens to a player in their own roster | **Coach** attempts `TOKEN_GIFT` — denied outright, "trainers only, not coaches" (AC-05-33) | Trainer A attempts `TOKEN_GIFT` against a `PlayerTrainerMembership` belonging to Trainer B — structurally invisible, `404`. Separately, `TOKEN_PURCHASE`: parent in Trainer-A context attempts to spend against a `TokenBalance` scoped to Trainer B — denied, `TokenBalance` is keyed per (parent, trainer) (BR-05-15) |
| `TrainerSettingsVoter::TRAINER_SETTINGS_EDIT` | Trainer edits their own `TrainerBillingSettings`/`TrainerBrandingSettings` | A Coach attempts `TRAINER_SETTINGS_EDIT` — denied | Trainer A attempts `TRAINER_SETTINGS_EDIT` on Trainer B's `TrainerBillingSettings` id — structurally invisible, `404` ("Sensitive; never cross-tenant," `architect-architecture.md:368`). Separately, `TRAINER_FEE_EDIT`: Super Admin with `AdministrativeScope` open for Trainer A attempts the fee edit against Trainer B — denied; the clause checks the *specific* adopted trainer |
| `PaymentMethodVoter::PAYMENT_METHOD_MANAGE` | Adult player/parent manages their own payment methods (redirect to Stripe Customer Portal) | Unauthenticated request → `302`/`401`, not `403` (no session to evaluate); separately, a **child's own login** — denied outright, no bypass exists (AC-01-30) | N/A — `StripeCustomerLink` is global, deliberately shared across all of a parent's trainers (BR-05-16); the risk here is under-sharing, not leakage, so there is no tenant-boundary test to write |
| `CouponVoter::COUPON_APPLY` | Player applies an active, unexpired, under-limit `Coupon` belonging to their current trainer context | Coupon has reached its usage limit or is expired — denied, "Invalid or expired code" | Player in Trainer-A context attempts to apply a code created by Trainer B — denied, per BR-06-11's own words: "players cannot redeem Trainer A's coupon against Trainer B's events or content" (`specs/requirements-analyst-epic-06-marketing-growth-spec.md:231`) — the most explicit cross-tenant citation of any business rule in this design |
| `FormVoter::FORM_EDIT` | Trainer edits their own `Form` | **Coach** attempts `FORM_EDIT` — denied outright, "trainer-only feature" (BR-08-20); separately, Super Admin with an `AdministrativeScope` open (but **not** impersonating) attempts `FORM_EDIT` — denied, since `FormVoter` carries no `AdministrativeScope` clause at all | Trainer A attempts `FORM_EDIT` on Trainer B's `Form` id — structurally invisible, `404` |
| `FormSubmissionVoter::FORM_SUBMISSION_CREATE` | Anonymous visitor submits to an active, under-capacity, published `Form` via its code | Code resolves to a `Form` that is disabled ("Registration Closed") or at capacity ("Camp Full") — denied | By construction, source 5's precedence sets `TenantContext` to the code's own trainer before the voter runs, so a "wrong-tenant" submission cannot occur via this route. Adjacent structural case (mirrors `ShareLinkVoter`'s drift row): a `PublicTenantCode` row surviving a deleted `Form` resolves a tenant but the `Form` no longer loads — same "closed" denial path, benign by construction (`council-sharelink-tenant-resolution.md:289`) |

---

## Decisions

| Decision | Chosen | Rejected | Because |
|---|---|---|---|
| `ChildApprovalVoter::CHILD_APPROVAL_BYPASS` subject shape | A typed `ChildActionAttempt` DTO (actor, beneficiary, funding method) | The bare `PlayerProfile`; encoding funding method into the attribute name (`CHILD_APPROVAL_BYPASS_TOKEN`/`_USD`) | The bypass rule depends on how the action is funded (BR-01-18 vs BR-01-19), which a bare profile can't carry without the caller pre-branching in a controller — exactly what `architect-architecture.md:479-480` forbids. A `_USD` attribute would be permanently-denying dead code, since BR-01-18 has no bypass at all |
| `CHILD_APPROVAL_DECIDE` re-checks the 48-hour window itself | The voter independently re-applies the cutoff, not only the read-side "Expired" label | Trusting the read-side display alone | A voter that still granted a decision after the request auto-denied (BR-01-18) would let a late parent override an outcome the business rule already settled. This document's own inference from "derived at read time," not a verbatim rule (Decisions row "Time-based state," `architect-architecture.md:714`) |
| `PlatformConfigurationVoter` module placement | `Platform` | `Administration` (matching `FeatureToggle`'s own entity-population tag) | A voter class has one namespace; `Platform` already owns `PlatformConfiguration` outright and both subjects are identically-shaped global Super-Admin config. `specs/api-designer-spec.md` names the voter and attribute but never a module for it |
| `TrainerSettingsVoter` module placement | `Billing` | `Platform` (matching `TrainerBrandingSettings`/`Trainer`'s own tag); splitting into two voter classes | Two of the three attributes protect money-shaped, `Billing`-owned settings; splitting would rename `specs/api-designer-spec.md`'s already-settled single-voter, single-attribute vocabulary, which the brief forbids doing silently |
| `FeatureGate` consulted by Content-module voters for the LPPP toggle | Every `PlaylistVoter`/`ContentItemVoter` attribute ANDs a `FeatureGate::isEnabled($trainer, 'lppp')` check | Leaving Content ungated because `specs/api-designer-spec.md`'s Content route table never repeats the "(feature toggle)" annotation Growth/Forms carry | BR-07-1 states the LPPP-disabled outcome explicitly ("players see no content"), and the architecture's own `FeatureGate` definition already names LPPP as one of the three toggles it evaluates — the silence reads as the route table's terseness (it only annotates each module's first row), not as an exemption |
| CRM Master's Super-Admin clause on `PlayerVoter` | Write the clause where an AC settles a capability (`PLAYER_VIEW`, `PLAYER_FLAG_MANAGE`, `PLAYER_NOTE_MANAGE`, `PLAYER_FEEDBACK_EDIT`); write an explicit **absence** where an AC settles a prohibition (`LabelVoter::LABEL_MANAGE`) | Asserting `AdministrativeScope` as the mechanism for the granted cases, by analogy with Event Master | AC-03-56/58 settle *that* Super Admin can act, never *how* (no route is drawn, unlike Event Master's explicit `AdministrativeScope` and Forms' explicit impersonation) — asserting an unstated mechanism would be inventing a requirement. Carried to Open questions instead |
| Cross-tenant test substitutes for global-subject voters | Every test-matrix row gets a boundary-deny case, even where the subject carries no tenant column | Marking global-subject voters "N/A" and stopping there | A bare N/A invites exactly the complacency this design exists to prevent; the nearest real boundary (role, ownership, or family) is always worth a named test even when it isn't literally a tenant |

---

## Open questions

1. **CRM Master's write mechanism.** AC-03-56/58
   (`specs/requirements-analyst-epic-03-crm-players-spec.md:266-268`)
   settle that Super Admin may view/edit all player data and apply/remove
   flags across trainers, and edit or delete any note (BR-03-12, line 319)
   or feedback (AC-03-49, line 255) at any time — but
   `specs/api-designer-spec.md`'s Crm module route table has no
   `crm_super_admin_*` routes at all, despite its own Decisions table
   committing to one (line 1122). This design writes the capability into
   `PlayerVoter`'s attributes but cannot state whether the write mechanism
   is `AdministrativeScope` (like Event Master) or impersonation-only
   (like Forms) — the two have different reachability and audit shapes
   (see Super Admin, above), and picking one would invent a requirement
   neither the epic nor the architecture states. The same gap applies to
   `AttendanceVoter`'s Super-Admin "any time" branch (BR-02-18) — no
   `/super-admin/events/{event}/attendance` route is drawn either.
2. **Feature-toggle-off direct navigation.** Inherited from
   `specs/api-designer-spec.md:1191-1199`: BR-07-1/3 state a disabled
   feature disappears from the trainer's UI, not what happens on a direct
   hit to the URL. This design's voters deny (`403`) rather than `404` or
   upsell, on the principle that "a disabled feature is denied at the same
   place as any other permission" (`architect-architecture.md:474-475`) —
   but the HTTP-shape/copy question api-designer-spec left open remains
   open here too, since it is a response-rendering choice, not an
   authorization one.
3. **Q-03.05 — can coaches apply flags?** Unresolved upstream
   (`specs/requirements-analyst-open-questions.md`, Section B, line 190;
   default "Trainers only"). `PlayerVoter::PLAYER_FLAG_MANAGE`'s coach
   branch is designed and present but denies by default until this
   resolves, matching `specs/api-designer-spec.md:539-546`'s own
   treatment.
4. **Q-06.10 — coupon eligibility rule.** Unresolved upstream (Section B,
   line 197; default "per-coupon setting").
   `CouponVoter::COUPON_APPLY` is designed to accept an eligibility rule
   as a parameter without yet fixing its content.
5. **The ShareLink-acceptance tenant-resolution gap upstream of
   `ShareLinkVoter`.** `specs/api-designer-spec.md:872-917` flags that no
   resolution source covers the moment `MembershipService` writes the
   **first** `PlayerTrainerMembership`/`CoachMembership` row from an
   authenticated (not code-resolved) ShareLink acceptance.
   `ShareLinkVoter::SHARELINK_RESOLVE` assumes a genuinely resolved
   `TenantContext` by the time it runs; if that gap is ever closed by
   anything other than the option already adopted into
   `architect-architecture.md`, this voter's assumption should be
   re-checked against whatever mechanism is chosen. Not re-opened or
   re-solved here — flagged only because this design depends on its
   resolution.
6. **Whether `PLAYLIST_VIEW`'s locked/unlocked distinction needs its own
   attribute.** This design folds "is it unlocked" into
   `CONTENT_ITEM_VIEW` and leaves `PLAYLIST_VIEW` as a pure
   ownership/visibility check (see Content module notes). If a future
   screen needs to show unlock state on the *playlist* row itself, not
   just gate individual item playback, a read-only
   `PLAYLIST_CONTENT_UNLOCKED`-shaped attribute may be worth splitting
   out — not designed here since no route currently needs it.

---

*Written 2026-08-09. Inputs: `specs/architect-architecture.md`,
`specs/council-sharelink-tenant-resolution.md`, `specs/api-designer-spec.md`,
and Section A of `specs/requirements-analyst-open-questions.md`, cross-checked
against the Business rules and permission-bearing Acceptance criteria of all
eight `specs/requirements-analyst-epic-0{1..8}-*-spec.md` files. This
document's own additions — `ChildApprovalVoter::CHILD_APPROVAL_BYPASS`, the
`ChildActionAttempt` subject, and the module-placement calls for
`PlatformConfigurationVoter` and `TrainerSettingsVoter` — are marked "(new —
this document)" at first use and recorded in the Decisions table above.*
