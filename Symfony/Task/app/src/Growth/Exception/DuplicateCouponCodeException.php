<?php

declare(strict_types=1);

namespace App\Growth\Exception;

/**
 * AC-06-18: "the code must be unique within the trainer's own coupons."
 */
final class DuplicateCouponCodeException extends \RuntimeException
{
    public static function forCode(string $code): self
    {
        return new self(sprintf('A coupon with the code "%s" already exists.', $code));
    }
}
