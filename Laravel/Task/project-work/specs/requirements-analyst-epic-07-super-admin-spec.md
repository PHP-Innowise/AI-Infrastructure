# Spec: Epic-07 — Super Admin & System Management

Source: `Task/Epics/Epic-07_Super_Admin_System_Management_SPEC.md` (read-only client material)

## Problem

Epic-07 gives the platform's single Super Admin operator the tools to run
PracticePerfect once accounts, events, and payments already exist. It is
explicitly built on top of three other epics and cannot function without
them: Epic-01 (user accounts and impersonation), Epic-02 (events to display
in the Event Master Tool), and Epic-05 (Stripe integration for financial
links) ("Dependencies — Required Before This Epic", lines 89-92). It blocks
no other epic and can be developed in parallel with Epic-06 ("Dependencies —
Blocks These Epics", lines 94-95). Without a dedicated admin layer, the
Super Admin would have no way to onboard trainers, diagnose trainer-reported
problems, control feature rollout, or establish who changed what and when —
undermining the platform's ability to scale support, roll out features
safely, and stay accountable ("Business Value", lines 10-13; "Success
Metrics", lines 16-19). This epic is explicitly Super Admin-only: no other
role interacts with any tool it describes ("User Roles Involved", line 108).

Epic-07 must therefore give Super Admin: an operational, non-financial
dashboard for platform health and growth (US-07.01); a searchable, editable
view of every user across every trainer (US-07.02, US-07.03); the ability to
onboard new trainers (US-07.04); per-trainer control over which of three
MVP features (LPPP, Marketing, Camps) a trainer can use (US-07.05); a
system-wide view of every event with the power to edit, cancel, or override
scheduling conflicts on any of them (US-07.06, US-07.07); an accountability
trail of critical admin actions (US-07.08); and a clean hand-off to Stripe
for anything financial, since Stripe — not the platform — is the source of
truth for revenue, payouts, and transactions (US-07.09; "Scope Summary —
Goals", lines 25-31; "In Scope (MVP) — Dashboard & Analytics", lines 48-50).

**Cross-epic dependency notes** (from "Integration Points", line 593):
- Epic-01 (User Management): user editing in this epic uses the same user
  data model as Epic-01; impersonation is Epic-01 US-01.07's functionality,
  only referenced from Super Admin tools here; trainer creation is Epic-01
  US-01.01's functionality, made accessible from the Super Admin dashboard
  here — lines 595-598.
- Epic-02 (Event Management): the Event Master Tool displays all of
  Epic-02's events; Super Admin can edit/cancel any of them — lines 600-602.
- Epic-05 (Payments): Stripe Dashboard links surface Epic-05's financial
  data; trainer subscription status (also Epic-05) is visible in the
  trainer list — lines 604-606.
- Epic-06 (Marketing): the Marketing Tools feature toggle enables/disables
  Epic-06's referral and coupon features — lines 608-609.
- Epic-04 (LPPP Content): the LPPP feature toggle enables/disables Epic-04's
  content system — lines 611-612.
- Epic-03 (CRM): trainers have their own Quick View Dashboard (Epic-03
  US-03.09) showing player metrics, Top Players, and attendance trends for
  their own organization; the Super Admin dashboard in this epic (US-07.01)
  shows a system-wide version of similar metrics — lines 614-617.

**External dependency** (from "Dependencies — External Dependencies", line
97): the Stripe Dashboard is where all financial data is viewed; the
platform keeps no independent view of it (line 98).

## User scenarios

1. **Super Admin** views platform usage and engagement metrics on a
   dashboard so that they can monitor platform health and growth.
   Path: Super Admin logs in (or navigates to "Dashboard") and sees
   operational-only metrics — user counts, session/RSVP/attendance stats,
   top performers — with a date-range selector (7/30/90 days, default 30),
   plus a button linking out to Stripe for all financial data.
   Source: US-07.01 "Super Admin Views Operational Dashboard" (line 114)

2. **Super Admin** searches and views all users across all trainers so that
   they can provide support and manage accounts.
   Path: Super Admin opens the Users tool, searches/filters by role and
   status, and sees a paginated table (50/page) with per-user actions to
   view, edit, impersonate, or deactivate.
   Source: US-07.02 "Super Admin Views All Users" (line 160)

3. **Super Admin** directly edits a user profile so that they can fix
   issues quickly without impersonating.
   Path: Super Admin clicks "Edit" on a user row, changes name/email/status
   (role and trainer stay view-only), saves, and an audit log entry records
   the edit.
   Source: US-07.03 "Super Admin Edits User Profile" (line 195)

4. **Super Admin** creates a new trainer account so that they can onboard
   trainers to the platform.
   Path: Super Admin clicks "Create Trainer" from the Users tool or a
   dashboard quick-action, fills business name/trainer name/email/
   subscription tier, and saves; the trainer account and Stripe
   subscription are created and an invitation email is sent (full creation
   mechanics are Epic-01 US-01.01).
   Source: US-07.04 "Super Admin Creates Trainer Account" (line 223)

5. **Super Admin** enables or disables specific features for an individual
   trainer so that they can control feature rollout and customize per
   trainer.
   Path: Super Admin opens a trainer's "Feature Settings," toggles
   LPPP/Marketing/Camps on or off, confirms the effect, and the change
   applies immediately to that trainer's UI; the toggle is logged.
   Source: US-07.05 "Super Admin Configures Feature Toggle per Trainer"
   (line 243)

6. **Super Admin** views and manages all events across all trainers so that
   they can resolve scheduling issues and monitor platform activity.
   Path: Super Admin opens "Event Master," searches/filters by
   trainer/date/type/status, and can view, edit, cancel, or view the RSVP
   list of any event system-wide.
   Source: US-07.06 "Super Admin Views Event Master Tool" (line 287)

7. **Super Admin** overrides a scheduling-conflict warning so that they can
   resolve urgent issues without restriction.
   Path: While creating/editing an event via the Event Master Tool, if a
   conflict is detected (e.g. a double-booked coach), no warning blocks the
   save; the override is logged.
   Source: US-07.07 "Super Admin Overrides Scheduling Conflict" (line 332)

8. **Super Admin** views all critical admin actions so that they can track
   changes and ensure accountability.
   Path: Super Admin opens the Audit Log, filters by date range/action
   type/subject, and can export the results to CSV; entries are retained
   at least 1 year.
   Source: US-07.08 "Super Admin Views Audit Log" (line 354)

9. **Super Admin** accesses the Stripe Dashboard from the platform so that
   they can view financial data and manage trainer subscriptions.
   Path: Super Admin clicks "View Financial Reports in Stripe" (or a
   per-trainer "View [Trainer]'s Stripe Account" link) and is taken to the
   Stripe Express/Connect dashboard, auto-logged-in via SSO if their
   session is active.
   Source: US-07.09 "Super Admin Links to Stripe Dashboard" (line 392)

## Acceptance criteria

Every checkbox below is renumbered sequentially as AC-07-1 .. AC-07-41 from
the epic's per-story "Acceptance Criteria" lists (US-07.01–US-07.09), three
criteria derived from "In Scope (MVP)" § "Trainer Management" and §
"System Configuration" scope notes that have no dedicated user story, one
epic-level performance/scale criterion with no matching story, and the
epic-level "Acceptance Criteria (Epic Level)" section. Where an epic-level
criterion restates a story or scope-note criterion, the story/scope-note
version is kept and both origins are cited.

**US-07.01 — Super Admin Views Operational Dashboard** (line 114)
- [ ] **AC-07-1** After login (or via "Dashboard" navigation), Super Admin
  sees a dashboard showing **operational metrics only** — no financial
  data — `Epic-07_Super_Admin_System_Management_SPEC.md` § "US-07.01: Super
  Admin Views Operational Dashboard" (lines 121-122; also "In Scope (MVP)"
  § "Dashboard & Analytics", line 48; epic-level AC § "Acceptance Criteria
  (Epic Level)" § "Dashboard", line 624)
- [ ] **AC-07-2** Dashboard User Metrics: total trainers (active Stripe
  subscriptions), total players (registered), total coaches (registered),
  new users this week, new users this month, and a 30-day user growth
  chart — § "US-07.01..." § "User Metrics" (lines 125-130)
- [ ] **AC-07-3** Dashboard Session Metrics: sessions this week (training,
  private, camps), RSVPs this week, attendance rate, no-show rate — §
  "US-07.01..." § "Session Metrics" (lines 133-136)
- [ ] **AC-07-4** Dashboard Top Performers: most active trainers (name,
  session count, player count) and top players (name, trainer, attendance
  count) — § "US-07.01..." § "Top Performers" (lines 139-142)
- [ ] **AC-07-5** Dashboard date-range selector offers presets Last 7/30/90
  days, defaulting to Last 30 days — § "US-07.01..." § "Date Range
  Selector" (lines 145-146)
- [ ] **AC-07-6** Dashboard shows a prominent "View Financial Reports in
  Stripe" button that opens the Stripe Express Dashboard (Super Admin's own
  account) — § "US-07.01..." § "Financial Data → Stripe Dashboard" (lines
  149-150; also "In Scope (MVP)" § "Dashboard & Analytics", line 49;
  epic-level AC § "Dashboard", line 625)
- [ ] **AC-07-7** (Negative criterion) No financial data — revenue,
  payouts, transactions — is duplicated on the platform; none of it appears
  on the platform dashboard or anywhere else, and Stripe is the sole source
  of truth for all financial reporting — § "US-07.01..." § "Financial Data
  → Stripe Dashboard" (line 151; also "In Scope (MVP)" § "Dashboard &
  Analytics", line 50; "Out of Scope (Post-MVP)" § "Phase 2 Deferrals",
  line 74; epic-level AC § "Dashboard", line 626; US-07.09 § "Rationale",
  lines 415-418)

**US-07.02 — Super Admin Views All Users** (line 160)
- [ ] **AC-07-8** Super Admin navigates to a "Users" tool and views a list
  of all users across all trainers (trainers, coaches, players, parents) —
  § "US-07.02: Super Admin Views All Users" (lines 167-168; also "In Scope
  (MVP)" § "Tools", line 65; epic-level AC § "Tools", line 637)
- [ ] **AC-07-9** The Users tool provides a tool-specific search — a
  "Search users by name or email" box plus filters for Role (All, Trainer,
  Coach, Player) and Status (Active, Inactive) — with results shown in a
  table — § "US-07.02..." § "Tool-specific search" (lines 169-172)
- [ ] **AC-07-10** The user table shows name, email, role, associated
  trainer (for coaches/players), status (Active/Inactive/Pending),
  registration date, and last login — § "US-07.02..." § "User Table Shows"
  (lines 175-181)
- [ ] **AC-07-11** Each row offers actions to view the user's profile, edit
  the user, impersonate the user, and deactivate the user; impersonation
  and deactivation follow Epic-01's mechanics (US-01.07, line 383, and
  US-01.12, line 507, respectively) — § "US-07.02..." § "Actions Per User"
  (lines 184-187; also "In Scope (MVP)" § "Tools", line 66)
- [ ] **AC-07-12** The Users tool paginates at 50 users per page with page
  navigation — § "US-07.02..." § "Pagination" (lines 190-191)

**US-07.03 — Super Admin Edits User Profile** (line 195)
- [ ] **AC-07-13** Super Admin clicks "Edit" on a user row in the Users
  tool to open an edit form — § "US-07.03: Super Admin Edits User Profile"
  (line 202)
- [ ] **AC-07-14** The edit form allows changing name, email (with a
  warning that changing email requires re-verification), and status
  (Active/Inactive toggle); role and associated trainer are shown
  view-only (role cannot be changed here), and player-specific profile
  details (age, gender, etc.) are editable when the user is a player — §
  "US-07.03..." (lines 203-209)
- [ ] **AC-07-15** Saving the edit form persists the changes and creates an
  audit log entry ("Super Admin edited user [Name]") — § "US-07.03..."
  (lines 210-212)

**US-07.04 — Super Admin Creates Trainer Account** (line 223)
- [ ] **AC-07-16** Super Admin creates a trainer account from the Users
  tool via "Create Trainer" (the account-creation mechanics themselves are
  Epic-01 US-01.01, line 144); Epic-07 additionally specifies that the
  creation form captures a subscription tier, and that saving creates a
  Stripe subscription (Epic-05 integration) alongside sending the trainer
  their invitation/login — § "US-07.04: Super Admin Creates Trainer
  Account" (lines 230-236; also "In Scope (MVP)" § "Trainer Management",
  line 53; epic-level AC § "Trainer Management", line 630)
