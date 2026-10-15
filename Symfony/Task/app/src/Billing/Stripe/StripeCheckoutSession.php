<?php

declare(strict_types=1);

namespace App\Billing\Stripe;

/**
 * The result of creating a Checkout Session — a URL to redirect the browser
 * to (used exactly once, in the same request, never persisted — see
 * `PaymentRecord`'s own docblock) plus the underlying PaymentIntent id
 * (`mode=payment`, captured immediately at session-creation time) or
 * Subscription id (`mode=subscription`), whichever the request asked for.
 */
final readonly class StripeCheckoutSession
{
    public function __construct(
        public string $checkoutUrl,
        public ?string $paymentIntentId = null,
        public ?string $subscriptionId = null,
    ) {
    }
}
