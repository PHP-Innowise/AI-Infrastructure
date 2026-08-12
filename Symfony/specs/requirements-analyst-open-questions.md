# PracticePerfect — Consolidated Open Questions Register

Collected from all eight epic specs in `specs/` (which derive from the
read-only client material in `Task/Epics/`) plus the cross-epic council on
player identity. This is the Phase 1 blocking checkpoint: Phase 2 (design)
should not start until Section A is answered.

**Totals**

| Origin | Count |
|---|---:|
| Client questions still open, stated by the epics | 29 |
| Client questions the epics already resolved | 10 |
| Analyst-raised (contradictions, gaps, source defects) | ~75 |
| Council questions on cross-epic player identity | 7 |

Each per-epic spec carries its own full `## Open questions` section with
verbatim client wording and line-level citations. This file is the index and
the priority call; it does not replace them.

---

## Section A — ANSWERED 2026-08-09. These are now decisions.

The owner answered A1, A3, A6 and A8 directly; the rest were carried on the
recommendation stated with each, without objection. **Do not relitigate these
in a later session** — reopen only if the client changes the underlying epic.

| # | Decision |
|---|---|
| A1 | **All under-18 players require a parent-managed account.** Epic-01 BR-01-17 governs; the COPPA question is closed for MVP. No independent 16-18 accounts. |
| A2 | **User Role Editor is out of MVP.** The four roles are fixed constants, not runtime-editable data. Epic-07 governs over the plan. |
| A3 | **A camp registrant who never converts gets a form submission plus a camp payment record, and no account.** Requires a fourth CRM association source (`camp_registration`) and a payment record that does not require an account holder. Auto-creating a shadow account is explicitly rejected. |
| A4 | Follows from A3: camp money is recorded platform-side against the submission, not against a player account, so it can be reconciled and refunded. |
| A5 | **On conversion, the earlier camp payment and registration attach to the new account.** |
| A6 | **No combined cross-trainer view on any player-facing screen.** Epic-01 AC-01-15 governs; Epic-05 AC-05-24's "Family Overview" is dropped. Tenancy isolation wins. |
| A7 | **Every token spend records the beneficiary player**, even though the balance sits at the parent-trainer pair. |
| A8 | **Content is sold as per-playlist one-time purchase.** The other three candidate models in D-SCOPE-011 are out of MVP; the purchase record must not foreclose adding them later. |
| A9 | **Playlist visibility is three-state: public / private / coach-only**, not binary. |
| A10 | **Build order: 01 -> 02 -> {03, 04} -> 05 -> {06, 07, 08}**, derived from each epic's own "Required Before This Epic". The plan's diagram is wrong about Epic-08. |
| A11 | **Epic-04 content structure first, then Epic-05, then wire the paywall.** |
| A12 | **Proceed with 85 user stories.** US-02.09 does not exist in the source; report it to the client rather than inventing it. |
| Fee | **The trainer absorbs the 5% platform fee**; it is included in the listed price. Epic-05 BR-05-7 governs. |

### Original statement of the blocking questions

These change the data model, the authorization model, or the money path.
Getting one wrong is expensive after data exists.

### A1. Can a 16–18 year old hold an account with no parent attached?

Epic-01 states as a settled business rule that **all** under-18 players
require parent-managed accounts (BR-01-17), and in the same file carries an
unresolved question asking exactly that, citing COPPA (US-01.06). The epic
contradicts itself.

Blocks: registration, the parent-approval workflow, and who may hold a payment
method. Touches AC-01-19 through AC-01-31 and every Epic-05 approval rule.

### A2. Is the User Role Editor in MVP?

`Epic_Areas_Plan.md` lists it as MVP-in twice (lines 383, 408). Epic-07 lists
it as Post-MVP (line 78). The specs follow the epic (out).

Blocks: whether the four roles are fixed constants or runtime-editable data.
Retrofitting editable roles onto hard-coded ones is a rewrite of the
authorization layer.

*Recommendation*: keep it out, per the epic. Roles stay fixed constants.

### A3. Does a camp registrant who never creates an account appear in the CRM?

Epic-03 recognises exactly three ways a player joins a trainer — `sharelink`,
`event_registration`, `coach_invite` (BR-03-1, AC-03-68). Camp registration is
not among them, so even a registrant who *does* convert has no legal route
into the CRM, despite Epic-08 BR-08-17 promising auto-assignment to the
trainer.

Blocks: Epic-08 cannot be built correctly, and the CRM association-source rule
must be reopened either way.

### A4. Where does camp money live inside the platform?

Epic-08 takes real money through Stripe Checkout with the platform's 5% fee
(BR-08-8, BR-08-11, BR-08-12) from someone who has no account. Epic-05 never
mentions camps, evaluations, or Epic-08 anywhere, and its transaction records
require a parent/player account holder.

Blocks: the payment records design. Money already moves on this path, and
Epic-08 BR-08-13's manual refunds have nothing platform-side to reverse.

