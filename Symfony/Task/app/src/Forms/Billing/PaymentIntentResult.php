<?php

declare(strict_types=1);

namespace App\Forms\Billing;

/**
 * Wraps `PaymentIntentOutcome` with what `FormSubmissionService` needs back
 * — matching `App\Scheduling\Billing\PaymentIntentResult`'s own shape and
 * reasoning (a bare enum cannot carry per-call data; PHP enum cases are
 * singletons).
 */
final readonly class PaymentIntentResult
{
    private function __construct(
        public PaymentIntentOutcome $outcome,
        public ?int $paymentRecordId,
        public ?string $redirectUrl,
    ) {
    }

    public static function pending(int $paymentRecordId, string $redirectUrl): self
    {
        return new self(PaymentIntentOutcome::Pending, $paymentRecordId, $redirectUrl);
    }

    public static function failed(): self
    {
        return new self(PaymentIntentOutcome::Failed, null, null);
    }
}