- [ ] **AC-07-17** The Super Admin dashboard shows a "Create Trainer"
  quick-action button — § "US-07.04..." § "Dashboard Link" (line 239)

**US-07.05 — Super Admin Configures Feature Toggle per Trainer** (line 243)
- [ ] **AC-07-18** Super Admin navigates to the Trainers list, selects a
  trainer, and opens "Feature Settings" to view that trainer's feature
  toggle list — § "US-07.05: Super Admin Configures Feature Toggle per
  Trainer" (lines 250-252; also "In Scope (MVP)" § "System Configuration",
  line 60; epic-level AC § "Trainer Management", line 633)
- [ ] **AC-07-19** Exactly three features are toggleable per trainer for
  MVP, each an independent ON/OFF toggle defaulting to ON: LPPP Content
  System (affects Epic-04 — "Enable Learn, Practice, Perfect content
  creation and player access"), Marketing Tools (affects Epic-06 — "Enable
  referrals and coupon codes"), and Camps (affects Epic-08 per the resolved
  Q-07.01 — "Enable camp events with external registration forms," though
  the toggle's own description labels its epic as "Epic-02"; see Open
  questions) — § "US-07.05..." § "Feature Toggles (3 for MVP)" (lines
  255-268; also epic-level AC § "Feature Toggles", lines 642-644;
  resolution, line 588)