### A5. If a camp registrant later creates an account, does their earlier payment and registration attach to it?

Epic-08 has a "converted" marker and a nullable user reference, but no rule
says history follows. Blocks refunds and transaction history for converted
registrants.

### A6. Can a parent ever see one combined view across trainers?

Direct contradiction. Epic-01 AC-01-15 / BR-01-10: separated views per trainer
context, explicitly "no combined view". Epic-05 AC-05-24: an optional "Family
Overview" showing the 10 most recent transactions **across all children and
all trainers**.

Blocks: the entire context-switching rule, and it is the only place any spec
proposes crossing a trainer boundary on a player-facing screen. Tenancy
isolation is the highest-risk defect class in this product.

*Recommendation*: honour Epic-01 — no combined view — unless you want the
Family Overview, in which case it needs an explicit carve-out.

### A7. Must each token spend record which child it was for?

Token balances sit at the parent-trainer pair and are spendable for any of
that parent's children with that trainer (Epic-05 BR-05-2, AC-05-5, AC-05-9).
Nothing requires recording the beneficiary child.

Blocks: nothing today — which is the problem. Cheap to capture now,
unreconstructable later.

*Recommendation*: record the beneficiary player on every spend regardless.

### A8. Which content pricing model applies?

Epic-04's own paywall rules (D-SCOPE-011) say "(To be finalized with client)"
and list four candidates without choosing: per-playlist one-time purchase,
all-content subscription, bundle pricing, or inclusion with an event
subscription (BR-04-8). A second, separately-numbered client question inside
US-05.06 asks the same thing and is marked "Needs Dale approval".

Blocks: the content purchase flow and its Epic-05 integration.

### A9. Is "Coach-Only Content" in MVP, and what is the visibility model?

Asserted once as an in-scope bullet (Epic-04 lines 81-84) with no story, no
business rule and no data field. Playlist visibility is modelled as binary
Public/Private (line 601), not three-state.

Blocks: the content visibility model and the audience check.

### A10. Build order — where does Epic-08 sit?

Your brief specifies `01 → 02, 03, 04, 08 → 05 → 06, 07`, taken from the
dependency diagram in `Epic_Areas_Plan.md:15-32`. But Epic-08's own §5
requires Epic-05, and the plan's own caption three lines below the diagram
agrees (line 36). Epic-08 needs Stripe Checkout to exist.

Derived order from each epic's own "Required Before This Epic":
**01 → 02 → {03, 04} → 05 → {06, 07, 08}.**

*Recommendation*: adopt the derived order.

### A11. Epic-04 ↔ Epic-05 dependency cycle

Epic-05 §5 requires Epic-04 ("content must exist to purchase");
`Epic_Areas_Plan.md:318` says Epic-04 requires Epic-05.

*Recommendation*: build Epic-04's content structure first, then Epic-05, then
wire the paywall. Confirm, because it changes what "Epic-04 done" means.

### A12. Is US-02.09 a missing story or a numbering gap?

Epic-02 runs US-02.01–08 then US-02.10–15 = 14 stories. The plan's summary
table claims 15. `grep -rn 'US-02\.09'` across `Task/Epics/` returns nothing.
Platform total is 85 stories present against 86 claimed.

Blocks: nothing structurally, but if a story was lost, its criteria are lost
with it and no test will ever miss them.

---

## Section B — Client questions still open, not blocking design

Answerable during the epic that needs them. Each has a safe default that does
not constrain the schema.

| ID | Question | Epic | Default if unanswered |
|---|---|---|---|
| Q-01.01 | Skill level definitions (Beginner/Intermediate/Advanced/Elite or custom?) | 01 | Configurable lookup, not a hard-coded enum |
| Q-01.02 | How are age groups defined (birth year, ranges, grade levels?) | 01 | Store date of birth; derive groups at query time |
| Q-01.04 | Which automated emails are required? | 01 | Welcome, password reset, invitation, RSVP confirmation only |
| Q-01.05 | Email verification required before login, or optional? | 01 | Required before first login |
| Q-01.06 | Notify a coach when their availability is overridden? | 01 | Yes, notify |
| Q-01.07 | Session timeout / how long users stay logged in | 01 | 7 days |
| Q-02.02 | Auto-confirm coach assignments after 48h, or always explicit? | 02 | Always explicit |
| Q-02.08 | Event reminders 24h before — defer to post-MVP? | 02 | Defer |
| Q-02.09 | Can a trainer manually add a player to a full event? | 02 | Yes, with an override recorded in the audit log |
| Q-03.01 | Can players/parents view coach feedback? | 03 | No — the epic's own out-of-scope list already excludes it |
| Q-03.04 | Auto-flag "high no-show" — MVP or Phase 2? | 03 | Phase 2, per the epic's own business rules |
| Q-03.05 | Can coaches apply flags, or only trainers? | 03 | Trainers only |
| Q-03.07 | Label usage analytics | 03 | Defer |
| Q-04.03 | Content moderation for inappropriate public content | 04 | Defer; no public flagging in MVP |
| Q-04.06 | Playlist due-date reminder notifications | 04 | Defer |
| Q-04.09 | Can trainers schedule playlist assignments for a future date? | 04 | No; immediate assignment only |
| Q-04.10 | Notify trainers reusing public content when the original is edited? | 04 | Defer |
| Q-05.03 | Token package sizes and discount strategy | 05 | Seed the epic's illustrative packages as configuration |
| Q-06.10 | Coupon eligibility — new players only, existing too, or per-coupon? | 06 | Per-coupon setting (superset; keeps the field) |
| Q-06.11 | Referral reward ratio, and does the referee get a welcome token? | 06 | Super Admin configuration, seeded 1:1, no referee token |
| Q-06.12 | Confirm the 30-day referral attribution window | 06 | 30 days, configurable |
| Q-07.02 | Dashboard date range — presets enough, or custom picker? | 07 | Presets only |
| Q-08.01 | Maximum custom fields per form | 08 | 20, the epic's own recommendation |
| Q-08.02 | Form completion-rate analytics | 08 | Defer, per the epic's recommendation |
| Q-08.03 | Evaluation appointment time slots | 08 | Defer, per the epic's recommendation |
| Q-08.04 | Camp refund policy | 08 | Manual via Stripe, per the epic's recommendation |

