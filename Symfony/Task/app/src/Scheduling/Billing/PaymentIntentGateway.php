<?php

declare(strict_types=1);

namespace App\Scheduling\Billing;

/**
 * The narrow seam between Scheduling and Billing (Epic-05), which does not
 * exist yet. Scheduling owns this interface; Billing will implement it
 * against Stripe (for `usd`) and `TokenLedgerService` (for `token`) once it
 * lands (architect-architecture.md "Module map": "Until Billing exists,
 * Content and Scheduling depend on a narrow payment-intent interface that
 * Billing later implements").
 *
 * `App\Scheduling\Billing\NoopPaymentIntentGateway` is the only
 * implementation shipped with Epic-02 — it cannot move money and never
 * claims to (see its own docblock). This interface exists so `RsvpService`'s
 * paid-RSVP and refund logic is fully exercised by tests today, against a
 * real contract, without Scheduling ever importing a Stripe type or writing
 * a token-ledger row.
 */
interface PaymentIntentGateway
{
    /**
     * BR-02-9: request that a charge (or token spend) be taken for a paid
     * RSVP. Must not confirm the RSVP itself — the caller does that only on
     * `PaymentIntentOutcome::Succeeded`.
     */
    public function requestPayment(PaymentIntentRequest $request): PaymentIntentOutcome;

    /**
     * BR-02-11/12: request a refund for a canceled paid RSVP. The caller
     * has already decided eligibility (the 24-hour policy, or an
     * unconditional trainer-cancellation refund) before calling this — the
     * gateway only attempts the transfer.
     */
    public function requestRefund(PaymentIntentRequest $request): PaymentIntentOutcome;
}
