# Epic 07: Stripe Platform Subscriptions & Billing

## Outcome

A Mandant administrator can subscribe Project to a platform plan through Stripe-hosted payment pages, manage billing, view invoices, and receive access according to synchronized subscription state.

**Scope status:** New planned scope. No payment implementation exists in the Project commit history.

## Billing model

- The subscriber is the Mandant, not an individual building, project, task, contractor, or resident.
- Stripe is the payment and invoice system of record; Project stores only identifiers and the local entitlement projection needed at runtime.
- One Mandant maps to one Stripe Customer and at most one current platform Subscription.
- Payment details are collected only by Stripe Checkout or Customer Portal.

## Capability tasks

### E07-T01 — Plan catalogue

- [ ] Define the permitted platform plans and their Stripe Price identifiers on the server.
- [ ] Show plan name, billing interval, currency, and price before Checkout.
- [ ] Reject arbitrary or inactive Price identifiers supplied by a client.
- [ ] Keep feature entitlements mapped to stable internal plan keys rather than display text.

### E07-T02 — Mandant billing identity

- [ ] Create or reuse one Stripe Customer for a Mandant.
- [ ] Persist the Stripe Customer identifier against the Mandant billing record.
- [ ] Store the current Stripe Subscription identifier, Price identifier, status, period end, and cancellation state.
- [ ] Enforce uniqueness so one Stripe Customer or Subscription cannot be assigned to multiple Mandants.
- [ ] Never store card numbers, CVC, or bank-account credentials.

### E07-T03 — Subscription Checkout

- [ ] Allow only an authorized Mandant administrator to start Checkout.
- [ ] Build the Checkout Session server-side from an allow-listed plan.
- [ ] Attach the Mandant reference as non-sensitive metadata.
- [ ] Use an idempotency key so retries cannot create duplicate subscriptions.
- [ ] Treat the success redirect as presentation only; grant access from verified webhook state.

### E07-T04 — Customer Portal

- [ ] Allow an authorized Mandant administrator to open Stripe Customer Portal.
- [ ] Create the portal session server-side for the Mandant's stored Stripe Customer.
- [ ] Let Stripe handle payment-method changes, invoice access, plan changes, and cancellation according to configured portal policy.
- [ ] Refuse portal creation when the Customer does not belong to the active Mandant.

### E07-T05 — Verified and idempotent webhooks

- [ ] Read the unmodified request body and verify the Stripe signature before processing.
- [ ] Reject invalid signatures without changing billing state.
- [ ] Persist each processed Stripe Event identifier under a unique constraint.
- [ ] Return success for an already processed event without applying it twice.
- [ ] Process events safely out of order by retrieving or comparing the authoritative Stripe object when required.
- [ ] Handle at least Checkout completion, subscription created/updated/deleted, invoice paid, and invoice payment failed.
- [ ] Log identifiers and outcomes without secrets or payment details.

### E07-T06 — Subscription state and entitlement projection

- [ ] Synchronize Stripe status, Price, current-period end, cancellation flags, and last event time into the Mandant billing record.
- [ ] Grant normal platform access for `active` and `trialing` subscriptions.
- [ ] Keep access through the paid period when cancellation is scheduled at period end.
- [ ] Keep `past_due` access while Stripe's retry flow remains active, while showing a billing warning to Mandant administrators.
- [ ] Block tenant application access for `unpaid`, `incomplete_expired`, or ended subscriptions, while retaining a billing-management path.
- [ ] Never block platform operators from the support path needed to repair billing state.

### E07-T07 — Billing status and invoices

- [ ] Show the current plan, status, renewal/end date, and scheduled cancellation to authorized Mandant administrators.
- [ ] Show invoice number, date, currency, amount, status, and Stripe-hosted invoice/PDF links.
- [ ] Keep billing data hidden from users without billing permission.
- [ ] Display payment-failure and cancellation warnings without exposing sensitive payment details.

### E07-T08 — Cancellation, plan change, and resynchronization

- [ ] Apply cancellation and plan changes from verified Stripe state.
- [ ] Preserve local access until the synchronized entitlement says it ends.
- [ ] Provide an operator-only resynchronization action for one Mandant.
- [ ] Make resynchronization idempotent and auditable.
- [ ] Never delete tenant building-register data automatically because a subscription ends.

### E07-T09 — Security and operational checks

- [ ] Keep Stripe secret and webhook-signing keys in deployment secrets, never source control.
- [ ] Separate test and live Stripe identifiers and prevent mode mixing.
- [ ] Apply authorization, CSRF/session protection, and rate limits to billing-session creation.
- [ ] Cover duplicate, invalid-signature, delayed, and out-of-order webhook cases with focused tests.
- [ ] Alert on sustained webhook failures without logging request bodies containing payment data.

## Acceptance criteria

- A Mandant administrator can subscribe through Checkout and manage the subscription through Customer Portal.
- A Checkout redirect alone cannot activate access.
- Replaying a valid webhook does not duplicate state changes, invoices, or audit records.
- An invalid webhook signature cannot change any record.
- Subscription state cannot be attached across Mandants.
- Entitlement checks use synchronized server-side state and preserve a billing-management route when tenant access is blocked.
- Project stores no raw payment-method data.

## Dependencies

- Epic 01 supplies Mandant ownership and billing-administrator authorization.
- Epic 02 supplies authenticated sessions and API security.
- Epic 06 supplies billing screens and notices.
- Epic 08 supplies secrets, webhook routing, monitoring, and deployment configuration.

## Excluded

- Stripe Connect and marketplace accounts.
- Token wallets or prepaid credits.
- Contractor, vendor, construction invoice, or milestone payments.
- Metered/usage billing.
- Custom card-entry forms.
- Automatic deletion of Mandant data after cancellation.

