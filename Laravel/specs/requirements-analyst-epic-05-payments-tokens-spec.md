# Spec: Epic-05 — Payments & Tokens

Source: `Task/Epics/Epic-05_Payments_Tokens_SPEC.md` (read-only client material)

> Citation note: as in the Epic-01 and Epic-02 specs, every back-reference
> below cites heading **text** plus a line number, never a bare section
> number. Epic-05's own numbered sections (§1–§15) are not themselves
> duplicated the way Epic-01's were, but most of its rule/criteria detail
> lives under **bold sub-labels** (e.g. "Stripe Configuration",
> "Parent-Trainer Token Balance", "24-Hour Refund Policy") rather than
> markdown headings. Those are cited as a second `§` level — `§ "Parent
> Heading" § "Sub-label"` — the same convention the Epic-01 and Epic-02
> specs used for bold sub-labels.

## Problem

Epic-05 is the platform's monetization layer: every paid interaction
elsewhere on the platform — event RSVPs, content purchases, a trainer's own
subscription to the platform, and a player's subscription to a trainer —
resolves through this epic ("Purpose", line 7). It exists so trainers can
collect payment reliably and receive automated payouts, so players/parents
can pay conveniently — by card or by pre-purchased trainer-specific tokens —
without re-entering card details each time, and so the platform itself earns
revenue via a 5% default application fee on transactions plus a $15/month
per-trainer subscription ("Business Value", lines 11-13). Success is judged
against concrete targets: a 90%+ first-attempt payment success rate,
sub-5-second payment processing, under 1% payment dispute rate, 80%+ of
active trainers using tokens, an average of 3+ token purchases per player
per month, and zero PCI compliance issues because Stripe — not the platform
— handles cardholder data ("Success Metrics", lines 16-21).

