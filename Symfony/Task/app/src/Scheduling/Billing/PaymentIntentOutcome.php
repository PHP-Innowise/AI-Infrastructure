<?php

declare(strict_types=1);

namespace App\Scheduling\Billing;

/**
 * The result of asking `PaymentIntentGateway` to take or refund a payment.
 * Kept deliberately small — Scheduling never inspects anything about a
 * payment beyond "did this succeed" (BR-02-9: "the RSVP is not confirmed
 * until payment succeeds").
 */
enum PaymentIntentOutcome
{
    case Succeeded;

    /**
     * Requires a step Scheduling does not own (a client-side Stripe
     * Checkout redirect, a webhook) — the RSVP stays pending until Billing
     * (Epic-05) resolves it.
     */
    case Pending;

    case Failed;
}
