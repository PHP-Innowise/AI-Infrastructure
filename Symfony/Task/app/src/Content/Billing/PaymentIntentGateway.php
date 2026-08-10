<?php

declare(strict_types=1);

namespace App\Content\Billing;

/**
 * The narrow seam between Content and Billing (Epic-05), which does not
 * exist yet — matching `App\Scheduling\Billing\PaymentIntentGateway`'s own
 * precedent exactly (architect-architecture.md "Module map": "Until Billing
 * exists, Content and Scheduling depend on a narrow payment-intent interface
 * that Billing later implements"). Content owns this interface; Billing will
 * implement it once it lands (Stripe for `usd`, `TokenLedgerService` for
 * `token`).
 *
 * Only `requestPayment()` — unlike Scheduling's version, no `requestRefund()`
 * — because no Epic-04 acceptance criterion or business rule describes a
 * content-purchase refund; adding an unused method here would be a seam wider
 * than anything in this epic actually calls.
 *
 * `App\Content\Billing\NoopPaymentIntentGateway` is the only implementation
 * shipped with Epic-04 — it cannot move money and never claims to. This
 * interface exists so `PurchasePlaylistAccessService` is fully exercised by
 * tests against a real contract, without Content ever importing a Stripe
 * type or writing a token-ledger row.
 */
interface PaymentIntentGateway
{
    /**
     * BR-04-6/7: request that a charge (or token spend) be taken to unlock a
     * playlist. Must not grant access itself — the caller does that only on
     * `PaymentIntentOutcome::Succeeded`.
     */
    public function requestPayment(PaymentIntentRequest $request): PaymentIntentOutcome;
}