The epic states its own dependency order plainly: it requires Epic-01
(players, trainers, and parent accounts must already exist), Epic-02 (events
must exist to be purchased), and Epic-04 (content must exist to be
purchased), and it blocks nothing downstream, so it can be built in parallel
with Epic-06 and Epic-07 ("Dependencies — Required Before This Epic", lines
146-148; "...Blocks These Epics", line 151). Its two external dependencies
are the Stripe platform itself (Payments, Connect, Billing, Webhooks) and an
email service for payment/receipt notifications ("...External
Dependencies", lines 154-155). See Open questions for a planning document
that states the opposite dependency direction between this epic and Epic-04.

Four roles interact with payments, and their permissions differ sharply: the
Trainer/Business Owner sets pricing and connects Stripe; the Player/Parent
makes payments and purchases tokens (a parent can pay for any of their
children); the Super Admin configures platform-wide and per-trainer fee
rates and can see all financial data; and the Coach/Contractor has **no
payment interaction or payment permissions at all** — coaches are paid
outside the platform ("User Roles Involved", lines 162-166). Because this
epic moves money — event and content charges, refunds, platform fees, and
multi-tenant token balances that must never leak between a parent's
different trainers — the platform's own planning document separately rates
it "High" complexity: "Payment security, Stripe integration complexity,
multiple payment flows" (`Epic_Areas_Plan.md` § "Epic 05: Payment
Processing & Financial Management" § "Estimated Complexity", line 255).

## User scenarios

1. **Trainer** connects their Stripe account so that they can receive
   payments from their players.
   Path: from Trainer Settings, the trainer clicks "Connect Stripe," is
   redirected to Stripe Connect Express onboarding, completes KYC
   (business/personal info, bank account, tax info — about 5-10 minutes),
   is redirected back, and sees "Stripe Connected ✓"; until connected, the
   trainer can still create free events but cannot create paid events or
   sell content.
   Source: US-05.01 "Trainer Connects Stripe Account" (line 172)

2. **Player or Parent** buys tokens from their trainer so that they can pay
   for sessions quickly without re-entering card details each time.
   Path: from the current trainer context, the player/parent opens
   "Tokens"/"Wallet," sees their balance, clicks "Buy Tokens," selects a
   package or a custom amount, completes Stripe Checkout, and sees their
   balance update immediately with a confirmation email; tokens are stored
   at the parent-trainer level, so any of the parent's children who train
   with that trainer can use them.
   Source: US-05.02 "Player Purchases Tokens" (line 202)

3. **Player or Parent** uses tokens to RSVP for an event so that they can
   register instantly with no payment-processing delay.
   Path: the player/parent views an event's dual price ("2 tokens or
   $20"), clicks "RSVP with Tokens," confirms, and their balance is
   deducted and the RSVP confirmed instantly; if they don't have enough
   tokens, they're offered "Buy More Tokens" or "Pay with Card Instead."
   Source: US-05.03 "Player Pays for Event with Tokens" (line 240)

4. **Player or Parent** pays for an event with a credit card so that they
   can register even without tokens.
   Path: the player/parent clicks "RSVP," chooses "Pay with Card,"
   completes Stripe Checkout (or reuses a saved card), and the RSVP is
   confirmed with an emailed receipt; for a child's paid RSVP, the request
   is held "Pending Parent Approval" until the parent completes the Stripe
   Checkout themselves.
   Source: US-05.04 "Player Pays for Event with USD" (line 270)

5. **Player or Parent** cancels an RSVP and receives a refund so that they
   can recover their payment if plans change.
   Path: canceling ≥24 hours before the event triggers an automatic full
   refund (tokens returned, or a Stripe refund initiated); canceling <24
   hours before forfeits the payment with no refund; if the trainer
   cancels the event instead, every registered player is refunded in full
   regardless of timing.
   Source: US-05.05 "Player Cancels RSVP and Receives Refund" (line 306)

6. **Player or Parent** purchases access to an LPPP content playlist so
   that they can learn and practice between sessions.
   Path: the player browses the content library, sees a locked playlist's
   price ("$50 one-time or 5 tokens"), clicks "Purchase Access," pays by
   card or tokens, and the playlist unlocks permanently; a trainer can also
   "suggest" a playlist to a specific player, who is notified but must
   still purchase it to gain access.
   Source: US-05.06 "Player Purchases Content (LPPP Playlist)" (line 344)

7. **Parent** manages their saved payment methods so that they control
   which cards are used for family payments.
   Path: the parent opens Settings → Payment Methods, clicks "Manage
   Payment Methods," and is redirected to the Stripe Customer Portal to
   view, add, set-default, or remove cards; one Stripe Customer per parent,
   and the same cards, are shared across every trainer and every child.
   Source: US-05.07 "Parent Manages Payment Methods" (line 378)

8. **Player or Parent** views their payment history so that they can track
   spending and verify charges.
   Path: the player/parent opens "Transactions"/"Payment History," which
   shows only transactions for the current trainer context (token
   purchases, event payments, content purchases, refunds), each entry
   showing date, type, description, amount, payment method, status, and a
   Stripe receipt link, filterable by date range, type, and payment method.
   Source: US-05.08 "Player Views Transaction History" (line 403)

9. **Trainer** views their payout history so that they can track their
   earnings.
   Path: the trainer opens "Payments"/"Earnings," sees a simplified
   in-platform summary (current-period earnings, next payout date, last
   payout amount, lifetime earnings) and a link to the full Stripe Express
   Dashboard, which remains the source of truth for detailed transaction
   history, payout/bank details, tax documents, and fee breakdowns.
   Source: US-05.09 "Trainer Views Payout History" (line 439)

10. **Super Admin** sets a custom fee rate for a specific trainer so that
    they can offer special pricing to partners or early adopters.
    Path: Super Admin opens a trainer's record, clicks "Edit Pricing,"
    changes the monthly subscription amount and/or the application fee
    percentage, and saves; the new subscription rate applies from the
    trainer's next billing cycle and the new fee rate applies to new
    transactions immediately; every change is audit-logged (who, when, old
    rate, new rate, reason).
    Source: US-05.10 "Super Admin Configures Per-Trainer Fee" (line 466)

## Acceptance criteria

Every checkbox below is renumbered sequentially as AC-05-1 .. AC-05-38 from
the epic's per-story "Acceptance Criteria" lists (including each story's
other bold-labeled sub-lists that state testable behavior, e.g. "Stripe
Configuration," "Parent-Trainer Token Balance"), the epic-level "Acceptance
Criteria (Epic Level)" section, and two further groups: five checkboxed
items from §3 "In Scope (MVP)" that name a specific sub-feature with its own
acceptance-relevant detail found nowhere else in the epic (Player
Subscriptions, Dual Pricing, Free events, Flexible Token Pricing, Trainer
Can Gift Tokens), and the behavioral content of §14 "Technical Notes"
(Stripe webhook events, Security Requirements, Error Handling), which the
epic itself frames as requirements rather than design. Where an epic-level
or scope-note criterion restates a story criterion, the story version is
kept and both origins are cited.

**US-05.01 — Trainer Connects Stripe Account** (line 172)
- [ ] **AC-05-1** Trainer connects Stripe from Trainer Settings ("Connect Stripe"), is redirected to Stripe Connect Express onboarding, completes KYC (business/personal info, bank account for payouts, tax information — W-9 or W-8 — about 5-10 minutes), is redirected back, sees status "Stripe Connected ✓," and can now create paid events and accept payments — `Epic-05_Payments_Tokens_SPEC.md` § "US-05.01: Trainer Connects Stripe Account" (lines 179-187)
- [ ] **AC-05-2** Stripe Connect configuration: Express Account type; application fee defaults to 5%, configurable per trainer by Super Admin; payout schedule is monthly on the 1st of the month (configurable to weekly); trainer subscription is $15/month via Stripe Billing — § "US-05.01..." § "Stripe Configuration" (lines 190-193; also epic-level AC § "Acceptance Criteria (Epic Level)" § "Stripe Integration", lines 868-869)
- [ ] **AC-05-3** If a trainer has not connected Stripe, they cannot create paid events (blocked with the message "Connect Stripe first") and cannot sell content, but can still create free events — § "US-05.01..." § "If Trainer Not Connected" (lines 196-198)

**US-05.02 — Player Purchases Tokens** (line 202)
- [ ] **AC-05-4** Player/parent buys tokens: from the current trainer context, navigate to Tokens/Wallet, see the current balance ("You have N tokens with [Trainer]"), click Buy Tokens, select a package (10 tokens/$90, 25 tokens/$225, or 50 tokens/$450 — each "save 10%" — or a custom amount; the trainer sets the per-token price, e.g. 1 token = $10), click Purchase, complete Stripe Checkout, and the balance updates immediately with a confirmation email and receipt — § "US-05.02: Player Purchases Tokens" (lines 209-224; also § "In Scope (MVP)" § "Token System", line 71; also epic-level AC § "Parent/Child Payments", line 862)
- [ ] **AC-05-5** Tokens are stored and displayed at the parent-trainer relationship level, not per child: a parent's token balance with a given trainer can be used for ANY of that parent's children who train with that trainer, not a separate balance per child — § "US-05.02..." § "Parent-Trainer Token Balance (Key Architecture Decision)" (lines 227-230; also § "Business Rules & Logic" § "Token System Rules" § "Token Balance Storage", lines 556-560; also epic-level AC § "Payment Processing", line 863, and § "Parent/Child Payments", line 874)
- [ ] **AC-05-6** A child with their own login who views tokens sees the parent's balance, and can initiate a token purchase; this triggers parent approval (see Epic-01 US-01.05), and only after the parent approves does the purchase complete and the tokens get added to the parent-trainer balance — § "US-05.02..." § "Token Purchase from Child View" (lines 233-236)

**US-05.03 — Player Pays for Event with Tokens** (line 240)
- [ ] **AC-05-7** Player/parent RSVPs with tokens: view the event's price (tokens or USD), see the current token balance, click "RSVP with Tokens," confirm ("Use N tokens for [Event]?"), the balance is deducted immediately, the RSVP is confirmed instantly with no payment-processing delay, and an email confirmation is sent — § "US-05.03: Player Pays for Event with Tokens" (lines 247-254; also epic-level AC § "Payment Processing", line 860)
- [ ] **AC-05-8** If the player has insufficient tokens, the system shows the shortfall ("You have 1 token, need 2 tokens") and offers "Buy More Tokens" (redirect to token purchase) or "Pay with Card Instead" (USD via Stripe Checkout) — § "US-05.03..." § "If Insufficient Tokens" (lines 257-260)
- [ ] **AC-05-9** A parent RSVPing a child with tokens deducts from the parent's token balance with that trainer (not a child-specific balance), and the child is registered for the event — § "US-05.03..." § "Parent RSVPing Child" (lines 263-266)

**US-05.04 — Player Pays for Event with USD** (line 270)
- [ ] **AC-05-10** Player/parent RSVPs and pays with USD: view the event's price, click "RSVP," select "Pay with Card," get redirected to Stripe Checkout, enter card details or use a saved payment method, complete payment, get redirected back, the RSVP is confirmed, and an email confirmation with receipt is sent — § "US-05.04: Player Pays for Event with USD" (lines 277-284, 288-290; also epic-level AC § "Payment Processing", line 861)
- [ ] **AC-05-11** For a $20 USD event charge, Stripe processes the payment with a platform fee of $1 (5%) and an approximate Stripe fee of $0.88 (2.9% + $0.30), leaving the trainer a net of approximately $18.12 — § "US-05.04..." § "If 'Pay with Card' selected" (lines 285-287)
- [ ] **AC-05-12** After a first payment, Stripe saves the payment method to the customer; subsequent purchases offer "Use card ending in [last 4]"; payment methods can be added/removed via the Stripe Customer Portal — § "US-05.04..." § "Payment Method Storage" (lines 293-295)
- [ ] **AC-05-13** A child's RSVP that requires payment is held "Pending Parent Approval"; the parent is notified, and on approval a Stripe Checkout opens for the parent; once the parent completes payment, the child is registered — § "US-05.04..." § "Child Payment with Parent Approval (see Epic-01 US-01.05)" (lines 298-302; also epic-level AC § "Parent/Child Payments", lines 875-876)

**US-05.05 — Player Cancels RSVP and Receives Refund** (line 306)
- [ ] **AC-05-14** Canceling an RSVP ≥24 hours before the event triggers an automatic full refund — tokens returned to balance, or a Stripe refund initiated for USD — with a "Full refund processed" confirmation and a refund-confirmation email — § "US-05.05: Player Cancels RSVP and Receives Refund" § "24-Hour Refund Policy" § "Scenario A: Cancel 24+ Hours Before Event" (lines 315-321; also epic-level AC § "Payment Processing", line 864)
- [ ] **AC-05-15** Canceling an RSVP <24 hours before the event shows a warning that no refund is available within the 24-hour window and requires confirmation to proceed; on confirmation the RSVP is canceled with tokens/payment forfeited and no refund issued — § "US-05.05..." § "Scenario B: Cancel <24 Hours Before Event" (lines 324-328)
- [ ] **AC-05-16** When a trainer cancels an event, all registered players are automatically refunded in full — tokens returned, USD refunded via Stripe — regardless of how close to the event time the cancellation occurs — § "US-05.05..." § "Scenario C: Trainer Cancels Event" (lines 331-335)
- [ ] **AC-05-17** Token refunds are instant; USD refunds take 5-10 business days (Stripe standard); all refunds are tracked in the transaction history log — § "US-05.05..." § "Refund Processing" (lines 338-340; also epic-level AC § "Transaction History", line 882)

**US-05.06 — Player Purchases Content (LPPP Playlist)** (line 344)
- [ ] **AC-05-18** Player purchases LPPP content: browse the content library, see a locked playlist with its price ("$50 one-time or 5 tokens"), click "Purchase Access," choose Card (Stripe Checkout) or Tokens (instant deduction), and on payment the playlist unlocks and stays accessible forever (one-time purchase) — § "US-05.06: Player Purchases Content (LPPP Playlist)" § "Acceptance Criteria (Paywall Model - D-SCOPE-011)" (lines 351-362; also epic-level AC § "Content Payments (Epic-04 integration)", lines 885-887). Cross-epic dependency: the content and playlist itself are Epic-04's; this criterion covers only the payment/unlock mechanics.
- [ ] **AC-05-19** A trainer can suggest a playlist to a specific player; the player is notified ("[Trainer] suggests: [Playlist]") and the suggestion is highlighted in their content library, but the player must still purchase it to gain access — § "US-05.06..." § "Trainer Content Suggestions" (lines 371-374). Cross-epic dependency: the suggestion mechanism itself is Epic-04's.

**US-05.07 — Parent Manages Payment Methods** (line 378)
- [ ] **AC-05-20** Parent manages payment methods via Settings → Payment Methods → "Manage Payment Methods," which redirects to the Stripe Customer Portal to view, add, set-default, or remove cards; changes sync automatically, and the default card is used for all future purchases — § "US-05.07: Parent Manages Payment Methods" (lines 385-393; also epic-level AC § "Parent/Child Payments", line 873)
- [ ] **AC-05-21** Payment method management is handled entirely via the Stripe Customer Portal with no custom UI; each parent/player account has exactly one Stripe Customer ID; payment methods are stored at Stripe, not on the platform (PCI compliance); the same cards are shared across all of a parent's trainers — § "US-05.07..." § "Implementation Note" (lines 396-399)

**US-05.08 — Player Views Transaction History** (line 403)
- [ ] **AC-05-22** Transaction history is scoped to the current trainer context only: a parent in a given child-trainer context sees only token purchases, event payments, content purchases, and refunds tied to that trainer — § "US-05.08: Player Views Transaction History" § "Context-Based Display" (lines 410-416; also § "Business Rules & Logic" § "Multi-Tenant Payment Rules" § "Transaction Isolation", lines 647-649; also epic-level AC § "Transaction History", line 879)
- [ ] **AC-05-23** Each transaction list entry shows date/time, type (Token Purchase, Event RSVP, Content Purchase, Refund), description, amount, payment method, status (Completed, Pending, Refunded, Failed), and a Stripe-generated receipt link; the list is filterable by date range, transaction type, and payment method — § "US-05.08..." § "Transaction List Shows" (lines 419-426); § "...Filters" (lines 428-430; also epic-level AC § "Transaction History", lines 880-882)
- [ ] **AC-05-24** An optional parent "Family Overview" dashboard view shows the 10 most recent transactions across all children and trainers, clickable to filter into a specific child-trainer context — § "US-05.08..." § "Parent 'Family Overview' (Optional)" (lines 433-435)

**US-05.09 — Trainer Views Payout History** (line 439)
- [ ] **AC-05-25** Trainer views payout summary: navigate to Payments/Earnings, see a message directing to the full Stripe Dashboard with a link to the Stripe Express Dashboard, plus a simplified in-platform summary of current-period earnings, next payout date, last payout amount, and total lifetime earnings — § "US-05.09: Trainer Views Payout History" (lines 446-453)
- [ ] **AC-05-26** The Stripe Express Dashboard is the source of truth for detailed transaction history, payout history/bank details, tax documents (1099s), refund management, and fee breakdown; the platform links to Stripe rather than duplicating this financial reporting — § "US-05.09..." § "Stripe Dashboard (Source of Truth - D-ARCH-001)" (lines 456-462)

**US-05.10 — Super Admin Configures Per-Trainer Fee** (line 466)
- [ ] **AC-05-27** Super Admin can view and edit a specific trainer's pricing from the Trainers section ("Edit Pricing"): modify the monthly subscription amount (default $15/month) and/or the application fee rate (default 5%) per trainer; the new subscription rate applies from the trainer's next billing cycle, and new transactions use the new fee rate immediately — § "US-05.10: Super Admin Configures Per-Trainer Fee" (lines 473-484)
- [ ] **AC-05-28** All per-trainer pricing changes are audit-logged, capturing who changed it, when, the old rate, the new rate, and an optional reason — § "US-05.10..." § "Audit Logging" (lines 487-488)

**Scope-note criteria (§3 "In Scope (MVP)", not tied to a single dedicated user story)**
- [ ] **AC-05-29** Player Subscriptions - Unlimited Access are in scope: implemented as special tokens granting unlimited access (not a separate Stripe subscription); the trainer sets the subscription price in the platform and can activate it from a selected future date; the subscription grants 30-day unlimited access to the trainer's qualifying events; RSVP is restricted to 1 event booked in advance per day, with unlimited same-day bookings; payment is via Stripe — § "In Scope (MVP)" § "Payment Methods" § "Player Subscriptions - Unlimited Access" (lines 56-62); confirmed again — as an implementation-approach note, filed under the wrong heading; see Open questions — at § "Out of Scope (Post-MVP)" § "✅ CONFIRMED (January 2026)" (lines 119-123); also § "Business Rules & Logic" § "Subscription Rules" § "Player Subscriptions" (lines 628-634)
- [ ] **AC-05-30** Dual Pricing per Event: events can carry both a USD price and a token price at the same time; the player chooses which to pay with; trainers can toggle either pricing option on or off independently per event; the default configuration is token pricing enabled (1 token) and USD pricing disabled ($0) — § "In Scope (MVP)" § "Payment Methods" § "Dual Pricing per Event" (lines 63-67). Cross-epic dependency: the event-side toggle and display is Epic-02's (its AC-02-58); this criterion covers only the payment-method defaults.
- [ ] **AC-05-31** Free events (priced at $0) are supported as a payment-method category requiring no payment step — § "In Scope (MVP)" § "Payment Methods" (line 68). Cross-epic dependency: the RSVP-side "no payment step" behavior is Epic-02's (its BR-02-8).
- [ ] **AC-05-32** Flexible Token Pricing: events can cost multiple tokens, not always a 1:1 token-to-session ratio (example: a premium 3-hour event = 2 tokens); the per-event token cost is trainer-configurable, and each trainer sets their own token pricing independently — § "In Scope (MVP)" § "Token System" § "Flexible Token Pricing" (lines 72-76)
- [ ] **AC-05-33** Trainer Can Gift Tokens: a trainer can manually add tokens to a player's balance without payment (use cases: refunds for edge cases, rewards for top performers, promotions); this permission belongs to trainers only, not coaches; gifting requires audit logging (Epic-07); the UI flow is trainer selects player, enters a token amount, and adds a note — § "In Scope (MVP)" § "Token System" § "Trainer Can Gift Tokens" (lines 81-86)

**Epic-level (from "Technical Notes," line 824, and "Acceptance Criteria (Epic Level)," line 857) — not already covered above**
- [ ] **AC-05-34** On receiving Stripe webhooks, the system must: confirm the RSVP and unlock content on `payment_intent.succeeded`; notify the player and retry payment on `payment_intent.payment_failed`; update transaction history and restore tokens on `charge.refunded`; grant subscription access on `customer.subscription.created`; revoke subscription access on `customer.subscription.deleted`; and update the trainer's Stripe connection status on `account.updated` — § "Technical Notes" § "Stripe Webhook Events to Handle" (lines 826-832)
- [ ] **AC-05-35** Webhooks are processed reliably at a 99%+ success rate (epic-level AC § "Stripe Integration", line 870); their Stripe signature is verified to prevent spoofing (§ "Technical Notes" § "Security Requirements", line 835); failed webhooks are retried with exponential backoff (§ "...Error Handling", line 844), up to 3 attempts over 24 hours (§ "Performance & Scale Targets" § "Reliability", line 761)
- [ ] **AC-05-36** Security requirements: idempotency keys are used for payment operations to prevent duplicate charges; card details are never stored on the platform (PCI compliance); Stripe API keys are encrypted in environment variables; payment attempts are rate-limited to prevent brute force — § "Technical Notes" § "Security Requirements" (lines 836-839)
- [ ] **AC-05-37** Error handling: Stripe API errors are logged and the admin is notified; payment failures show clear, user-friendly messages; insufficient token balance prompts the player to purchase more; a network failure during Stripe Checkout resumes on return — § "Technical Notes" § "Error Handling" (lines 842-846)
- [ ] **AC-05-38** Performance targets: token balance check <100ms, token purchase redirect to Stripe <500ms, payment confirmation webhook processing <2 seconds, refund processing <5 seconds (tokens) / <10 seconds (USD initiation); 100 concurrent payments supported; 1,000 transactions/day (Year 1 estimate); 50 webhooks/minute; 99.9% payment processing uptime (Stripe SLA); <0.1% payment data loss — § "Performance & Scale Targets" (lines 747-761; also epic-level AC § "Performance", lines 890-893)

**Final count: 38 acceptance criteria (AC-05-1 .. AC-05-38)**, covering all
10 user stories (US-05.01–US-05.10), five scope-note criteria from §3 "In
Scope (MVP)" not tied to any single story, and five criteria drawn from
epic-level "Technical Notes" and "Acceptance Criteria (Epic Level)," with
epic-level and scope-bullet restatements folded into their matching story
criterion rather than duplicated.

## Business rules

Restated from "Business Rules & Logic" (line 546). The epic groups these
under five named categories — Token System, Payment Processing, Refund
Policy, Subscription, and Multi-Tenant Payment — all five are represented
below; none are dropped.

**Token System Rules** (line 548)
- [ ] **BR-05-1** The trainer sets the per-token price (e.g. 1 token = $10); token packages can offer bundle discounts (e.g. 10 tokens for $90 instead of $100); tokens are trainer-specific and cannot be used with a different trainer — `Epic-05_Payments_Tokens_SPEC.md` § "Business Rules & Logic — Token System Rules" § "Token Pricing" (lines 550-553)
- [ ] **BR-05-2** Tokens are stored at the parent-trainer relationship level, not per child: a parent's balance with a given trainer is usable by any of that parent's children who train with that trainer — § "...Token System Rules" § "Token Balance Storage" (lines 555-560)
- [ ] **BR-05-3** When an event allows both tokens and USD, the player explicitly chooses the payment method; there is no automatic token usage — § "...Token System Rules" § "Token Usage Priority" (lines 562-564)
- [ ] **BR-05-4** Tokens do not expire for MVP; expiration may be added in Phase 2 if needed — § "...Token System Rules" § "Token Expiration" (lines 566-567)
- [ ] **BR-05-5** Refunded tokens return to the parent-trainer balance immediately; partial token refunds are possible (e.g. a 2-token event refunds 2 tokens) — § "...Token System Rules" § "Token Refunds" (lines 569-571)

**Payment Processing Rules** (line 573)
- [ ] **BR-05-6** All USD payments go through Stripe Checkout (a Stripe-hosted page); Stripe handles card validation, 3D Secure, and fraud detection; the user is redirected back to the platform after payment, and a webhook confirms payment success asynchronously — § "...Payment Processing Rules" § "Stripe Checkout Flow" (lines 575-579)
- [ ] **BR-05-7** The 5% application fee is included in the listed price and absorbed by the trainer, not added on top for the player: for a $100 event, the player pays $100, and the trainer nets approximately $91.80 after the ~$3.20 Stripe fee (2.9% + $0.30) and the $5 (5%) platform fee; the trainer sees the full breakdown in their Stripe Dashboard — § "...Payment Processing Rules" § "Application Fee Deduction" (lines 581-588). See Open questions for ambiguity in how this fee is described elsewhere in the epic.
- [ ] **BR-05-8** If a payment fails, the RSVP is not confirmed and the spot remains available; the player is notified and offered a "Try Again" retry; after 3 failed attempts the system suggests a different payment method — § "...Payment Processing Rules" § "Failed Payments" (lines 590-594)
- [ ] **BR-05-9** Payment methods that take time to clear (e.g. bank transfers) hold the RSVP as "Payment Pending"; a webhook updates the status once payment clears; if payment has not succeeded after 7 days, the RSVP is auto-canceled — § "...Payment Processing Rules" § "Pending Payments" (lines 596-600)

**Refund Policy Rules** (line 602)
- [ ] **BR-05-10** 24-hour refund rule: canceling ≥24 hours before the event yields a full refund; canceling <24 hours before yields no refund; a trainer-initiated event cancellation always yields a full refund regardless of timing — § "...Refund Policy Rules" § "24-Hour Rule" (lines 604-607)
- [ ] **BR-05-11** Token refunds are instant; USD refunds are initiated immediately but take 5-10 business days to complete, with Stripe handling the actual refund processing — § "...Refund Policy Rules" § "Refund Processing" (lines 609-612)
- [ ] **BR-05-12** Trainers may manually issue refunds via the Stripe Dashboard for special cases (e.g. injury, family emergency) at their own discretion; the platform does not enforce rules on manual refunds — § "...Refund Policy Rules" § "Trainer Manual Override" (lines 614-617)

**Subscription Rules** (line 619)
- [ ] **BR-05-13** The trainer's own platform subscription is $15/month by default, billed on the 1st of each month via Stripe Billing; on payment failure, 3 retry attempts occur over 10 days; if still failing, the trainer's account is suspended (cannot create paid events) until payment succeeds, at which point the account is reactivated — § "...Subscription Rules" § "Trainer Subscription (to Platform Owner)" (lines 621-626)
- [ ] **BR-05-14** Player Subscriptions grant 30-day unlimited access to a trainer's qualifying events, implemented as special tokens activated from a selected date (not a separate Stripe subscription); the trainer sets the subscription price; payment is processed via Stripe; the subscription token is granted only after successful payment, with no grace period; RSVP is restricted to 1 event booked in advance per day, with unlimited same-day bookings — § "...Subscription Rules" § "Player Subscriptions" (lines 628-634). Cross-epic reference: the same 1-per-day advance-booking restriction is independently stated and enforced on the Epic-02 side (`requirements-analyst-epic-02-event-management-spec.md`, AC-02-63).

**Multi-Tenant Payment Rules** (line 636)
- [ ] **BR-05-15** A parent with multiple trainers holds a separate token balance per trainer (example: 10 tokens with Trainer A, 5 tokens with Trainer B); tokens cannot be mixed or transferred between trainers — § "...Multi-Tenant Payment Rules" § "Parent with Multiple Trainers" (lines 638-641)
- [ ] **BR-05-16** Payment methods (cards) are shared across all of a parent's trainers at the Stripe Customer level; the same cards are used regardless of which trainer context the parent is purchasing in — § "...Multi-Tenant Payment Rules" § "Payment Methods" (lines 643-645)
- [ ] **BR-05-17** Transaction history is strictly isolated per trainer context: a trainer cannot see a player's transactions with a different trainer — § "...Multi-Tenant Payment Rules" § "Transaction Isolation" (lines 647-649)

### Integration points (cross-epic dependency notes)

Restated from the epic's own "Integration Points" section (line 792), added
as an explicit sub-list per this run's brief — Epic-05 is unusually precise
about how deeply payments reach into every other epic, and folding this
only into individual AC/BR cross-references (as the Epic-02 and Epic-04
specs did) would understate that.

**With Epic-02 (Event Management)** (line 794):
- Event RSVP triggers payment flow
- Event cancellation triggers refund flow
- Event capacity checks payment status before confirmation
- Trainer-canceled events trigger automatic refunds

**With Epic-04 (LPPP Content)** (line 800):
- Content purchase triggers payment flow
- Content access gates verify payment/subscription status
- Trainer content suggestions link to purchase flow

**With Epic-01 (User Management)** (line 805):
- Child actions trigger parent approval workflow
- Parent manages payment methods via Stripe (for the whole family)
- Stripe Customer ID stored at parent account level
- Multi-trainer context affects token balance display

**With Epic-06 (Marketing Tools)** (line 811):
- Referral rewards trigger token grants
- Coupon codes apply discounts to payments
- First purchase triggers referral reward

**With Stripe** (line 816):
- Stripe Connect for trainer accounts
- Stripe Checkout for payment processing
- Stripe Billing for trainer subscriptions (trainers pay platform owner $15/month)
- Stripe Webhooks for event updates (payment success, refund, trainer subscription changes)

## Data requirements

Restated from "Data Requirements" (line 492) — entities, the fields the
epic names, and the relationships it implies from those fields. No schema,
keys, or types are proposed here.

- **Token Balance**: parent/player account reference, trainer reference, current balance (integer), last updated timestamp. Relationship: one balance per parent/player-account-and-Trainer pair. (line 496)
- **Token Transaction**: transaction unique identifier, parent/player account, trainer, type (Purchase, Usage, Refund), amount (tokens), reason (e.g. "Event RSVP: Basketball Skills", "Token Purchase", "Refund"), timestamp, related event or content if applicable. Relationship: references a parent/player account, a Trainer, and optionally an Event (Epic-02) or content item (Epic-04). (line 502)
- **USD Transaction**: transaction unique identifier, parent/player account, trainer, Stripe Payment Intent ID (reference to Stripe), amount (USD), type (Event Payment, Content Purchase, Subscription), status (Pending, Completed, Failed, Refunded), related event or content if applicable, timestamp. Relationship: references a parent/player account, a Trainer, and optionally an Event (Epic-02) or content item (Epic-04); references Stripe via the Payment Intent ID. (line 512)
- **Stripe Customer**: parent/player account reference, Stripe Customer ID, default payment method ID (optional). Relationship: one per parent/player account, shared across all of that account's trainers. (line 523)
- **Trainer Stripe Connect**: trainer account reference, Stripe Connect Account ID, onboarding status (Pending, Complete, Incomplete), subscription status (Active, Past Due, Canceled), custom fee rate (if different from the default 5%), connected timestamp. Relationship: one per Trainer account. (line 528)
- **Refund**: refund unique identifier, original transaction reference, refund amount (USD or tokens), refund reason (Player canceled, Trainer canceled, Manual), refund timestamp, Stripe Refund ID (if a USD refund). Relationship: references the original Token Transaction or USD Transaction being refunded. (line 536)

## Edge cases

| Case | Expected |
|---|---|
| Player has insufficient tokens for an event | Shown the shortfall ("You have 1 token, need 2 tokens"); offered "Buy More Tokens" or "Pay with Card Instead" — US-05.03 § "If Insufficient Tokens" (lines 257-260) |
| Trainer has not connected Stripe | Cannot create paid events ("Connect Stripe first") or sell content; can still create free events — US-05.01 § "If Trainer Not Connected" (lines 196-198) |
| USD payment fails during RSVP or Checkout | RSVP not confirmed, spot remains available, player notified with a "Try Again" option; after 3 failed attempts the system suggests a different payment method — Business Rules § "Payment Processing Rules" § "Failed Payments" (lines 590-594) |
| A pending payment method (e.g. bank transfer) does not clear | RSVP held "Payment Pending"; a webhook updates status when it clears; if not cleared after 7 days, the RSVP is auto-canceled — Business Rules § "...Pending Payments" (lines 596-600) |
| Player cancels RSVP <24 hours before the event | No refund; confirmation warns "No refund available (within 24-hour window). Continue?"; tokens/payment forfeited on confirmation — US-05.05 § "Scenario B: Cancel <24 Hours Before Event" (lines 323-328) |
| Player cancels RSVP ≥24 hours before the event | Automatic full refund (tokens returned, or Stripe refund initiated for USD) — US-05.05 § "Scenario A: Cancel 24+ Hours Before Event" (lines 314-321) |
| Trainer cancels the event | All registered players refunded in full, regardless of timing — US-05.05 § "Scenario C: Trainer Cancels Event" (lines 330-335) |
| Network failure during Stripe Checkout | Resumes on return — Technical Notes § "Error Handling" (line 846) |
| Webhook delivery fails | Retried with exponential backoff, up to 3 attempts over 24 hours — Technical Notes § "Error Handling" (line 844); Performance & Scale Targets § "Reliability" (line 761) |
| Trainer's own platform subscription payment fails | 3 retry attempts over 10 days; if still failing, the trainer account is suspended (cannot create paid events) until payment succeeds, then reactivated — Business Rules § "Subscription Rules" § "Trainer Subscription (to Platform Owner)" (lines 624-626) |
| Player Subscription payment fails | No access granted; subscription token is granted only after successful payment, with no grace period — Business Rules § "Subscription Rules" § "Player Subscriptions" (line 633); Questions/Open Issues Q-05.07 resolution (line 787) |
| Child initiates a token purchase or a paid RSVP | Held for parent approval; processed only after the parent approves — US-05.02 § "Token Purchase from Child View" (lines 234-235); US-05.04 § "Child Payment with Parent Approval" (lines 298-302) |
| Stripe API error occurs | Logged, admin notified — Technical Notes § "Error Handling" (line 842) |

## Out of MVP scope

Verbatim from "Out of Scope (Post-MVP)" (line 117):

**✅ CONFIRMED (January 2026)** (line 119) — this entry is a scope
*confirmation*, not an exclusion; see Open questions for why it is filed
under this heading:
- Player Subscriptions: Special tokens activated from selected date after purchase
  - Implementation confirmed: Token-based (not separate Stripe subscription)
  - Trainer can activate subscription from any future date
  - Simple implementation using existing token system

**Phase 2 Deferrals** (line 125):
- Multi-currency (EUR, GBP, etc.)
- Payment plans / Buy Now Pay Later
- Platform-wide wallet (tokens are trainer-specific only)
- Gift cards / promo codes (coupons are in, but not gift cards)
- Trainer-to-coach payment distribution (trainers pay coaches outside platform)
  - Platform provides analytics (sessions covered, player counts, attendance) for trainer's records
  - Trainers handle coach compensation independently
  - See Q-05.08 for future consideration
- Invoice generation (trainers use Stripe Dashboard)
- Recurring token auto-purchase
- Token expiration dates
- Fractional tokens (e.g., 0.5 tokens)
- Token transfer between trainers
- Refund requests (manual, not automated approval)

**Also out of scope per "Scope Summary — Non-Goals (Post-MVP)" (line 36),
not repeated in the "Out of Scope (Post-MVP)" list above**:
- Cryptocurrency payments
- Tax calculation (trainers handle via Stripe)
- Token balance alerts (low balance notifications) — resolved as "not needed for MVP" per Q-05.06
- Partial refunds (price change after RSVP) — resolved as "too complex for MVP; trainers can use token gifting to handle edge cases" per Q-05.05
- Subscription grace period (payment failure handling) — resolved as "no grace period" per Q-05.07 and Business Rules § "Subscription Rules" § "Player Subscriptions"

## Open questions

### From the epic's "Questions / Open Issues" section (line 770, verbatim)

| ID | Question | Priority | Status | Owner |
|:---|:---|:---:|:---|:---|
| Q-05.01 | Refund policy finalized: 24-hour window | P0 | ✅ RESOLVED | Client |
| Q-05.02 | **Player subscriptions**: Should trainers be able to offer monthly subscription for unlimited content/event access? Or one-time purchases only for MVP? | P1 | ✅ RESOLVED | Client |
| Q-05.03 | Token package sizes and discounts: What's the pricing strategy? | P2 | Open | Client |
| Q-05.04 | Failed payment retry: How many attempts? Auto-email reminders? | P2 | ✅ RESOLVED | Team |
| Q-05.05 | Partial refunds: If event price changes after RSVP, refund difference? | P2 | ✅ RESOLVED | Client |
| Q-05.06 | Token balance alerts: Notify player when balance low? | P2 | ✅ RESOLVED | Client |
| Q-05.07 | Subscription grace period: How long until access cut off after failed payment? | P2 | ✅ RESOLVED | Client |
| Q-05.08 | **Trainer-to-coach payments**: Should platform facilitate coach payment distribution in future? Or always handled outside platform? (Assume OUT for MVP, provide analytics only) | P2 | ✅ RESOLVED | Client |

**Resolutions** (line 783, verbatim):
- **Q-05.04**: Stripe handles payment retry logic automatically
- **Q-05.05**: Too complex for MVP - trainers can use token gifting to handle edge cases
- **Q-05.06**: NO - token balance alerts not needed for MVP
- **Q-05.07**: No grace period - subscription token granted only after successful payment (immediate access on payment, no access on failed payment)
- **Q-05.08**: OUT of MVP - trainer-to-coach payments handled externally, platform provides analytics only

*Note*: the table marks Q-05.01 and Q-05.02 ✅ RESOLVED too, but neither has
an entry in this Resolutions list — see analyst-raised below.

### Additional client question embedded in US-05.06 (line 364, verbatim)

This question is **not** part of the table above — it is embedded directly
in the US-05.06 story and reuses the same ID, "Q-05.02," for a different
question than the table's Q-05.02 (see analyst-raised note below):

> **Alternative Pricing Models** (Q-05.02 - Not Confirmed):
> - Subscription model (monthly access to all content)
> - Bundle pricing (multiple playlists at discount)
> - Event + content bundles
> - **Status**: Needs Dale approval before implementation

### Analyst-raised

- **(analyst-raised)** Dependency cycle with Epic-04. Epic-05's own §5
  "Dependencies — Required Before This Epic" (`Epic-05_Payments_Tokens_SPEC.md`,
  line 148) lists Epic-04 as required before Epic-05 ("Content must exist to
  purchase"). But `Epic_Areas_Plan.md`'s Epic-04 section lists "Epic-05:
  Payment Processing (content purchases)" under its own "### Dependencies"
  heading (`Epic_Areas_Plan.md`, line 318) — implying Epic-04 needs Epic-05,
  not the reverse. The Epic-04 spec produced in this same run documents the
  identical cycle from its side (`requirements-analyst-epic-04-lp-content-spec.md`,
  Open questions, analyst-raised). This spec does not resolve the cycle;
  that is an architect-stage decision.
- **(analyst-raised)** Player Subscriptions are filed under the wrong
  heading. "In Scope (MVP) — Payment Methods" lists "Player Subscriptions -
  Unlimited Access" as an in-scope, checkboxed item with its full terms
  (lines 56-62). The identical feature then reappears inside "4. Out of
  Scope (Post-MVP)" under "✅ CONFIRMED (January 2026)" (lines 119-123) — a
  heading that, read literally, says the feature is excluded from MVP.
  Reading the entry's actual content, it is not an exclusion at all: it
  confirms the feature's implementation approach (token-based, not a
  separate Stripe subscription) and reaffirms it is in scope. This spec
  treats Player Subscriptions as in scope per §3 (AC-05-29, BR-05-14) and
  flags the §4 placement as a filing error, not a scope decision.
- **(analyst-raised)** Ambiguity about who bears the 5% platform fee. The
  epic's most explicit statement is unambiguous: "Application Fee
  Deduction: Fee included in price (trainer absorbs)" (§9 "Business Rules &
  Logic — Payment Processing Rules", line 582), backed by two worked
  examples where the player is charged exactly the listed price and the
  trainer's payout is reduced by both the Stripe fee and the platform fee
  (US-05.04, lines 284-288; §9, lines 583-587) — this matches
  `Epic_Areas_Plan.md`'s "Application Fee model (5% platform cut included
  in price)" (line 224). However, every *other* mention of the
  "application fee" in the epic states only the percentage and that the
  platform receives it, without repeating "included in price" or "trainer
  absorbs": Business Value (line 13), §3 "In Scope (MVP) — Stripe
  Integration" (line 94), US-05.01 "Stripe Configuration" (line 191),
  US-05.10 (lines 477, 481), and the epic-level AC (line 868). A reader who
  sees only one of those five mentions could reasonably assume the fee is
  added on top of the player's charge rather than deducted from the
  trainer's payout. This spec follows the epic's explicit "trainer absorbs"
  statement (BR-05-7) but flags the surrounding ambiguity rather than
  treating it as fully settled, because it changes what the player is
  actually charged.
