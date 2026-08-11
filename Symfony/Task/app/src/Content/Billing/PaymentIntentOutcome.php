<?php

declare(strict_types=1);

namespace App\Content\Billing;

/**
 * The three-case result of asking `PaymentIntentGateway` to take a payment
 * for a playlist purchase — wrapped by `PaymentIntentResult` for the same
 * reason `App\Scheduling\Billing\PaymentIntentOutcome`'s own docblock
 * states, kept as a separate class per the module boundary.
 */
enum PaymentIntentOutcome
{
    /**
     * A `token` spend completed synchronously.
     */
    case Succeeded;

    /**
     * A `card` payment: Stripe Checkout has been created and the caller
     * must redirect there (`PaymentIntentResult::$redirectUrl`); access
     * unlocks only once the webhook resolves it.
     */
    case Pending;

    case Failed;
}
