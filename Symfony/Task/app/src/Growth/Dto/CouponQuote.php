<?php

declare(strict_types=1);

namespace App\Growth\Dto;

use App\Growth\Entity\Coupon;

/**
 * The result of validating and pricing a coupon code against one purchase —
 * AC-06-21/22/23's "exists, active, unexpired, under limit, player
 * eligible" check plus BR-06-9's discount arithmetic, in one immutable
 * result both the JSON preview endpoint
 * (`growth_portal_coupon_validate`) and the real checkout flows
 * (`PurchasePlaylistAccessService`, `RsvpService`) consume.
 */
final readonly class CouponQuote
{
    private function __construct(
        public bool $valid,
        public string $message,
        public ?Coupon $coupon,
        public ?int $originalAmountMinorUnits,
        public ?int $discountAmountMinorUnits,
        public ?int $finalAmountMinorUnits,
    ) {
    }

    public static function invalid(string $message): self
    {
        return new self(false, $message, null, null, null, null);
    }

    /**
     * BR-06-9: the discount is computed here (percentage of the original
     * price, or a flat amount), floored at 0.
     */
    public static function valid(Coupon $coupon, int $originalAmountMinorUnits): self
    {
        $discount = Coupon::DISCOUNT_PERCENTAGE === $coupon->getDiscountType()
            ? (int) round($originalAmountMinorUnits * $coupon->getDiscountValue() / 100)
            : $coupon->getDiscountValue();

        $discount = min($discount, $originalAmountMinorUnits);
        $final = $originalAmountMinorUnits - $discount;

        return new self(
            true,
            sprintf('%s off with %s', self::formatDiscount($coupon), $coupon->getCode()),
            $coupon,
            $originalAmountMinorUnits,
            $discount,
            $final,
        );
    }

    private static function formatDiscount(Coupon $coupon): string
    {
        return Coupon::DISCOUNT_PERCENTAGE === $coupon->getDiscountType()
            ? sprintf('%d%%', $coupon->getDiscountValue())
            : sprintf('$%s', number_format($coupon->getDiscountValue() / 100, 2));
    }
}
