<?php

declare(strict_types=1);

namespace App\Scheduling\Billing;

/**
 * The three-case result of asking `PaymentIntentGateway` to take or refund
 * a payment — wrapped by `PaymentIntentResult` (see its own docblock for
 * why a bare enum cannot carry the extra Epic-05 information alongside
 * this). Scheduling never inspects anything about a payment beyond "did
 * this succeed" (BR-02-9: "the RSVP is not confirmed until payment
 * succeeds").
 */
enum PaymentIntentOutcome
{
    /**
     * A `token` spend completed synchronously, inside the same transaction
     * that requested it.
     */
    case Succeeded;

    /**
     * A `card` payment: Stripe Checkout has been created and the caller
     * must redirect there (`PaymentIntentResult::$redirectUrl`); the RSVP
     * stays pending until the webhook resolves it
     * (`App\Billing\MessageHandler\ProcessStripeWebhookEventHandler`).
     */
    case Pending;

    case Failed;
}
