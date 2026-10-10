<?php

declare(strict_types=1);

namespace App\Tests\Billing;

use App\Billing\Service\FeeCalculator;
use PHPUnit\Framework\TestCase;

/**
 * BR-05-7, AC-05-11: the 5% platform fee, round-half-up, integer-only.
 *
 * @see specs/database-designer-schema.md "Money and the platform fee"
 */
final class FeeCalculatorTest extends TestCase
{
    private FeeCalculator $calculator;

    protected function setUp(): void
    {
        $this->calculator = new FeeCalculator();
    }

    /**
     * BR-05-7: "$100 event, 5%... the $5 (5%) platform fee."
     */
    public function testHundredDollarEventAtFivePercentIsExactlyFiveDollars(): void
    {
        self::assertSame(500, $this->calculator->compute(10000, 500), 'BR-05-7: $100 at 5% is exactly $5.00.');
    }

    /**
     * AC-05-11: "a $20 USD event charge, Stripe processes the payment with
     * a platform fee of $1 (5%)."
     */
    public function testTwentyDollarEventAtFivePercentIsExactlyOneDollar(): void
    {
        self::assertSame(100, $this->calculator->compute(2000, 500), 'AC-05-11: $20 at 5% is exactly $1.00.');
    }

    /**
     * A genuine half-cent case confirms round-half-up (away from zero), not
     * banker's rounding: intdiv(10*500 + 5000, 10000) = intdiv(10000, 10000) = 1.
     */
    public function testExactHalfCentRoundsUpNotToEven(): void
    {
        self::assertSame(1, $this->calculator->compute(10, 500), 'A 0.5-cent fee rounds up to 1 cent (round-half-up), not down to 0.');
    }

    public function testZeroAmountYieldsZeroFee(): void
    {
        self::assertSame(0, $this->calculator->compute(0, 500));
    }

    public function testZeroRateYieldsZeroFee(): void
    {
        self::assertSame(0, $this->calculator->compute(10000, 0));
    }

    /**
     * AC-05-27: a custom, non-default fee rate (Super Admin per-trainer
     * override) computes correctly too — not hardcoded to 500 bps.
     */
    public function testCustomFeeRateComputesCorrectly(): void
    {
        // $50.00 at 10% (1000 bps) = $5.00 exactly.
        self::assertSame(500, $this->calculator->compute(5000, 1000));
    }

    public function testRejectsNegativeAmount(): void
    {
        $this->expectException(\InvalidArgumentException::class);
        $this->calculator->compute(-1, 500);
    }

    public function testRejectsOutOfRangeFeeRate(): void
    {
        $this->expectException(\InvalidArgumentException::class);
        $this->calculator->compute(1000, 10001);
    }
}