- [ ] **AC-07-20** Toggling a feature requires a confirmation naming the
  trainer and feature and warning of the effect (e.g. "Disable LPPP
  Content for [Trainer]? Existing content will be hidden from players.");
  saving applies the change immediately so the trainer's UI shows or hides
  the feature — § "US-07.05..." § "Toggle Actions" (lines 271-275; also
  epic-level AC § "Feature Toggles", line 645)
- [ ] **AC-07-21** Every feature-toggle change is logged — who toggled it,
  which feature, for which trainer, and when — § "US-07.05..." § "Audit
  Logging" (line 283)

**US-07.06 — Super Admin Views Event Master Tool** (line 287)
- [ ] **AC-07-22** Super Admin navigates to "Events"/"Event Master" and
  views all events from all trainers, system-wide — § "US-07.06: Super
  Admin Views Event Master Tool" (lines 294-295; also "In Scope (MVP)" §
  "Tools", line 64; epic-level AC § "Tools", line 636)
- [ ] **AC-07-23** The event list shows event title, trainer, date & time,
  event type (Session, Private, Camp), capacity (e.g. "15 / 20"), status
  (Upcoming, Completed, Canceled), and assigned coach (if any) — §
  "US-07.06..." § "Event List Shows" (lines 298-304)
- [ ] **AC-07-24** The Event Master Tool provides tool-specific search and
  filters: search by event title or trainer name, and filter by date range
  (next 7/30 days, custom), trainer, event type, and status — §
  "US-07.06..." § "Filters (Tool-Specific Search - D-SCOPE-005)" (lines
  307-311)
