<?php

declare(strict_types=1);

namespace App\Growth\Exception;

use App\Growth\Entity\Coupon;

/**
 * US-06.07's own open question (a coupon that has already been used cannot
 * simply vanish out from under its redemption history) — this codebase's
 * resolution: deletion is blocked outright once a coupon has any use,
 * matching AC-06-27's literal "delete it if it has never been used" (the
 * converse case needs SOME defined behavior; deactivation remains available
 * as the "stop new uses" alternative already named in the same criterion).
 */
final class CouponInUseException extends \RuntimeException
{
    public static function forCoupon(Coupon $coupon): self
    {
        return new self(sprintf(
            'Coupon "%s" has already been used %d time(s) and cannot be deleted; deactivate it instead.',
            $coupon->getCode(),
            $coupon->getUsageCount(),
        ));
    }
}
