<?php

declare(strict_types=1);

namespace App\Growth\Dto;

use App\Growth\Entity\Coupon;

/**
 * AC-06-26: one row of the trainer's per-coupon analytics list.
 *
 * `conversionRatePercent` is this codebase's own reading of an
 * under-specified metric — the epic lists "conversion rate" alongside
 * "uses" and "usage limit" as separate coupon-analytics fields but never
 * defines what a coupon "converts" against; no impression/attempt tracking
 * exists anywhere in this schema to compute a rate from (every recorded
 * `CouponRedemption` is, by construction, already a successful purchase).
 * Computed here as uses / usage limit when a limit is set — "how much of
 * its capacity has this coupon converted" — and left `null` ("N/A" at
 * render time) for unlimited coupons, which have no ceiling to rate
 * against. Recorded as a judgment call in the coder's final report, not a
 * resolution of the epic's silence.
 */
final readonly class CouponAnalyticsRow
{
    public const STATUS_ACTIVE = 'active';
    public const STATUS_EXPIRED = 'expired';
    public const STATUS_INACTIVE = 'inactive';

    public function __construct(
        public Coupon $coupon,
        public string $status,
        public int $totalUses,
        public int $totalDiscountGivenMinorUnits,
        public int $revenueGeneratedMinorUnits,
        public ?float $conversionRatePercent,
    ) {
    }
}
