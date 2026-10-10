# Spec: Epic-06 — Marketing & Growth Tools

Source: `Task/Epics/Epic-06_Marketing_Growth_Tools_SPEC.md` (read-only client material)

> Citation note: as in the Epic-01, Epic-02, and Epic-05 specs, every
> back-reference below cites heading **text** plus a line number, never a
> bare section number. Epic-06's own numbered sections (§1–§14) are not
> themselves duplicated, but most of its rule/criteria detail lives under
> **bold sub-labels** (e.g. "Multi-Trainer Context," "Attribution Rules,"
> "Platform-Wide Reward Rules") rather than markdown headings. Those are
> cited as a second `§` level — `§ "Parent Heading" § "Sub-label"` — the
> same convention the earlier specs used for bold sub-labels.

## Problem

Epic-06 gives trainers tools to grow their player base through referrals
and promotional campaigns — enabling word-of-mouth marketing and
incentivizing player acquisition through the "Get the Assist" referral
program and a discount coupon system ("Purpose", line 7). It exists so
trainers get organic growth at a reduced customer-acquisition cost, players
get rewarded for referring friends and discounted for joining or attending,
and the platform owner benefits from network effects — more players driving
more transactions and more platform revenue ("Business Value", lines
11-13). Success is judged against five concrete targets: 40%+ of new
players arriving via referral within 3 months, an average of 2+ referrals
per active player per year, a 60%+ referral-to-first-purchase conversion
rate, 30%+ of trainers actively using coupon codes, and an average 10%+
revenue increase from promotional campaigns ("Success Metrics", lines
16-20).

