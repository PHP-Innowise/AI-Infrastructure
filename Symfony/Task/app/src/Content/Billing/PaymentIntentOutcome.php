<?php

declare(strict_types=1);

namespace App\Content\Billing;

/**
 * The result of asking `PaymentIntentGateway` to take a payment for a
 * playlist purchase. Deliberately the same three-case shape as
 * `App\Scheduling\Billing\PaymentIntentOutcome` — NOT the same class,
 * because Content must not depend on Scheduling (architect-architecture.md
 * "Module map": Content "may call: Platform, Identity, Billing... Crm" —
 * Scheduling is not on that list). Each module owns its own narrow seam to
 * the not-yet-existing Billing module.
 */
enum PaymentIntentOutcome
{
    case Succeeded;

    /**
     * Requires a step Content does not own (a client-side Stripe Checkout
     * redirect, a webhook) — the purchase stays unlocked-pending until
     * Billing (Epic-05) resolves it. This is the only outcome the shipped
     * `NoopPaymentIntentGateway` ever returns.
     */
    case Pending;

    case Failed;
}
