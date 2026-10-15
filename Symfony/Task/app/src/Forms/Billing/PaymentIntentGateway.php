<?php

declare(strict_types=1);

namespace App\Forms\Billing;

/**
 * The narrow seam between Forms and Billing (Epic-05), matching
 * `App\Scheduling\Billing\PaymentIntentGateway`/`App\Content\Billing\PaymentIntentGateway`'s
 * own precedent exactly. Forms owns this interface;
 * `App\Billing\Service\FormsPaymentIntentGateway` is the real implementation,
 * registered by Symfony's single-implementation autowiring alias (no
 * explicit `services.yaml` entry needed — the same as the other two).
 *
 * `FormSubmissionService` is the only caller. Module map: Forms "Must not:
 * ...write a payment record itself" — this interface is what keeps that
 * true; `App\Billing\Service\FormsPaymentIntentGateway`, not Forms, is the
 * only code that ever constructs a `PaymentRecord` for a camp registration.
 */
interface PaymentIntentGateway
{
    /**
     * BR-08-8: request that a charge be taken for a paid camp/evaluation
     * registration. Must not confirm the registration itself — the caller
     * only marks it Paid on the async webhook's `PaymentRecordSettled`
     * (BR-08-14), never here.
     */
    public function requestPayment(PaymentIntentRequest $request): PaymentIntentResult;
}
