<?php

declare(strict_types=1);

namespace App\Forms\Dto;

use App\Forms\Entity\FormSubmission;

/**
 * What `FormSubmissionService::submit()` hands back to
 * `PublicFormController` — enough to decide the AC-08-15 branch (redirect to
 * confirmation now, or redirect to Stripe Checkout first) without the
 * controller re-deriving `Form::isFree()` itself.
 */
final readonly class SubmissionOutcome
{
    private function __construct(
        public FormSubmission $submission,
        public bool $requiresPayment,
        public ?string $redirectUrl,
    ) {
    }

    public static function confirmed(FormSubmission $submission): self
    {
        return new self($submission, false, null);
    }

    public static function pendingPayment(FormSubmission $submission, string $redirectUrl): self
    {
        return new self($submission, true, $redirectUrl);
    }
}
