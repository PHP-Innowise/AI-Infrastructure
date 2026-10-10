<?php

declare(strict_types=1);

namespace App\Forms\Billing;

use App\Forms\Entity\FormSubmission;
use App\Platform\Entity\Trainer;

/**
 * What Forms tells Billing (Epic-05) it needs paid for one camp/evaluation
 * registration. Deliberately narrow — no Stripe types — matching
 * `App\Scheduling\Billing\PaymentIntentRequest`/`App\Content\Billing\PaymentIntentRequest`'s
 * own shape, with one structural difference: **no `Account $payer`**. A3/A4
 * (owner decision): a camp registrant has no account at all — the payer is a
 * name/email/phone, never an `Account` — so this request carries contact
 * details directly instead.
 */
final readonly class PaymentIntentRequest
{
    public function __construct(
        public Trainer $trainer,
        public FormSubmission $submission,
        public string $contactName,
        public string $contactEmail,
        public ?string $contactPhone,
        public int $amountMinorUnits,
        /**
         * Built by `FormSubmissionService`, not `FormsPaymentIntentGateway`
         * — these are FORMS' own routes (`forms_public_checkout_success`/
         * `_cancel`), unlike Scheduling/Content's success/cancel pages
         * (Billing's own `billing_portal_checkout_*` routes, built inside
         * their gateways). Passing ready-made URLs keeps Billing's
         * implementation ignorant of Forms' route names entirely.
         */
        public string $successUrl,
        public string $cancelUrl,
    ) {
    }
}
