<?php

declare(strict_types=1);

namespace App\Billing\Stripe;

final readonly class StripeRefund
{
    public function __construct(
        public string $refundId,
        public bool $succeeded,
    ) {
    }
}