One further ambiguity, worth an explicit confirmation because it changes what
a player is charged: **who bears the 5% platform fee.** Epic-05 states once,
explicitly, that the fee is included in the price and the trainer absorbs it
(BR-05-7, with two worked examples). Five other mentions of the fee omit that
qualifier. The specs follow "trainer absorbs".

---

## Section C — Already resolved by the epics

Recorded so no later session reopens them. Q-05.01 (24-hour refund window),
Q-05.02 (player subscriptions: yes, token-based), Q-05.04 (Stripe handles
retries), Q-05.05 (no partial refunds; use token gifting), Q-05.06 (no balance
alerts), Q-05.07 (no subscription grace period), Q-05.08 (trainer-to-coach
payments stay outside the platform), Q-06.13 (one coupon per purchase),
Q-07.01 (camps are in MVP as Epic-08), Q-07.03 (no bulk trainer operations).

---

## Section D — Source-document defects

No decision needed; these are reported so the client can correct the source.
The specs work around each one.

- **Section numbering.** Epic-01 uses §10, §11 and §12 twice each. Epics 03
  and 04 skip §14. Every back-reference in `specs/` therefore cites heading
  text plus line number rather than a section number.
- **Question-ID collisions.** "Q-01.05" labels both the email-verification
  question and the COPPA question. "Q-05.02" labels both the resolved
  subscriptions question and an unresolved content-pricing question.
  "Q-06.10" labels both coupon eligibility and an untabled question in
  US-06.05.
- **Question-ID gaps.** Epic-01 skips Q-01.03; Epic-02 skips six IDs; Epic-03
  skips three; Epic-04 skips six. Epic-04 cites "(per Q-P1-016)", an ID in no
  scheme used anywhere in the file.
- **Wrong cross-references.** Epic-01 points portal branding at US-01.12; it
  is US-01.14. Epic-01's closing summary says 12 user stories; there are 14.
  Epic-03 US-03.09 cites "(see US-03.11)" for session feedback; it is
  US-03.10.
- **Stale content in Epic-04.** The role table, the US-04.07 tab list, the
  Pillar/Type field values and the Super Admin analytics all still reference
  the PERFECT feedback pillar, removed from MVP.
- **Misfiled scope in Epic-05.** Player Subscriptions appear in full under
  "Out of Scope (Post-MVP)" while also being in scope in §3.
- **Plan-versus-epic conflicts.** Recurring events, event types, bulk
  operations, the User Role Editor, and Epic-03 export are each stated one way
  in `Epic_Areas_Plan.md` and the other way in the epic. The specs follow the
  epic in every case.
- **Plan defects.** Four mutually incompatible build orders; Epic-08 omitted
  from all four MVP delivery phases; the number "Epic 08" reused for a Phase-2
  Advanced Communication epic.
- **Undecided cutoffs the epics flag themselves.** Epic-02 marks the free-event
  cancellation cutoff and the trainer-cancellation cutoff both "policy TBD".
- **Scope items with no detail.** Epic-02 "Event notes (coach preparation
  notes)" and "Event analytics and reporting"; Epic-03 "Coach Hours Tracking"
  and search-by-attendance. Each is in scope with no story, criteria or data
  field.
- **Unreconciled duplicate targets.** Performance targets are stated twice and
  differently in Epics 01, 02 and 04. Epic-08 states both "<5 minutes" and
  "<3 minutes" for form completion, and two different "20%+" camp metrics.
- **Terminology.** Epic-03 uses "event" and "session" interchangeably for the
  same attendance unit without saying whether they are the same thing.
