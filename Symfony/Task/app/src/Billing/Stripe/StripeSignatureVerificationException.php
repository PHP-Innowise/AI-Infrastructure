<?php

declare(strict_types=1);

namespace App\Billing\Stripe;

/**
 * AC-05-36: a `Stripe-Signature` header that does not match the raw body —
 * `BillingWebhookController` maps this to a bare `400`, per
 * specs/api-designer-spec.md "Stripe webhook contract" step 2.
 */
final class StripeSignatureVerificationException extends \RuntimeException
{
}
