<?php

declare(strict_types=1);

namespace App\Content\Billing;

/**
 * Wraps `PaymentIntentOutcome` with the two pieces Epic-05 adds — see
 * `App\Scheduling\Billing\PaymentIntentResult`'s own docblock for the full
 * reasoning, kept as a separate class per the module boundary (matching
 * `PaymentIntentOutcome`'s own precedent: Content must not depend on
 * Scheduling).
 */
final readonly class PaymentIntentResult
{
    private function __construct(
        public PaymentIntentOutcome $outcome,
        public ?int $paymentRecordId,
        public ?string $redirectUrl,
    ) {
    }

    /**
     * $paymentRecordId is nullable so a test stub can report Succeeded
     * without a real, persisted `PaymentRecord` — see
     * `App\Scheduling\Billing\PaymentIntentResult::succeeded()`'s own
     * docblock for the identical reasoning.
     */
    public static function succeeded(?int $paymentRecordId = null): self
    {
        return new self(PaymentIntentOutcome::Succeeded, $paymentRecordId, null);
    }

    public static function pending(int $paymentRecordId, ?string $redirectUrl = null): self
    {
        return new self(PaymentIntentOutcome::Pending, $paymentRecordId, $redirectUrl);
    }

    public static function failed(): self
    {
        return new self(PaymentIntentOutcome::Failed, null, null);
    }
}
