<?php

declare(strict_types=1);

namespace App\Growth\Service;

use App\Growth\Dto\CouponAnalyticsRow;
use App\Growth\Dto\CouponAnalyticsSummary;
use App\Growth\Entity\Coupon;
use App\Growth\Repository\CouponRedemptionRepository;
use App\Growth\Repository\CouponRepository;
use App\Platform\Entity\Trainer;

/**
 * US-06.07: coupon analytics read model.
 *
 * @see specs/requirements-analyst-epic-06-marketing-growth-spec.md AC-06-26..28
 */
final readonly class CouponAnalyticsService
{
    public function __construct(
        private CouponRepository $coupons,
        private CouponRedemptionRepository $redemptions,
    ) {
    }

    public function rowFor(Coupon $coupon): CouponAnalyticsRow
    {
        $totals = $this->redemptions->totalsForCoupon($coupon);
        $usageLimit = $coupon->getUsageLimit();

        return new CouponAnalyticsRow(
            coupon: $coupon,
            status: $this->statusFor($coupon, new \DateTimeImmutable()),
            totalUses: $totals['uses'],
            totalDiscountGivenMinorUnits: $totals['discountGiven'],
            revenueGeneratedMinorUnits: $totals['revenueGenerated'],
            conversionRatePercent: (null !== $usageLimit && $usageLimit > 0)
                ? round(100 * $totals['uses'] / $usageLimit, 1)
                : null,
        );
    }

    /**
     * AC-06-26: the trainer's full coupon list, newest first.
     *
     * @return list<CouponAnalyticsRow>
     */
    public function allRows(): array
    {
        return array_map($this->rowFor(...), $this->coupons->findAllForActiveTenant());
    }

    public function summary(Trainer $trainer, \DateTimeImmutable $monthStart, \DateTimeImmutable $monthEnd): CouponAnalyticsSummary
    {
        $monthTotals = $this->redemptions->totalsBetween($trainer, $monthStart, $monthEnd);

        return new CouponAnalyticsSummary(
            totalCouponsCreated: $this->coupons->countCreated($trainer),
            activeCoupons: $this->coupons->countActive($trainer),
            usesThisMonth: $monthTotals['uses'],
            discountGivenThisMonthMinorUnits: $monthTotals['discountGiven'],
            revenueThisMonthMinorUnits: $monthTotals['revenueGenerated'],
        );
    }

    private function statusFor(Coupon $coupon, \DateTimeImmutable $now): string
    {
        if (!$coupon->isActive()) {
            return CouponAnalyticsRow::STATUS_INACTIVE;
        }

        if ($coupon->isExpired($now)) {
            return CouponAnalyticsRow::STATUS_EXPIRED;
        }

        return CouponAnalyticsRow::STATUS_ACTIVE;
    }
}
