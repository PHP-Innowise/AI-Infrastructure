<?php

declare(strict_types=1);

namespace App\Billing\Message;

/**
 * Dispatched by `BillingWebhookController` once a Stripe event's signature
 * is verified and its `StripeEventReceipt` row is inserted. Carries ONLY
 * the receipt's own id — never the payload, never the event type — matching
 * `examples/symfony-clean-code-patterns.md` §8's "the payload is stable"
 * guidance (specs/api-designer-spec.md "Stripe webhook contract" step 5):
 * the handler re-reads `rawPayload` from the persisted row rather than
 * trusting a payload that traveled through the queue.
 */
final readonly class ProcessStripeWebhookEvent
{
    public function __construct(
        public int $stripeEventReceiptId,
    ) {
    }
}
