<?php

declare(strict_types=1);

namespace App\Growth\Dto;

/**
 * AC-06-28: "Analytics Summary (all coupons combined)."
 */
final readonly class CouponAnalyticsSummary
{
    public function __construct(
        public int $totalCouponsCreated,
        public int $activeCoupons,
        public int $usesThisMonth,
        public int $discountGivenThisMonthMinorUnits,
        public int $revenueThisMonthMinorUnits,
    ) {
    }
}
