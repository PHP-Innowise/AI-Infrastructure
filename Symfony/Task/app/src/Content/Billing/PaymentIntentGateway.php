<?php

declare(strict_types=1);

namespace App\Content\Billing;

/**
 * The narrow seam between Content and Billing (Epic-05). Content owns this
 * interface; `App\Billing\Service\ContentPaymentIntentGateway` is Epic-05's
 * real implementation, registered in `config/services.yaml` in place of the
 * Epic-04 shipped `NoopPaymentIntentGateway` (removed — see git history).
 *
 * No `lockFundingForUpdate()` counterpart to
 * `App\Scheduling\Billing\PaymentIntentGateway`'s own: a content purchase
 * never locks a second, capacity-shaped row the way a paid RSVP locks the
 * event row (`PlaylistAccessGrant` uniqueness is a plain unique index, not
 * a manually-locked counter), so there is no second lock to sequence
 * against — architecture's "lock ordering" rule has nothing to do here.
 *
 * `PurchasePlaylistAccessService` is the only caller.
 */
interface PaymentIntentGateway
{
    /**
     * BR-04-6/7: request that a charge (or token spend) be taken to unlock a
     * playlist. Must not grant access itself — the caller does that only on
     * `PaymentIntentOutcome::Succeeded`.
     */
    public function requestPayment(PaymentIntentRequest $request): PaymentIntentResult;
}
