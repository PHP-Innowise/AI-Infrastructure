<?php

declare(strict_types=1);

namespace App\Billing\Stripe;

/**
 * A verified, decoded Stripe `Event` — the result of
 * `StripeClient::verifyWebhookSignature()`. `$data` is the event's own
 * `data.object`, decoded to an associative array (never re-serialized: the
 * signature was already checked against the exact raw bytes before this
 * object exists).
 */
final readonly class StripeWebhookEvent
{
    /**
     * @param array<string, mixed> $data
     */
    public function __construct(
        public string $id,
        public string $type,
        public array $data,
    ) {
    }
}
