<?php

declare(strict_types=1);

namespace App\Scheduling\Billing;

/**
 * The Epic-02 shipped implementation of `PaymentIntentGateway`. It cannot
 * move money — there is no Stripe integration and no token ledger to spend
 * against yet (Billing is Epic-05) — and it never pretends otherwise: every
 * call returns `Pending`, never `Succeeded`. A paid RSVP therefore always
 * stays in "pending_payment" today; nothing here fabricates a successful
 * charge or a processed refund.
 *
 * This is the "in-memory/no-op implementation" the task brief calls for:
 * marks the paid path clearly (every paid RSVP visibly stalls at
 * pending_payment rather than silently confirming for free) without
 * building a payment gateway or writing a token-ledger row. Once Epic-05
 * exists, `Billing` registers its own implementation of
 * `PaymentIntentGateway` (Stripe for `usd`, `TokenLedgerService::spend` for
 * `token`) and this class is removed — nothing in `RsvpService` needs to
 * change, only the service wired behind the interface.
 */
final class NoopPaymentIntentGateway implements PaymentIntentGateway
{
    public function requestPayment(PaymentIntentRequest $request): PaymentIntentOutcome
    {
        return PaymentIntentOutcome::Pending;
    }

    public function requestRefund(PaymentIntentRequest $request): PaymentIntentOutcome
    {
        return PaymentIntentOutcome::Pending;
    }
}
