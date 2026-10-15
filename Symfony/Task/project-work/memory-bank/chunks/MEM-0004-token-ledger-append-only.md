---
{
  "id": "MEM-0004",
  "title": "Token ledger is append-only and balance is a projection",
  "type": "architecture",
  "status": "active",
  "scope": ["billing", "ledger"],
  "tags": ["immutability", "database-privilege", "lock-ordering", "payment"],
  "created": "2026-08-09",
  "last_verified": "2026-08-09",
  "review_after": "2027-02-09",
  "sources": [
    "specs/architect-architecture.md"
  ],
  "supersedes": [],
  "superseded_by": null
}
---

# Token Ledger is Append-Only and Balance is a Projection

## Durable Context

The token ledger is append-only entry log as truth, with balance as a locked projection. Refunds are compensating entries. Nothing is ever mutated. This shape is enforced by database privilege, not by code convention.

### Entry Kinds and Their Signs

| Kind | Sign | Notes |
|---|---|---|
| `purchase` | positive | Funded by a payment record |
| `gift` | positive | Trainer grants tokens |
| `referral_reward` | positive | References the triggering referral (Epic-06) |
| `refund` | positive | Compensating entry referencing the original spend |
| `spend` | negative | Beneficiary player required (decision A7) |
| `adjustment` | signed | Super Admin correction for out-of-band Stripe activity (BR-05-12). Requires a reason and is audit-logged |

### Core Invariants, Enforced As Rules

- **I1** Balance equals the sum of that (parent account, trainer) pair's entries. Self-policing; verified by console command and test.
- **I2** Balance is never negative. Database check constraint backstops this; negative `adjustment` is rejected loudly.
- **I3** Every `spend` records the beneficiary player. Every `refund` inherits the beneficiary of the spend it references.
- **I4** Refunds referencing one spend never exceed that spend. BR-05-5 permits partial refunds; this must be enforced.
- **I5** Sign matches kind per the table above.
- **I6** At most one entry exists per (payment record, purpose). Outbound idempotency at the ledger level.
- **I7** **No entry is ever updated or deleted.** Enforced by database privilege: the application role holds `INSERT` and `SELECT` only.

### Lock Ordering: The RSVP Deadlock Prevention

A paid RSVP takes two locks — the token balance row and the event capacity row. Concurrent RSVPs can deadlock if the order varies.

**Fixed global order: token balance row first, then the event row.** Every path that takes both obeys it:
- Paid RSVP
- RSVP cancellation refund
- Content purchase that also touches capacity

**Why this order:** The event row is the hot row (many parents contending for the last spots). The balance row is contended only within one family. Acquiring the balance first means the hot row is held for the minimum span — from acquisition to commit — instead of also spanning the balance validation. An **unlocked capacity pre-check** gives fail-fast UX for "event full"; it is explicitly non-authoritative and the locked check inside the transaction is the one that decides (AC-02-67).

**Trainer-initiated cancellation never holds both.** It locks the event, marks it Cancelled and commits — closing the door on new RSVPs — then refunds each player in its own transaction taking only that player's balance lock. A single transaction holding the event lock across N Stripe refund calls is not viable.

This is also why trainer-initiated cancellation refunds are asynchronous: one message per RSVP.

### Subscriptions as Entitlements, Not Entries

`SubscriptionEntitlement` carries its own (parent account, trainer) subject, an activation date chosen by the purchaser, a 30-day window and a state. Spending against it decrements nothing and writes no token entry. RSVPs record which entitlement covered them via `EntitlementCoverage`.

The "1 advance booking per day, unlimited same-day" rule (BR-05-14, AC-02-63): let `D` be the calendar date of the event's start in the trainer's timezone; `T` today's date in that timezone. If `D == T` same-day and unconstrained. If `D > T` advance booking, permitted only when the player holds no other active entitlement-covered RSVP for an event on date `D`. The limit counts **entitlement-covered RSVPs only** — a subscriber may still pay tokens or money for an additional advance event.

### Payment Records and Stripe Mirror

- Fee rate in force is recorded on every payment at creation. AC-05-27 changes the rate for new transactions only; reading live would retroactively rewrite history.
- Payer is nullable, with an always-present contact snapshot from form submission. A camp registrant who never converts has a payment and no account (A3).
- Refunds are new records, never mutations. The charge record's status still moves to mirror Stripe, but "Refunded" in history is derived from refund records (AC-05-23).
- **Refund direction exactly matches spec**: cancelling **at least 24 hours** before start is full refund; **under 24 hours**, none; trainer-initiated cancellation always refunds in full (BR-05-10, AC-05-14/15, BR-02-11/12). A coded inversion refunds exactly the wrong population.

### Idempotency

**Inbound**: Stripe event id is inserted into uniquely-constrained `StripeEventReceipt` first, in its own transaction. The unique violation *is* the duplicate detection.

**Outbound**: Idempotency key is derived deterministically from a payment record persisted **before** the Stripe call, so a timeout retry reuses the key.

### Earnings: Read from Stripe, Never Summed Locally

Epic-07 AC-07-7 states four times that no revenue, payout or transaction data is duplicated. AC-05-25 shows trainers their lifetime earnings. Resolution: **`StripeReportingReader` is the only source of such figures, it persists nothing, and no repository exposes a method that sums payment amounts.** When Stripe is unavailable the earnings panel shows an error — never a locally-summed fallback, which would silently violate AC-07-7.

## Consequences

- Refunds are not reversals: they are forward-dated compensating entries. Treating them as mutations is a defect.
- Balance queries are always projection queries summing entries, never reading a stored balance directly. Balances are locked projections for concurrency anchoring only.
- Lock order is structural, not a discipline. Tests must verify the order is never inverted.
- `TokenLedgerService` is the **only** writer of both entries and balances. Other modules call it; they never write directly.

## Verification

Verify invariant I1 by running the balance reconciliation command before every deployment. Verify I7 by checking that a production entry table has never had an UPDATE or DELETE issued against it.
