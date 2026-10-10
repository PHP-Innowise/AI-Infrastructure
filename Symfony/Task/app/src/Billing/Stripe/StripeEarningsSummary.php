<?php

declare(strict_types=1);

namespace App\Billing\Stripe;

/**
 * AC-05-25: "current-period earnings, next payout date, last payout
 * amount, and total lifetime earnings" — read live from Stripe by
 * `StripeReportingReader`, the only source of any such figure
 * (architect-architecture.md "The earnings boundary"). Nothing here is
 * ever persisted.
 */
final readonly class StripeEarningsSummary
{
    public function __construct(
        public int $currentPeriodEarningsMinorUnits,
        public ?\DateTimeImmutable $nextPayoutDate,
        public ?int $lastPayoutAmountMinorUnits,
        public int $lifetimeEarningsMinorUnits,
    ) {
    }
}
