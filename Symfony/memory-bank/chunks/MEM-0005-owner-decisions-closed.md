---
{
  "id": "MEM-0005",
  "title": "Owner decisions A1–A12 and platform-fee call are closed",
  "type": "decision",
  "status": "active",
  "scope": ["requirements", "product"],
  "tags": ["decisions", "settled", "no-relitigate"],
  "created": "2026-08-09",
  "last_verified": "2026-08-09",
  "review_after": "2027-02-09",
  "sources": [
    "specs/requirements-analyst-open-questions.md"
  ],
  "supersedes": [],
  "superseded_by": null,
  "valid_from": "2026-08-09",
  "valid_to": null
}
---

# Owner Decisions A1–A12 and Platform-Fee Call Are Closed

## Durable Context

The owner answered A1, A3, A6 and A8 directly. The rest were carried on recommendations without objection. **These are now decisions, not open questions.** Do not relitigate these in a later session — reopen only if the client changes the underlying epic.

| Decision | Ruling | Epic Impact |
|---|---|---|
| **A1** | All under-18 players require a parent-managed account. COPPA question closed for MVP. No independent 16-18 accounts | Epic-01 BR-01-17 governs. Blocks registration, parent-approval workflow, payment method holders |
| **A2** | User Role Editor is out of MVP. The four roles (`ROLE_SUPER_ADMIN`, `ROLE_TRAINER`, `ROLE_COACH`, `ROLE_PLAYER`) are fixed constants, not runtime-editable data | Epic-07 governs over the plan. Role hierarchy is not configured; each Super Admin capability is explicit in a voter |
| **A3** | A camp registrant who never converts gets a form submission plus a camp payment record, and no account. Auto-creating a shadow account is explicitly rejected | Epic-08 requires a fourth CRM association source (`camp_registration`). Blocks Epic-08 to be built correctly without reopening CRM rules |
| **A4** | Camp money is recorded platform-side against the submission, not against a player account, so it can be reconciled and refunded | Follows from A3. Epic-05 payment records design affects camp path |
| **A5** | On conversion, the earlier camp payment and registration attach to the new account. Conversion is an additive link, not a re-modelling | Affects refunds and transaction history for converted registrants |
| **A6** | No combined cross-trainer view on any player-facing screen. Tenancy isolation wins. Epic-01 AC-01-15 governs; Epic-05 AC-05-24's "Family Overview" is dropped | Blocks cross-trainer view logic anywhere in the platform. Single trainer context only |
| **A7** | Every token spend records the beneficiary player, even though the balance sits at the parent-trainer pair | Cheap to capture now, unreconstructable later. Blocks spending logic to record player |
| **A8** | Content is sold as per-playlist one-time purchase. The other three candidate models (all-content subscription, bundle pricing, event-subscription inclusion) are out of MVP. The purchase record must not foreclose adding them later | Epic-04 paywall rules settled. Blocks content purchase flow and Epic-05 integration |
| **A9** | Playlist visibility is three-state: `public` / `private` / `coach-only`, not binary. The storage splits visibility (in public library) and audience (players and coaches, or coaches only) into two orthogonal facts | Epic-04 visibility model. Their three valid combinations are exactly A9's three states |
| **A10** | Build order: `01 → 02 → {03, 04} → 05 → {06, 07, 08}`. Derived from each epic's own "Required Before This Epic". The plan's diagram is wrong about Epic-08 | Epic-08 requires Epic-05 (Stripe Checkout must exist first). Blocks implementation scheduling |
| **A11** | Epic-04 content structure first, then Epic-05, then wire the paywall. Changes what "Epic-04 done" means | Affects the dependency between content and billing. Paywall wiring is the last step |
| **A12** | Proceed with 85 user stories. US-02.09 does not exist in the source; report it to the client rather than inventing it | Platform total is 85 stories present. Numbering gap, not a missing story |
| **Fee** | Trainer absorbs the 5% platform fee; it is included in the listed price | Epic-05 BR-05-7 governs. Blocks price calculation and fee logic |

## Consequences

**When implementing any epic:**

- A1 and A2 set the authorization model: four fixed roles, all under-18 players must have a parent.
- A3, A4, A5 set the camp registration path: form submission (unauthenticated), payment record (no account required), additive conversion.
- A6 sets the data access model: never cross-trainer on a player-facing screen.
- A7 affects every token spend: record the beneficiary player.
- A8 and A9 set the content model: per-playlist one-time purchase, three-state visibility.
- A10 and A11 determine the build sequence: content before billing, then wire the paywall.
- The fee decision affects AC-05-3 (trainer discount calculation in API response): fee is included, not added.

**When a later session asks "should we support X," look at the underlying epic first:**

- If the epic is unchanged, the decision is final.
- If the client changes the epic, reopen the decision using the same council process.
- Do not invent new questions in between.

## Verification

Each decision is cited from the source epic at `specs/requirements-analyst-open-questions.md` Section A. Re-read that section before proposing to change any of them.