- **(analyst-raised)** ID collision on "Q-05.02." The table's Q-05.02 asks
  the broad question "Should trainers be able to offer monthly subscription
  for unlimited content/event access? Or one-time purchases only for MVP?"
  and is marked ✅ RESOLVED (line 775) — resolved, per §3/§4, as "yes, via
  token-based Player Subscriptions." US-05.06 embeds a differently-scoped
  question, also labeled "Q-05.02," specifically about alternative pricing
  *models for LPPP content* (a content-only monthly subscription, bundle
  pricing, event+content bundles) and explicitly marks it "Not Confirmed...
  Needs Dale approval before implementation" (lines 364, 368) — the
  opposite status. These are two distinct questions sharing one ID with two
  different resolution states; the content-pricing question embedded in
  US-05.06 remains genuinely open regardless of the table row's RESOLVED
  status. This mirrors the Q-01.05 ID collision the Epic-01 spec flagged.
- **(analyst-raised)** Two of the eight table rows (Q-05.01, Q-05.02) are
  marked ✅ RESOLVED but have no corresponding entry in the "Resolutions"
  list that follows the table (line 783), which explains only Q-05.04
  through Q-05.08. Q-05.01's resolution is inferable from §9's stated
  24-hour rule; Q-05.02's is inferable from §4's "CONFIRMED (January 2026)"
  note (itself flagged above as misfiled). Neither resolution is spelled
  out where the table's own structure says to look for it.
- **(analyst-raised)** Of the eight questions in the table, only Q-05.03
  (token package sizes and discounts / pricing strategy) remains genuinely
  Open. Every token-package price shown elsewhere in the epic (10
  tokens/$90, 25 tokens/$225, 50 tokens/$450, all "save 10%" — US-05.02,
  lines 214-216) is therefore only an illustrative example, not a confirmed
  pricing strategy, pending this question's resolution.
- **(analyst-raised)** "Scope Summary — Non-Goals (Post-MVP)" (line 36) and
  "Out of Scope (Post-MVP)" (line 117) are two separate exclusion lists in
  the source that overlap but do not match: five items appear only in the
  former (cryptocurrency payments, tax calculation, token balance alerts,
  partial refunds, subscription grace period) and are not repeated in the
  latter, more detailed list. This spec's Out of MVP scope section merges
  both without dropping either; a future edit to one list should check the
  other.