The epic states its own dependency order plainly: it requires Epic-01's
ShareLink system for referral tracking and Epic-05's token system for
rewards plus payment processing for the first-purchase trigger, and it
blocks nothing downstream, so it can be built in parallel with Epic-07
("Dependencies — Required Before This Epic", lines 80-82; "...Blocks These
Epics", line 85). It has no external dependencies beyond the platform's own
internal systems ("...External Dependencies", line 88). The platform's own
planning document independently rates it "Medium" complexity — "Tracking
logic, discount application, but manageable scope" (`Epic_Areas_Plan.md` §
"Epic 06: Marketing & Growth Tools" § "Estimated Complexity", line 370) —
and states the same two-epic dependency (`Epic_Areas_Plan.md` § "...
Dependencies", lines 366-367).

Three roles interact with marketing and growth tools, not the platform's
full four-role set: the Trainer/Business Owner creates coupons and views
their own referral analytics; the Player/Parent refers friends and applies
coupons; and the Super Admin configures platform-wide referral rules and
can see all referral/coupon data across every trainer ("User Roles
Involved", lines 94-98). The epic's role table simply never mentions the
Coach role — no stated Coach permission and no stated Coach exclusion — so
this spec treats Coaches as having no interaction with marketing/growth
tools at all: a narrower subset of Epic-01's 4-role MVP set, not a conflict
with it.

## User scenarios

1. **Player** has an automatic referral link so that they can invite
   friends and earn rewards.
   Path: every player automatically has a referral link with no generation
   step, in the format `platform.com/join/{trainer-slug}/{player-id}` (or
   similar); they view it from their profile or dashboard's "Get the
   Assist" section, which shows the link, a "Share" button that copies it
   to the clipboard, and referral-count stats; a player training with
   multiple trainers has a separate link, and a separate referral count,
   per trainer.
   Source: US-06.01 "Player Has Automatic Referral Link" (line 104)

2. **Player** refers a friend so that the friend can join their trainer's
   program and they can earn a reward.
   Path: the player copies their referral link and shares it outside the
   platform (text, email, social media); the friend clicks it and is taken
   to the registration page for that specific trainer; the system records
   the click and, after the friend registers, the referral as "Pending,"
   subject to a 30-day, last-click, cookie-based attribution window.
   Source: US-06.02 "Player Refers Friend" (line 130)

3. **Player** receives a reward when a referral makes their first purchase
   so that they're incentivized to keep referring friends.
   Path: when the referee makes their first purchase (an event RSVP, a
   content purchase, or a token purchase), the system looks up who referred
   them, checks both accounts are active, and increments the referrer's
   assist count with that trainer; once the count reaches the
   Super-Admin-configured platform-wide threshold, the referrer receives a
   token added to their balance with that trainer plus a notification, and
   the counter resets toward the next reward.
   Source: US-06.03 "System Awards Referral Rewards (First Purchase
   Trigger)" (line 159)

4. **Trainer** views a referral dashboard so that they can recognize and
   reward their top advocates.
   Path: from "Marketing" → "Referrals" ("Get the Assist"), the trainer
   sees overview metrics (referrals and conversions this month, conversion
   rate, referral revenue), a Top Referrers leaderboard, and a Referral
   Activity Log; the platform surfaces the data only — any actual reward to
   a top referrer beyond the token system happens outside the platform, at
   the trainer's discretion.
   Source: US-06.04 "Trainer Views Referral Dashboard" (line 198)

5. **Trainer** creates a discount coupon code so that they can run
   promotions and attract new players.
   Path: from "Marketing" → "Coupons," the trainer clicks "Create Coupon,"
   fills in a code, discount type and value, what it applies to, a usage
   limit, an optional expiration date, and a status; the system validates
   the code's uniqueness and format, and the coupon then appears in the
   trainer's coupon list, where they can view, edit, deactivate, or share
   it.
   Source: US-06.05 "Trainer Creates Coupon Code" (line 235)

6. **Player or Parent** applies a coupon code to their purchase so that
   they get a discount.
   Path: at checkout for an event RSVP or content purchase, the player
   enters a code and clicks "Apply"; the system validates it (exists for
   this trainer, active, unexpired, under its usage limit, player
   eligible); if valid, the discounted price is shown and charged, with
   Stripe receiving only the final amount; if invalid, an error is shown
   and no discount applied; only one coupon can be used per transaction.
   Source: US-06.06 "Player Uses Coupon Code" (line 273)

7. **Trainer** views coupon analytics so that they can measure campaign
   effectiveness.
   Path: from "Marketing" → "Coupons," the trainer sees a per-coupon
   breakdown (status, uses, usage limit, discount given, revenue generated,
   conversion rate, dates) plus an all-coupons summary (coupons created,
   active coupons, uses/discount/revenue this month), and can edit,
   deactivate, delete an unused coupon, or drill into usage details.
   Source: US-06.07 "Trainer Views Coupon Analytics" (line 324)

8. **Super Admin** configures the platform-wide referral reward ratio so
   that the referral program stays consistent across every trainer.
   Path: from "System Settings" → "Referral Program," Super Admin views the
   current rule, clicks "Edit," changes the referrals-required and
   tokens-awarded values (and optionally toggles a referee welcome bonus),
   and saves; the new rule applies to every trainer immediately, players'
   existing assist-count progress is preserved under the new ratio, and the
   change is audit-logged.
   Source: US-06.08 "Super Admin Configures Referral Reward Rules" (line
   360)

## Acceptance criteria

Every checkbox below is renumbered sequentially as AC-06-1 .. AC-06-33 from
the epic's per-story "Acceptance Criteria" lists (including each story's
other bold-labeled sub-lists that state testable behavior, e.g.
"Multi-Trainer Context," "Attribution Rules," "Platform-Wide Reward
Rules"), the epic-level "Acceptance Criteria (Epic Level)" section, and one
item with no story equivalent (analytics data freshness). The 11
checkboxed goals in §3 "In Scope (MVP)" (lines 47-59) were checked against
the criteria below and found fully restated there with no additional
acceptance-relevant detail, so they are not separately numbered — each is
cited alongside the story criterion it restates instead. Where an
epic-level criterion restates a story criterion, the story version is kept
and both origins are cited.

**US-06.01 — Player Has Automatic Referral Link** (line 104)
- [ ] **AC-06-1** Every player automatically has a referral link with no "generate" action needed, in the format `platform.com/join/{trainer-slug}/{player-id}` (or similar), which is always active with no expiration and unique per player-trainer relationship — `Epic-06_Marketing_Growth_Tools_SPEC.md` § "US-06.01: Player Has Automatic Referral Link" (lines 111-112, 115-116)
- [ ] **AC-06-2** The player can view their referral link from their profile or dashboard, in a "Get the Assist" section that shows the link, a "Share" button that copies it to the clipboard, and referral-count stats (e.g. "You've referred 3 players (2 converted)") — § "US-06.01..." (lines 113-114, 124-126)
- [ ] **AC-06-3** A player who trains with multiple trainers has a separate referral link per trainer (example: Sarah has one link tied to Coach Bob and a different link tied to Coach Lisa), and each referral is credited to that specific trainer's program — § "US-06.01..." § "Multi-Trainer Context" (lines 119-121)

**US-06.02 — Player Refers Friend** (line 130)
- [ ] **AC-06-4** The player copies their referral link and shares it outside the platform (text, email, social media); when a friend clicks it, they are taken to the registration page for that specific trainer — § "US-06.02: Player Refers Friend" (lines 137-140)
- [ ] **AC-06-5** The system tracks the link click (timestamp, referrer, referee), and after the friend registers, records the referral as "Pending," awaiting the friend's first purchase — § "US-06.02..." (lines 141-142; also § "In Scope (MVP)" § "Referral System" "Referral tracking via ShareLink system", line 48)
- [ ] **AC-06-6** Referral attribution follows a 30-day window from link click to registration, after which attribution expires (stated as an assumption pending confirmation — see Open questions, Q-06.12); attribution is last-click (if a friend clicks more than one referral link, the most recent one wins); referrer information is stored in a browser cookie for 30 days — § "US-06.02..." § "Attribution Rules" (lines 145-151)
- [ ] **AC-06-7** If the friend already has an account, the link still works but no referral credit is given; if the friend registers with a different trainer than the one whose link they clicked, no referral credit is given — § "US-06.02..." § "Edge Cases" (lines 154-155)

**US-06.03 — System Awards Referral Rewards (First Purchase Trigger)** (line 159)
- [ ] **AC-06-8** When the referee (Player B) makes their FIRST PURCHASE (event RSVP, content purchase, or token purchase), the system detects that it is Player B's first purchase, looks up who referred Player B (Player A), and checks that both Player A and Player B are active — § "US-06.03: System Awards Referral Rewards (First Purchase Trigger)" § "Reward Trigger" (lines 166-169; also epic-level AC § "Acceptance Criteria (Epic Level)" § "Referral tracking works across registration and first purchase", line 642; also § "In Scope (MVP)" § "Referral System" "Token rewards on referral's first purchase", line 50)
- [ ] **AC-06-9** Super Admin configures a platform-wide reward rule of the form "X referrals who purchase = Y token(s)" (illustrated in the epic as "3 referral purchases = 1 token"), which applies to ALL trainers with no per-trainer customization for MVP; the exact ratio is unresolved (see Open questions, Q-06.11) — § "US-06.03..." § "Platform-Wide Reward Rules (Simplified)" (lines 172-173, 178; also epic-level AC § "...Platform-wide rule configurable by Super Admin", line 644; also § "In Scope (MVP)" § "Referral System" "Platform-wide referral reward rules (Super Admin configures)", line 49)
- [ ] **AC-06-10** Once the threshold is reached, Player A (the referrer) receives a token added to their balance with that specific trainer, and a notification is sent ("You earned 1 token! Your friend [Name] just joined."); whether the referee (Player B) also receives a token is optional and unresolved (see Open questions, Q-06.11) — § "US-06.03..." § "Token Grant" (lines 181-184)
- [ ] **AC-06-11** The system tracks an "assist count" per player-trainer relationship, incrementing it on each qualifying referral purchase (example: 7 total assists under a 3:1 rule equals 2 tokens earned, with 1 assist toward the next) — § "US-06.03..." § "Tracking" (lines 187-189; also epic-level AC § "...Token rewards granted when assist count reaches threshold", line 643)
- [ ] **AC-06-12** The reward triggers ONLY on the referee's first purchase, not on every purchase; a player cannot refer themselves; and a fake account cannot trigger a reward because a real payment is required — § "US-06.03..." § "Anti-Abuse" (lines 192-194)

**US-06.04 — Trainer Views Referral Dashboard** (line 198)
- [ ] **AC-06-13** The trainer navigates to "Marketing" → "Referrals" (or "Get the Assist") to view the referral dashboard — § "US-06.04: Trainer Views Referral Dashboard" (line 205; also § "In Scope (MVP)" § "Referral System" "Referral dashboard for trainers", line 51)
- [ ] **AC-06-14** The dashboard's overview metrics show total referrals this month, total conversions this month, conversion rate (%), and total referral revenue (the sum of first purchases from referrals) — § "US-06.04..." § "Overview Metrics" (lines 209-212; also epic-level AC § "...Referral dashboard shows accurate stats", line 645, and § "...Referral dashboard shows: conversions, top referrers, activity log", line 656)
- [ ] **AC-06-15** The dashboard shows a "Top Referrers" leaderboard listing, per player, total referrals sent, total conversions, conversion rate (%), and last referral date — § "US-06.04..." § "Top Referrers (leaderboard)" (lines 215-219; also epic-level AC, line 656; also § "In Scope (MVP)" § "Referral System" "Referral stats per player", line 52)
- [ ] **AC-06-16** The dashboard shows a Referral Activity Log listing, per entry, date, referrer, referee, status (Pending, Converted, or Rewarded), and first purchase amount if converted — § "US-06.04..." § "Referral Activity Log" (lines 222-226; also epic-level AC, line 656)

**US-06.05 — Trainer Creates Coupon Code** (line 235)
- [ ] **AC-06-17** The trainer navigates to "Marketing" → "Coupons," clicks "Create Coupon," and fills a form capturing code, discount type (percentage or fixed amount), discount value, what the coupon applies to (Events, Content, or Both — the epic itself flags this as "needs confirmation"; see Open questions), a usage limit (unlimited, single-use, or multi-use), an optional expiration date, and a status (active or inactive) — § "US-06.05: Trainer Creates Coupon Code" (lines 242-251; also epic-level AC § "...Trainers can create, edit, deactivate coupons", line 648; also § "In Scope (MVP)" § "Coupon System" (lines 55-57))
- [ ] **AC-06-18** Coupon creation is validated: the code must be unique within the trainer's own coupons, 4-20 characters (alphanumeric plus hyphens), the discount value must be greater than 0, percentage discounts must be 1-100%, fixed discounts are checked against the event/content price at redemption (cannot exceed it), and an expiration date, if set, must be a future date — § "US-06.05..." § "Validation" (lines 254-259)
- [ ] **AC-06-19** After creation, the coupon appears in the trainer's coupon list; the trainer can view, edit, or deactivate it there, and share the code with players outside the platform (social media, website, etc.) — § "US-06.05..." § "After Creation" (lines 262-264; also epic-level AC, line 648)

**US-06.06 — Player Uses Coupon Code** (line 273)
- [ ] **AC-06-20** At checkout (event RSVP or content purchase), the payment screen shows a "Have a coupon code?" prompt with a text input and an "Apply" button — § "US-06.06: Player Uses Coupon Code" (lines 280-283; also epic-level AC § "...Players can apply coupons at checkout", line 649)
- [ ] **AC-06-21** When the player enters a code and clicks "Apply," the system validates that the code exists for this trainer, is active, has not expired, has not reached its usage limit, and that the player is eligible — eligibility itself is an unresolved rule (see Open questions, Q-06.10) — § "US-06.06..." § "Apply Coupon Flow" (lines 286-293; also epic-level AC § "...Expiration dates enforced", line 652)
- [ ] **AC-06-22** If valid, the discount is applied in-platform, the price updates on screen (example: "$20 → $16 (20% off with SUMMER20)"), and payment proceeds at the final discounted price; Stripe receives only the final discounted amount, with no coupon details sent to Stripe — § "US-06.06..." § "Apply Coupon Flow" § "If valid" (lines 294-299; also epic-level AC § "...Discounts calculated correctly (percentage and fixed)", line 650; also restated, oddly filed under § "Questions / Open Issues" § "Coupon Implementation", lines 607-611 — see Open questions, analyst-raised)
- [ ] **AC-06-23** If invalid, an "Invalid or expired code" error is shown and no discount is applied — § "US-06.06..." § "Apply Coupon Flow" § "If invalid" (lines 300-302)
- [ ] **AC-06-24** Only one coupon can be applied per transaction; multiple codes cannot be combined — § "US-06.06..." § "Coupon Stacking" (lines 314-315)
- [ ] **AC-06-25** Each coupon use is logged (player, event/content, discount amount, timestamp) and increments the coupon's usage count; when the usage limit is reached, the code auto-deactivates — § "US-06.06..." § "Tracking" (lines 318-320; also epic-level AC § "...Usage limits enforced", line 651; also § "In Scope (MVP)" § "Coupon System" "Coupon redemption tracking", line 58)

**US-06.07 — Trainer Views Coupon Analytics** (line 324)
- [ ] **AC-06-26** The trainer navigates to "Marketing" → "Coupons" to view a list of all coupons, each showing code, discount, status (Active, Expired, or Inactive), total uses, usage limit (if set), total discount given, revenue generated, conversion rate, and created/expiration dates — § "US-06.07: Trainer Views Coupon Analytics" § "Per Coupon" (lines 335-343; also epic-level AC § "...Coupon analytics accurate", line 653, and § "...Coupon analytics shows: usage, discount given, revenue impact", line 657; also § "In Scope (MVP)" § "Coupon System" "Coupon analytics", line 59)
- [ ] **AC-06-27** Per coupon, the trainer can edit it (expiration, usage limit, status), deactivate it (stop new uses), delete it if it has never been used, or view usage details (the list of players who used it) — § "US-06.07..." § "Actions" (lines 346-349)
- [ ] **AC-06-28** An analytics summary across all coupons shows total coupons created, active coupons, total coupon uses this month, total discount given this month, and revenue from coupon users this month — § "US-06.07..." § "Analytics Summary (all coupons combined)" (lines 352-356; also epic-level AC, lines 653, 657)

**US-06.08 — Super Admin Configures Referral Reward Rules** (line 360)
- [ ] **AC-06-29** Super Admin navigates to "System Settings" → "Referral Program," views the current rule (e.g. "3 referral purchases = 1 token"), clicks "Edit," and modifies "Referrals Required," "Tokens Awarded," and an optional "Reward Referee Too" checkbox (whether the new player also gets a welcome token), then saves — § "US-06.08: Super Admin Configures Referral Reward Rules" (lines 367-374)
- [ ] **AC-06-30** On save, the rule applies to ALL trainers immediately, and existing per-player assist counts are preserved across the change (example: a player with 2 assists under an old 3:1 rule keeps those 2 assists under a new 5:1 rule) — § "US-06.08..." (lines 375-376)
- [ ] **AC-06-31** Every change to the platform-wide referral rule is audit-logged: who changed it, when, the old value, and the new value — § "US-06.08..." § "Audit Logging" (line 379)

**Epic-level (from "Acceptance Criteria (Epic Level)", line 638) — not already covered above**
- [ ] **AC-06-32** Referral and coupon analytics data updates in near real-time, with up to a 5-minute delay considered acceptable — § "Acceptance Criteria (Epic Level)" § "Analytics" (line 658)
- [ ] **AC-06-33** Performance and scale targets: referral link display under 100ms, coupon validation under 200ms, referral dashboard load under 2 seconds, and token reward processing under 5 seconds (asynchronous, not blocking the purchase); the system supports 1,000 referral clicks per day, 100 concurrent coupon redemptions, and 500 referral rewards processed per day; and it scales to 10,000+ referrals per trainer, 1,000+ active coupons platform-wide, and referral analytics across 100,000+ players — § "Performance & Scale Targets" (lines 577-591; also epic-level AC § "...Performance", lines 661-664 — see Open questions, analyst-raised, for a wording variance between the two)

**Final count: 33 acceptance criteria (AC-06-1 .. AC-06-33)**, covering all
8 user stories (US-06.01–US-06.08) and the epic-level "Acceptance Criteria
(Epic Level)" section, with epic-level restatements folded into their
matching story criterion rather than duplicated.

## Business rules

Restated from "Business Rules & Logic" (line 443). The epic groups these
under three named categories — Referral Attribution, Referral Reward, and
Coupon — all three are represented below; none are dropped.

**Referral Attribution Rules** (line 445)
- [ ] **BR-06-1** Attribution runs on a 30-day window from link click to registration; after 30 days, attribution expires (stated as an assumption pending confirmation — see Open questions, Q-06.12) — `Epic-06_Marketing_Growth_Tools_SPEC.md` § "Business Rules & Logic — Referral Attribution Rules" § "Attribution Window" (lines 447-449)
- [ ] **BR-06-2** Attribution is last-click — the most recent referral link a friend clicked wins if they clicked more than one — tracked via a browser cookie stored for 30 days — § "...Referral Attribution Rules" § "Attribution Method" (lines 451-453)
- [ ] **BR-06-3** No referral credit is given if the friend already has an account, or if the friend registers via a different trainer than the one whose link they clicked; self-referral is blocked outright — § "...Referral Attribution Rules" § "Edge Cases" (lines 455-458)

**Referral Reward Rules** (line 460)
- [ ] **BR-06-4** Super Admin sets a single platform-wide rule ("X referrals who purchase = Y tokens"); it applies to ALL trainers, with no per-trainer customization for MVP — § "...Referral Reward Rules" § "Platform-Wide Rule (Simplified)" (lines 462-465)
- [ ] **BR-06-5** The reward triggers when the referee makes their FIRST PURCHASE; the system increments the referrer's assist count; when the count reaches the configured threshold, a token is granted, and the counter resets and repeats, so a player can earn unlimited tokens over time — § "...Reward Trigger" (lines 467-471)
- [ ] **BR-06-6** The token is added to the referrer's balance with that specific trainer, and a notification is sent; whether the referee also receives a welcome-bonus token is optional and unresolved (see Open questions, Q-06.11) — § "...Reward Delivery" (lines 473-476)
- [ ] **BR-06-7** Assists are tracked per player-trainer relationship, not globally — the same player referring people to two different trainers accrues two separate, independent assist counts — § "...Multi-Trainer Context" (lines 478-481)

**Coupon Rules** (line 483)
- [ ] **BR-06-8** A coupon is valid for redemption only if it is active, not expired (when an expiration date is set), has not reached its usage limit (when set), and the player is eligible under the eligibility rule — which is itself unresolved (see Open questions, Q-06.10) — § "...Coupon Rules" § "Validation" (lines 485-489)
- [ ] **BR-06-9** Percentage discounts apply to the original price (example: $20 event, 20% off = $16); fixed discounts subtract from the original price (example: $20 event, $5 off = $15); the discounted price floors at $0 and can never go negative (example: $5 event, $10 off = $0, free) — § "...Discount Application" (lines 491-497)
- [ ] **BR-06-10** Only one coupon may be used per transaction; coupon codes cannot be combined or stacked — § "...Coupon Stacking" (lines 499-501)
- [ ] **BR-06-11** A coupon created by Trainer A only works for Trainer A's own events/content; players cannot redeem Trainer A's coupon against Trainer B's events or content — § "...Trainer Scope" (lines 503-505)

### Integration points (cross-epic dependency notes)

Restated from the epic's own "Integration Points" section (line 615), added
as an explicit sub-list per this run's brief.

**With Epic-01 (User Management)** (line 617):
- ShareLink system tracks referral attribution
- Player registration completes referral tracking
- Multi-trainer context affects referral links

**With Epic-05 (Payments)** (line 622):
- First purchase triggers referral reward
- Coupon codes apply discounts in-platform (Stripe receives final discounted amount)
- Token rewards credited to player-trainer balance
- Coupon discounts reflected in transaction logs

**With Epic-02 (Event Management)** (line 628):
- Coupons apply to event RSVPs
- Discounted prices processed at checkout

**With Epic-04 (LPPP Content)** (line 632):
- Coupons apply to content purchases
- Discounted content access processed

Cross-check: Epic-05's own spec states the same relationship from its side
with no contradiction — its "Business rules" § "Integration points
(cross-epic dependency notes)" § "With Epic-06 (Marketing Tools)"
(`requirements-analyst-epic-05-payments-tokens-spec.md`, citing
`Epic-05_Payments_Tokens_SPEC.md` line 811) lists the same three facts:
referral rewards trigger token grants, coupon codes apply discounts to
payments, first purchase triggers referral reward.

## Data requirements

Restated from "Data Requirements" (line 387) — entities, the fields the
epic names, and the relationships it implies from those fields. No schema,
keys, or types are proposed here.

- **Referral**: referrer (Player A) account reference, referee (Player B) account reference, trainer reference, referral date (link click / registration completed), attribution source (referral link ID), status (Pending, Converted, Rewarded), first purchase date (if converted), first purchase amount (if converted). Relationship: references a referrer Player, a referee Player, and a Trainer. (line 391)
- **Assist count** (per player-trainer): player reference, trainer reference, total assists (count of referrals who purchased), tokens earned from referrals, last assist date. Relationship: one running count per Player-and-Trainer pair. (line 401)
- **Coupon**: coupon unique identifier, trainer reference, code (unique per trainer), discount type (percentage or fixed), discount value, applies to (events, content, or both), usage limit (optional), current usage count, expiration date (optional), status (active, inactive, expired), created timestamp, created by (trainer). Relationship: belongs to a Trainer. (line 408)
- **Coupon usage**: usage unique identifier, coupon reference, player reference, transaction reference (event RSVP or content purchase), discount amount applied, original price, final price after discount, usage timestamp. Relationship: references a Coupon, a Player, and a transaction (Epic-02 event RSVP or Epic-04 content purchase, settled via Epic-05 payment processing). (line 422)
- **Referral reward**: reward unique identifier, player reference (who received the reward), trainer reference, reward type (token), reward amount (1 token), trigger event (the referral ID that triggered the reward), timestamp. Relationship: references a Player, a Trainer, and the triggering Referral. (line 432)

## Edge cases

| Case | Expected |
|---|---|
| Friend already has an account when they click a referral link | Link still works, but no referral credit is given — US-06.02 § "Edge Cases" (line 154) |
| Friend registers with a different trainer than the one whose link they clicked | No referral credit — US-06.02 § "Edge Cases" (line 155); Business Rules § "Referral Attribution Rules" § "Edge Cases" (line 457) |
| Player attempts to refer themselves | Blocked (self-referral) — Business Rules § "...Edge Cases" (line 458); US-06.03 § "Anti-Abuse" (line 193) |
| Friend clicks the referral link but registers more than 30 days later | Attribution has expired; no referral credit (30-day window stated as an assumption — see Open questions, Q-06.12) — US-06.02 § "Attribution Rules" (lines 145-147); Business Rules § "...Attribution Window" (lines 448-449) |
| Friend clicks multiple different referral links before registering | Last-click wins — the most recently clicked link is credited — US-06.02 § "Attribution Rules" (lines 148-149); Business Rules § "...Attribution Method" (line 452) |
| Referee makes a second (or later) purchase after their first | No further reward triggers — the reward triggers ONLY on the referee's first purchase — US-06.03 § "Anti-Abuse" (line 192) |
| Coupon entered at checkout is invalid, inactive, expired, over its usage limit, or the player is ineligible | "Invalid or expired code" error shown; no discount applied — US-06.06 § "Apply Coupon Flow" § "If invalid" (lines 300-302) |
| Coupon's usage count reaches its usage limit | Code auto-deactivates — US-06.06 § "Tracking" (line 320) |
| A fixed-amount discount exceeds the item's price | Discounted price floors at $0, never negative (example: $5 event, $10 off = $0) — Business Rules § "Coupon Rules" § "Discount Application" (lines 496-497) |
| Player attempts to apply a second coupon to the same transaction | Blocked — only one coupon per transaction, codes cannot be combined — US-06.06 § "Coupon Stacking" (lines 314-315); Business Rules § "...Coupon Stacking" (lines 499-501) |
| Player attempts to redeem Trainer A's coupon against Trainer B's event or content | Blocked — coupons are scoped to the trainer that created them — Business Rules § "Coupon Rules" § "Trainer Scope" (lines 503-505) |
| Super Admin changes the platform-wide referral reward ratio | Applies to all trainers immediately; each player's existing assist-count progress carries over under the new ratio rather than resetting — US-06.08 (line 376); User Flows § "Flow 3: Super Admin Adjusts Referral Reward Ratio" (lines 569-571) |

Boundary/failure behaviors the epic raises but does **not** state an
expected outcome for are listed in Open questions instead of guessed here:
whether a reward is skipped, deferred, or forfeited when the "are both
Player A and Player B active?" check (US-06.03, line 169) fails; whether a
"Pending" referral that never converts eventually expires; and what happens
if a trainer attempts to delete a coupon that has already been used.

## Out of MVP scope

Two overlapping-but-not-identical exclusion lists appear in the epic; both
are merged here without dropping either (see Open questions, analyst-raised,
for where they diverge).

Verbatim from "Scope Summary" § "Non-Goals (Post-MVP)" (line 33):
- Email list export (Phase 2)
- Campaign management (grouping multiple coupons)
- Automated campaigns (e.g., "Welcome discount for all new players")
- Social media integration
- Affiliate marketplace (trainer-to-trainer referrals)
- SMS marketing
- A/B testing for campaigns

Also verbatim from "Out of Scope (Post-MVP)" § "Phase 2 Deferrals" (line
65), items not already listed above:
- Per-trainer referral reward customization (platform-wide only for MVP)
- Alternative reward tracking (T-shirts, etc.) - trainers handle outside platform
- Referral contests/leaderboards

(The remaining "Phase 2 Deferrals" items — email list export, campaign
management, automated campaigns, social media sharing integration, SMS
campaigns, trainer-to-trainer affiliate program — restate the "Non-Goals"
list above under slightly different wording and are not repeated twice
here.)

## Open questions

### From the epic's "Questions / Open Issues" section (line 595, verbatim)

| ID | Question | Priority | Status | Owner |
|:---|:---|:---:|:---|:---|
| Q-06.10 | **Coupon eligibility**: Are coupons for new players only, existing players too, or trainer decides per coupon? | P1 | Open | Client |
| Q-06.11 | **Referral reward ratio**: What should the platform-wide rule be? (3:1? 5:1? 1:1?) Should referee also get welcome token? | P1 | Open | Client |
| Q-06.12 | **Attribution window**: 30-day assumption for referral link validity - confirm? | P2 | Open | Client |
| Q-06.13 | **Coupon stacking**: Can players use multiple coupons on one purchase? (Assume NO) | P2 | ✅ RESOLVED | Client |

**Resolutions** (line 604, verbatim):
- **Q-06.13**: NO - Players can only use one coupon per purchase

**Coupon Implementation** (line 607, verbatim — not itself a question; see
analyst-raised below):
- Coupons applied within platform (before Stripe)
- Platform calculates final discounted price
- Stripe receives final amount only (doesn't know about coupons)
- Coupon tracking and analytics handled in platform

### Analyst-raised

- **(analyst-raised)** ID mistagging: "Q-06.10" is used for two different
  questions. Four of its five occurrences consistently mean "coupon
  eligibility: are coupons for new players only, existing players too, or
  trainer's choice?" — the table (line 599), the redemption-time validation
  check (line 293), the dedicated "Eligibility Rules" sub-section (line
  304), and the Business Rules restatement (line 489). The fifth
  occurrence, inside US-06.05's coupon-creation form field list, tags a
  completely different, un-tabled question — whether a coupon's "Applies
  To" scope is Events, Content, or Both — as "(Q-06.10 - needs
  confirmation)" (line 248). Unlike the eligibility question, which at
  least floats Option B ("any player can use") as a "default assumption"
  (line 309), the "Applies To" question has no stated default anywhere in
  the epic. This spec keeps Q-06.10 as the table defines it (coupon
  eligibility) in the citations above, and treats the "Applies To" scope as
  a second, genuinely open, currently un-numbered question. This mirrors
  the Q-01.05 and Q-05.02 ID collisions the Epic-01 and Epic-05 specs
  flagged.
- **(analyst-raised)** Per this run's brief: `Epic_Areas_Plan.md`'s MVP
  Scope checklist for this epic states "Platform-wide referral ratio
  (trainer views stats)" (`Epic_Areas_Plan.md` § "Epic 06: Marketing &
  Growth Tools" § "MVP Scope", line 351), which reads as trainers having
  some read-only visibility into the platform-wide ratio itself. The epic
  file never states this: US-06.08 "Super Admin Configures Referral Reward
  Rules" is Super-Admin-only with no trainer view mentioned, and the
  Trainer Referral Dashboard (US-06.04) lists Overview Metrics, a Top
  Referrers leaderboard, and a Referral Activity Log — but not the current
  ratio/rule itself anywhere in its stated content (lines 208-226). Per the
  epic-file-wins rule this spec does not add a trainer-facing view of the
  ratio as a criterion, but whether trainers are meant to see the ratio
  value (so they understand what their players are working toward), or
  whether it is meant to stay entirely invisible to trainers with only its
  downstream effects (conversions, tokens) visible, is unresolved.
- **(analyst-raised)** Two exclusion lists overlap but do not match:
  "Scope Summary" § "Non-Goals (Post-MVP)" (line 33) includes "A/B testing
  for campaigns," which does not appear in "Out of Scope (Post-MVP)" §
  "Phase 2 Deferrals" (line 65); conversely, that second list includes
  "Per-trainer referral reward customization," "Alternative reward tracking
  (T-shirts, etc.)," and "Referral contests/leaderboards," none of which
  appear in the first. This spec's Out of MVP scope section merges both
  without dropping either; a future edit to one list should check the
  other. (This mirrors the same kind of two-non-matching-lists finding the
  Epic-05 spec raised for its own out-of-scope sections.)
- **(analyst-raised)** "Coupon Implementation" (lines 607-611) is settled,
  implementation-confirming content — not a question, not phrased as one —
  filed under the "Questions / Open Issues" heading (§12). It restates
  content already established via US-06.06 (AC-06-22): that coupons are
  applied in-platform before Stripe and Stripe never sees coupon details.
  This mirrors the filing-error pattern the Epic-01 spec found for portal
  branding and the Epic-05 spec found for Player Subscriptions.
- **(analyst-raised)** US-06.03's reward-trigger sequence states as a check
  that the system "checks: Are both Player A and Player B active?" (line
  169), but never states what happens if that check fails — is the reward
  silently skipped, deferred until both accounts are active again, or
  permanently forfeited? No outcome is stated either way.
- **(analyst-raised)** Whether a "Pending" referral (registered but not yet
  converted) ever expires is unstated. The 30-day attribution window
  (Q-06.12) governs only the click-to-registration step; nothing in the
  epic says whether a friend who registers but never makes a first purchase
  leaves the referral "Pending" indefinitely, or whether "Pending" status
  itself eventually lapses.
- **(analyst-raised)** US-06.07 states a coupon can be deleted "if never
  used" (line 348) but never states what happens if a trainer attempts to
  delete a coupon that has been used — blocked with an error, forced to
  deactivate instead, or something else is not specified.
- **(analyst-raised)** The relationship between the per-player referral
  link and Epic-01's ShareLink entity is unclear. The epic states referral
  tracking works "via ShareLink system" (§3 "In Scope (MVP)" § "Referral
  System," line 48) and lists Epic-01's ShareLink system as a hard
  dependency (§5 "Dependencies" § "Required Before This Epic," line 81),
  yet the referral link's own stated format —
  `platform.com/join/{trainer-slug}/{player-id}` (US-06.01, line 112) — is
  a readable, composed URL, not the "unique URL-safe code" that Epic-01's
  own ShareLink data model describes
  (`requirements-analyst-epic-01-user-management-spec.md`, Data
  requirements, "ShareLink" entry). It is unclear whether every player
  automatically receives their own Epic-01 ShareLink record (reusing that
  exact entity, with a different code format than previously documented),
  or whether Epic-06 introduces a second, related-but-distinct link
  mechanism that only reuses ShareLink's tracking/analytics plumbing. This
  spec does not resolve which; that is an architect-stage decision.
- **(analyst-raised)** A performance target is worded two different ways in
  two places: "Support 1,000 referral clicks per day" ("Performance &
  Scale Targets" § "Throughput," line 584) versus "System handles 1,000
  referrals per day" ("Acceptance Criteria (Epic Level)" § "Performance,"
  line 664). A link "click" and a completed "referral" are explicitly
  different events elsewhere in this same epic (US-06.02 distinguishes the
  click from the resulting "Pending" referral record, lines 139-142), so it
  is unclear whether these two figures describe the same 1,000/day ceiling
  measured two different ways, or two different ceilings.
- **(analyst-raised)** `Epic_Areas_Plan.md`'s cross-epic reference section
  states that Epic-07 provides trainer-level "Feature toggles" covering
  "Epic-04, Epic-06, Epic-08" (`Epic_Areas_Plan.md` § "Cross-Epic
  References & Integration Points" § "Epic-07 → Uses All Epics:" § "Key
  Integration Points," line 665), implying Super Admin/Epic-07 can enable
  or disable marketing tools per trainer. Epic-06's own "Integration
  Points" section (line 615) does not mention Epic-07 or any
  feature-toggle dependency at all — only Epic-01, Epic-05, Epic-02, and
  Epic-04 are listed. Per the epic-file-wins rule this spec does not add an
  Epic-07 dependency, but the omission from Epic-06's own integration list
  is worth flagging.