- [ ] **AC-07-25** Per event, Super Admin can view full details, edit the
  event as if they had created it, cancel it (emergency override), and
  view its RSVP list — § "US-07.06..." § "Actions Per Event" (lines
  314-317)
- [ ] **AC-07-26** Events can be sorted by date (ascending/descending),
  trainer, or capacity, and are paginated at 50 events per page — §
  "US-07.06..." § "Sort" (line 320) and § "Pagination" (line 323)

**US-07.07 — Super Admin Overrides Scheduling Conflict** (line 332)
- [ ] **AC-07-27** When Super Admin creates or edits an event via the Event
  Master Tool and a scheduling conflict is detected (e.g. a coach already
  assigned to another event at the same time, or a trainer with an event
  at an unavailable time), no warning is shown and the event saves without
  requiring confirmation — § "US-07.07: Super Admin Overrides Scheduling
  Conflict" (lines 339-344)
- [ ] **AC-07-28** Every such overridden conflict is recorded in the audit
  log ("Super Admin overrode conflict: [details]") — § "US-07.07..." (line
  345)

**US-07.08 — Super Admin Views Audit Log** (line 354)
- [ ] **AC-07-29** Super Admin navigates to "Audit Log"/"Activity Log" and
  views a chronological list of logged actions — § "US-07.08: Super Admin
  Views Audit Log" (lines 361-362; also "In Scope (MVP)" § "Tools", line
  67)
- [ ] **AC-07-30** The audit log captures, at minimum: impersonation
  sessions (who, when, duration — per Epic-01), trainer accounts
  created/deactivated, user accounts deleted (GDPR), feature-toggle
  changes (what, for whom), pricing changes (per-trainer rates), and
  overridden scheduling conflicts (which event) — § "US-07.08..." §
  "Logged Actions" (lines 365-370; also Business Rules § "Audit Logging
  Rules — What Gets Logged", lines 470-472; epic-level AC § "Tools", line
  638, and § "Audit Logging", line 648)
- [ ] **AC-07-31** Each log entry shows a timestamp, the action performed,
  the subject affected, action-specific details, and the admin who
  performed it — § "US-07.08..." § "Log Entry Shows" (lines 373-377)
- [ ] **AC-07-32** The audit log can be filtered by date range (last 7/30
  days, custom) and action type, and searched by subject (user or trainer
  name) — § "US-07.08..." § "Filters" (lines 380-382; also epic-level AC §
  "Audit Logging", line 649)
- [ ] **AC-07-33** Audit log entries can be exported to CSV for compliance
  audits — § "US-07.08..." § "Export" (line 385; also epic-level AC §
  "Audit Logging", line 650)
