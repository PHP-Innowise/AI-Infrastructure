<?php

declare(strict_types=1);

namespace App\Growth\Exception;

/**
 * AC-06-23: "Invalid or expired code" — thrown by the checkout-time
 * (Scheduling/Content) integration when a submitted coupon code fails
 * `CouponPricingService::quote()`'s validation. The JSON preview endpoint
 * (`growth_portal_coupon_validate`) does NOT throw this — it returns a
 * `CouponQuote` with `valid: false` instead, since a preview failure is not
 * exceptional there.
 */
final class InvalidCouponException extends \RuntimeException
{
    public static function withMessage(string $message): self
    {
        return new self($message);
    }
}
