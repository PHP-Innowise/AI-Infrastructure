<?php

declare(strict_types=1);

namespace App\Growth\Dto;

/**
 * AC-06-14: the trainer referral dashboard's overview metrics.
 */
final readonly class ReferralOverviewMetrics
{
    public function __construct(
        public int $totalReferralsThisMonth,
        public int $totalConversionsThisMonth,
        public float $conversionRatePercent,
        public int $totalReferralRevenueMinorUnits,
    ) {
    }
}