- [ ] **AC-07-34** Audit log entries are retained for at least 1 year, with
  retention configurable to extend further — § "US-07.08..." §
  "Retention" (line 388; also Business Rules § "Audit Logging Rules — Log
  Retention", lines 480-481; epic-level AC § "Audit Logging", line 651)

**US-07.09 — Super Admin Links to Stripe Dashboard** (line 392)
- [ ] **AC-07-35** The "View Financial Reports in Stripe" button opens the
  Stripe Express Dashboard in a new tab, auto-logging in via SSO if the
  Super Admin's session is active — § "US-07.09: Super Admin Links to
  Stripe Dashboard" (lines 399-401)
- [ ] **AC-07-36** The linked Stripe Dashboard, not the platform, is where
  revenue breakdown, transaction history, trainer subscriptions, payout
  schedules, tax documents, and dispute/refund management are viewed — §
  "US-07.09..." § "Stripe Dashboard shows" (lines 402-408)
- [ ] **AC-07-37** A per-trainer "View [Trainer]'s Stripe Account" link
  opens that trainer's Stripe Connect account, showing the trainer's
  earnings, payouts, and subscription status — § "US-07.09..." § "Also
  Available" (lines 411-413)

**Scope-note items (from "In Scope (MVP)" § "Trainer Management", lines
54-57, and § "System Configuration", line 61 — no dedicated user story)**
- [ ] **AC-07-38** Super Admin can view a list of all trainers, view/edit
  trainer details, and view each trainer's subscription status via Stripe
  — `Epic-07_Super_Admin_System_Management_SPEC.md` § "In Scope (MVP)" §
  "Trainer Management" (lines 54-55, 57; also epic-level AC § "Trainer
  Management", line 631)
- [ ] **AC-07-39** Super Admin can deactivate and reactivate trainer
  accounts, using the same deactivation/reactivation mechanism specified
  for any user in Epic-01 US-01.12 (line 507) — § "In Scope (MVP)" §
  "Trainer Management" (line 56; also epic-level AC § "Trainer Management",
  line 632)
- [ ] **AC-07-40** Pricing configuration per trainer is in scope, but its
  mechanics are specified in Epic-05, not here; this epic's own addition is
  that pricing changes appear in the Super Admin audit log ("Pricing
  changed (per-trainer rates)") — § "In Scope (MVP)" § "System
  Configuration" (line 61); § "US-07.08..." § "Logged Actions" (line 369)

**Epic-level (from "Acceptance Criteria (Epic Level)", line 621) — not
already covered above**
- [ ] **AC-07-41** Performance and scale: dashboard loads <2 seconds (with
  100+ trainers); all Super Admin tools load <2 seconds; user search
  returns in <500ms; the Event Master Tool loads <2 seconds with 1,000
  events (paginated, up to 5,000 total); the audit log loads <1 second with
  10,000 entries (storing up to 100,000); the platform supports 500
  trainers and 10,000 players — § "Success Metrics" (line 19); §
  "Acceptance Criteria (Epic Level)" § "Dashboard" (line 627) and § "Tools"
  (line 639); § "Performance & Scale Targets" §§ "Response Times" and
  "Scalability" (lines 561-564, 567-570)

**Final count: 41 acceptance criteria (AC-07-1 .. AC-07-41)**, covering all
9 user stories (US-07.01–US-07.09), 3 Trainer-Management/System-Configuration
scope-note items with no dedicated story, and the epic-level completion
checklist, with 18 epic-level items folded into their matching story or
scope-note criterion rather than duplicated, and 2 epic-level performance
items combined into one dedicated cross-cutting criterion (AC-07-41).

## Business rules

Restated from "Business Rules & Logic" (line 449):

**Feature Toggle**
- [ ] **BR-07-1** Disabling a feature blocks the corresponding trainer
  capability specifically: LPPP disabled removes trainer access to the
  LPPP section and players see no content; Marketing disabled removes the
  trainer's ability to create coupons or view the referral dashboard;
  Camps disabled removes the trainer's ability to create camp events —
  `Epic-07_Super_Admin_System_Management_SPEC.md` § "Business Rules &
  Logic — Feature Toggle Rules — Feature Scope" (lines 454-456)
- [ ] **BR-07-2** All three features default to enabled for new trainers;
  Super Admin can disable a feature before a trainer's onboarding
  completes (e.g. for custom plans) — § "...Feature Toggle Rules —
  Default State" (lines 459-460)
- [ ] **BR-07-3** Toggling takes effect immediately — the feature
  disappears from the trainer's UI within seconds; disabling a feature
  preserves its existing data (hides, does not delete), and re-enabling
  restores all previous data — § "...Feature Toggle Rules — Toggle Effect"
  (lines 463-465)

**Audit Logging**
- [ ] **BR-07-4** Every Super Admin action that modifies data is logged,
  including all impersonation sessions and critical configuration changes
  — § "...Audit Logging Rules — What Gets Logged" (lines 470-472)
- [ ] **BR-07-5** (Negative rule) Read-only Super Admin activity is
  explicitly excluded from the audit log as too verbose: viewing
  dashboards, searching users, and viewing the audit log itself are not
  logged — § "...Audit Logging Rules — What Does NOT Get Logged (too
  verbose)" (lines 475-477)
- [ ] **BR-07-6** Audit log entries are retained for a minimum of 1 year,
  with retention configurable to extend further for compliance; entries
  are appended in real time (not batched) — § "...Audit Logging Rules —
  Log Retention" (lines 480-481); § "Performance & Scale Targets — Data
  Refresh" (line 575) — **no immutability rule is stated; see Open
  questions (analyst-raised).**

**Dashboard Metrics**
- [ ] **BR-07-7** Dashboard user counts are defined precisely: active
  trainers = trainers with an active Stripe subscription; total players =
  player accounts with status active; total coaches = coach accounts with
  status active — § "...Dashboard Metrics Rules — User Count Logic" (lines
  486-488)
- [ ] **BR-07-8** New users this week = users registered in the last 7
  days; growth trend = percentage change of the current week's
  registrations against the previous week's — § "...Dashboard Metrics
  Rules — Growth Calculation" (lines 491-492)
- [ ] **BR-07-9** Most-active-trainers ranking uses session count in the
  last 30 days; top-players ranking uses attendance count in the last 30
  days — § "...Dashboard Metrics Rules — Top Performers" (lines 495-496) —
  **whether these are computed live or via a scheduled batch job is
  unclear; see Open questions (analyst-raised).**

## Data requirements

Restated from "Data Requirements" (line 422) — entities, the fields the
epic names, and the relationships it names. No schema, keys, or types are
proposed here.

**Note on Dashboard Metrics** (from "Data Requirements — For Dashboard
Metrics (calculated, not stored)", line 441, vs. "Performance & Scale
Targets — Data Refresh", line 573): the epic states in one place that
dashboard metrics are computed live via query (not stored) and in another
that they update hourly via a batch job — which implies persisted, cached
values. This spec does not resolve which model governs; see Open questions
(analyst-raised).

- **Audit Log entry**: unique identifier, timestamp, action type
  (impersonation, user edit, feature toggle, etc.), subject (user, trainer,
  or event affected), details (JSON or text field with action-specific
  data), admin who performed the action. Relationship: references an admin
  (Super Admin) User, and a variable subject that may be a User, a Trainer,
  or an Event depending on the action type. (lines 427-432)
- **Feature Toggle**: trainer reference, feature name (LPPP, Marketing, or
  Camps), status (enabled/disabled), last-updated timestamp, last-updated-by
  (Super Admin reference). Relationship: belongs to a Trainer; references
  the Super Admin who last changed it. (lines 435-439)
- **Dashboard Metrics** (calculated, not stored, per line 441 — but see the
  note above): user counts (queried from the database), session counts
  (queried from the events table), growth trends (calculated from user
  registration dates), top performers (calculated from attendance records).
  No dedicated storage entity is named beyond the source data owned by
  Epic-01 (users) and Epic-02 (events, attendance). (lines 442-445)

## Edge cases

| Case | Expected |
|---|---|
| Super Admin disables a feature (LPPP/Marketing/Camps) for a trainer with existing content/data under that feature | A confirmation warns the specific effect (e.g. "Disable LPPP Content for [Trainer]? Existing content will be hidden from players."); on confirm, existing data is preserved and merely hidden, not deleted — US-07.05 (line 272); Business Rules "Feature Toggle Rules — Toggle Effect" (line 464) |
| A previously-disabled feature is re-enabled for a trainer | All previously hidden data reappears — Business Rules "Feature Toggle Rules — Toggle Effect" (line 465) |
| Super Admin edits a user's email address | A warning is shown that changing the email requires re-verification — US-07.03 (line 205) |
| Super Admin creates or edits an event via the Event Master Tool and a scheduling conflict is detected (e.g. a double-booked coach, an unavailable trainer) | No warning is shown, the event saves without confirmation, and the override is recorded in the audit log — US-07.07 (lines 339-345) |
| Super Admin merely views a dashboard, searches the Users tool, or views the Audit Log itself | No audit log entry is created for these read-only actions — excluded as "too verbose" — Business Rules "Audit Logging Rules — What Does NOT Get Logged" (lines 475-477) |
| The Camps feature toggle is evaluated for MVP inclusion | Resolved: Camps ARE in MVP scope (via Epic-08); the toggle is always shown for MVP — § "Questions / Open Issues" resolution Q-07.01 (line 588) |
| Bulk deactivation of multiple trainers at once is requested | Resolved: not supported for MVP — one-by-one only — § "Questions / Open Issues" resolution Q-07.03 (line 589); "Out of Scope (Post-MVP)" (line 76) |

## Out of MVP scope

Verbatim from "Out of Scope (Post-MVP)" § "Phase 2 Deferrals" (lines 73-83):

- Financial metrics on dashboard (Stripe handles)
- System health monitoring (storage usage, error rates) - use
  infrastructure tools
- Bulk operations (activate 10 trainers at once)
- Advanced analytics (drill-down, custom date ranges beyond presets)
- User Role Editor (custom role creation)
- Content moderation (flag/review public LPPP content)
- Email/SMS campaign creation from admin panel
- Trainer onboarding workflows (checklists, automated emails)
- Platform-wide announcements
- System backup/restore UI

Also stated as a Non-Goal ("Scope Summary — Non-Goals", line 41) but not
repeated in the list above:
- Portal branding UI for Super Admin (trainers configure their own branding
  themselves — Epic-01 US-01.14, line 551; Super Admin uses impersonation
  to help if needed)

## Open questions

### From the epic's "Questions / Open Issues" section (line 579, verbatim)

| ID | Question | Priority | Status | Owner |
|:---|:---|:---:|:---|:---|
| Q-07.01 | **Camps scope**: Are camps in MVP scope? If yes, include in feature toggles. If no, remove toggle. | P1 | ✅ RESOLVED | Super Admin |
| Q-07.02 | Dashboard date range: Are presets (7/30/90 days) sufficient, or need custom date picker? | P2 | Open | Team |
| Q-07.03 | Bulk operations: Should Super Admin be able to deactivate multiple trainers at once? (Assume NO for MVP) | P2 | ✅ RESOLVED | Team |

**Resolutions** (line 587, verbatim):
- **Q-07.01**: YES - Camps are in MVP scope as Epic-08 (Forms &
  Registration), feature toggle "Camps" included
- **Q-07.03**: NO - Bulk operations deferred to Post-MVP

### Analyst-raised

- **(analyst-raised)** Scope conflict: `Epic_Areas_Plan.md` lists "User
  Role Editor" as in MVP scope twice — as a Key Feature ("User Role Editor
  (modify permissions)", line 383) and explicitly checked into MVP Scope
  ("✅ User Role Editor", line 408) — but this epic file lists "User Role
  Editor (custom role creation)" under "Out of Scope (Post-MVP)" (line 78).
  Per this run's rule that the epic file wins over the plan where they
  disagree, User Role Editor is treated as **out of MVP** for this spec (no
  AC is derived for it). This conflict is significant: it decides whether
  the platform's roles (Super Admin, Trainer, Coach, Player/Parent — Epic-01
  BR-01-7) are fixed constants baked into the codebase, or editable data a
  Super Admin can redefine at runtime — a decision that is expensive to
  retrofit later if resolved the wrong way now.
- **(analyst-raised)** Two related scope tensions around what the Event
  Master Tool "manages" and what "system monitoring" covers:
  (a) *Bulk operations.* This epic's own Scope Summary goal calls the tool
  "system-wide event *management*" (line 29), and "In Scope (MVP)" again
  calls it "view/manage all events system-wide" (line 64) — but this epic
  explicitly places bulk operations out of MVP scope in three places:
  Non-Goals ("Bulk operations (one-by-one for MVP)", line 36), Out of Scope
  ("Bulk operations (activate 10 trainers at once)", line 76), and the
  resolved Q-07.03 ("Bulk operations deferred to Post-MVP", line 589). The
  detailed US-07.06 "Actions Per Event" (lines 314-317) are all
  singular/per-event (view, edit, cancel, view RSVP), confirming the
  epic's own intent. However, `Epic_Areas_Plan.md`'s Epic-02 section lists
  "Bulk operations (Super Admin: clone, edit, cancel)" as a Key Feature
  (line 102) and checks "Bulk operations (simplified)" into MVP Scope (line
  119) — and per its own wording ("Super Admin: ...") this bulk capability
  would live in the Super Admin's Event Master Tool, i.e. this epic's
  territory. Per the epic-wins rule, this spec derives no bulk-action AC
  for the Event Master Tool (AC-07-25 lists only per-event actions) — but
  the client should confirm "manage" is not meant to imply any bulk
  capability, since a second planning document says otherwise.
  (b) *System monitoring.* This epic's own Purpose statement says Super
  Admin tools provide "system monitoring" among other things (line 8), but
  this epic's own Non-Goals ("System health monitoring (storage, errors) -
  use infrastructure tools", line 35) and Out of Scope ("System health
  monitoring (storage usage, error rates) - use infrastructure tools",
  line 75) explicitly exclude system health monitoring from MVP. No user
  story or AC in this epic delivers infrastructure-style monitoring
  (storage, error rates); the closest delivered capability is the
  operational Dashboard (US-07.01) and the Audit Log (US-07.08), neither of
  which covers storage or error-rate monitoring. This spec treats "system
  monitoring" in the Purpose statement as covered only by those two —
  true infrastructure/system-health monitoring is out of MVP — but the
  wording gap is worth a fix request to the client. (Unlike (a), no
  external document disagrees here — `Epic_Areas_Plan.md`'s own Epic-07
  section agrees system health monitoring is Post-MVP, line 410 — this is
  purely an internal tension within Epic-07's own text.)
- **(analyst-raised)** Not itself a contested question, but flagged here
  for visibility given how emphatically this epic repeats it: no financial
  data (revenue, payouts, transactions) is duplicated on the platform, and
  Stripe is the sole source of truth (stated four times: lines 50, 74,
  151, 415-418). Captured as a negative criterion at AC-07-7; it also
  constrains Data requirements above — no revenue/payout/transaction
  fields are derived there.
- **(analyst-raised)** Internal contradiction: "Data Requirements" states
  dashboard metrics are "(calculated, not stored)" (line 441) — i.e.
  computed live via query on each view — but "Performance & Scale Targets"
  § "Data Refresh" states "Dashboard metrics: Update hourly (batch job)"
  (line 573), which implies metrics ARE persisted and refreshed on a
  schedule, not computed live. This affects the Data requirements design
  directly: a live-query model needs no metrics-storage entity, while an
  hourly-batch model needs one to hold last-computed values between
  refreshes. Needs clarification before this area's data model can be
  finalized.
- **(analyst-raised)** This epic states an audit-log retention rule
  (minimum 1 year, configurable — line 388, lines 480-481) and that
  entries are appended in real time (line 575), but never states whether a
  written entry can later be edited or deleted. For a log whose stated
  purpose is accountability (US-07.08: "so that I can track changes and
  ensure accountability") and compliance export (line 385), whether
  entries are immutable/append-only is a materially important, currently
  unanswered question.
- **(analyst-raised)** The Event Master Tool this epic specifies in
  US-07.06 (lines 287-330) appears to be independently specified a second
  time, in more detail, as `Epic-02_Event_Management_Scheduling_SPEC.md`'s
  own "US-02.15: Super Admin Views All Events" (lines 530-555) — and the
  two disagree on specifics: (1) event-type vocabulary — this epic's
  Event List shows "Session, Private, Camp" (line 301), while Epic-02's
  US-02.15 filters by "Training, Private, Small Group" (line 544), and
  Epic-02's own MVP Scope elsewhere says "Training Sessions, Camps,
  Privates" (`Epic_Areas_Plan.md`, line 110) — three different lists across
  two files; (2) Epic-02's US-02.15 includes a Location filter (line 542)
  and an "Export events (CSV for reporting)" action (line 553), neither of
  which appears anywhere in this epic's US-07.06; (3) status vocabulary —
  this epic says "Upcoming, Completed, Canceled" (line 303) versus Epic-02's
  "Active, Canceled, Completed" (line 545). This spec derives its Event
  Master Tool ACs (AC-07-22 .. AC-07-26) from this epic's file only, per
  assignment scope, and does not add the CSV-export action or Location
  filter found only in Epic-02 — but the duplication and its discrepancies
  should be reconciled by the client before build.
- **(analyst-raised)** Minor inconsistency in which epic the "Camps"
  feature toggle affects: the toggle's own description labels it "(Epic-02
  - if in scope, Q-07.01)" (line 265), but the resolution of that same
  Q-07.01 says Camps are in MVP scope "as Epic-08 (Forms & Registration)"
  (line 588) — and this epic's own toggle description mentions "external
  registration forms" (line 267), which points to Epic-08 (forms) rather
  than Epic-02 (events) as the more precise downstream target. Camp
  *events* as an event type do belong to Epic-02 (confirmed in
  `Epic_Areas_Plan.md`'s Epic-02 MVP Scope, "Training Sessions, Camps,
  Privates", line 110), so the toggle may need to be understood as gating
  both — but this epic never states that explicitly.
