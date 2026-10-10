<?php

declare(strict_types=1);

namespace App\Billing\Stripe;

/**
 * What `StripeGateway` asks `StripeClient::createCheckoutSession()` for.
 * `applicationFeeMinorUnits`/`connectedAccountId` are null for a payment
 * that does not move through a trainer's Connect account (the trainer's
 * own platform subscription has no application fee; a player subscription
 * purchase is billed to the platform, not Connect-routed, since it funds
 * an entitlement rather than a per-trainer charge — see
 * `SubscriptionPurchaseService`'s own docblock for why).
 */
final readonly class StripeCheckoutSessionRequest
{
    public const MODE_PAYMENT = 'payment';
    public const MODE_SUBSCRIPTION = 'subscription';

    /**
     * @param array<string, string> $metadata Echoed back verbatim on the
     *                                        resulting PaymentIntent/Session, retrievable from the webhook
     *                                        payload later — Stripe's own idiomatic mechanism for carrying
     *                                        request-time information forward to webhook-confirmation time,
     *                                        used here for AC-05-29's purchaser-chosen subscription
     *                                        activation date (`PaymentRecord` has no column for it — a
     *                                        `SubscriptionEntitlement` is granted only once payment is
     *                                        confirmed, per BR-05-14's "no grace period", so this is the one
     *                                        value that must survive the gap between request and webhook).
     */
    public function __construct(
        public string $mode,
        public int $amountMinorUnits,
        public string $description,
        public string $successUrl,
        public string $cancelUrl,
        public string $idempotencyKey,
        public ?string $stripeCustomerId = null,
        public ?string $connectedAccountId = null,
        public ?int $applicationFeeMinorUnits = null,
        public array $metadata = [],
    ) {
    }
}
