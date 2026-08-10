<?php

declare(strict_types=1);

namespace App\Content\Billing;

/**
 * The Epic-04 shipped implementation of `PaymentIntentGateway`. It cannot
 * move money — there is no Stripe integration and no token ledger to spend
 * against yet (Billing is Epic-05) — and it never pretends otherwise: every
 * call returns `Pending`, never `Succeeded`. A playlist purchase attempt
 * therefore always stays locked today; nothing here fabricates a successful
 * charge or writes a token-ledger row.
 *
 * Mirrors `App\Scheduling\Billing\NoopPaymentIntentGateway`'s own precedent
 * and reasoning exactly. Once Epic-05 exists, `Billing` registers its own
 * implementation of `App\Content\Billing\PaymentIntentGateway` and this class
 * is removed — nothing in `PurchasePlaylistAccessService` needs to change,
 * only the service wired behind the interface.
 */
final class NoopPaymentIntentGateway implements PaymentIntentGateway
{
    public function requestPayment(PaymentIntentRequest $request): PaymentIntentOutcome
    {
        return PaymentIntentOutcome::Pending;
    }
}
