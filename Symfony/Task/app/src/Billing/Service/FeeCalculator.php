<?php

declare(strict_types=1);

namespace App\Billing\Service;

/**
 * BR-05-7: the 5% platform fee, included in the listed price and absorbed
 * by the trainer. Round-half-up, integer arithmetic only — "no floating
 * point anywhere near a money figure"
 * (specs/database-designer-schema.md "Money and the platform fee").
 *
 * Computed once, at `PaymentRecord` creation, from the rate in force at
 * that exact moment, and never recomputed — see `PaymentRecord::applyFee()`.
 *
 * @see specs/database-designer-schema.md "Money and the platform fee"
 */
final class FeeCalculator
{
    /**
     * `platform_fee_minor_units = FLOOR((amount * rate_bps + 5000) / 10000)`
     * — equivalently, round half up to the nearest minor unit (cent).
     *
     * Verified against both worked examples in the epic:
     * - BR-05-7: $100 (10000 minor units), 500 bps -> 500 minor units ($5.00 exactly).
     * - AC-05-11: $20 (2000 minor units), 500 bps -> 100 minor units ($1.00 exactly).
     */
    public function compute(int $amountMinorUnits, int $feeRateBasisPoints): int
    {
        if ($amountMinorUnits < 0) {
            throw new \InvalidArgumentException('An amount cannot be negative.');
        }

        if ($feeRateBasisPoints < 0 || $feeRateBasisPoints > 10000) {
            throw new \InvalidArgumentException('A fee rate must be between 0 and 10000 basis points.');
        }

        return intdiv($amountMinorUnits * $feeRateBasisPoints + 5000, 10000);
    }
}
