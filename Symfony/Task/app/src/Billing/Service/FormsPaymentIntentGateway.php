<?php

declare(strict_types=1);

namespace App\Billing\Service;

use App\Billing\Entity\PaymentRecord;
use App\Billing\Repository\PaymentRecordRepository;
use App\Forms\Billing\PaymentIntentGateway;
use App\Forms\Billing\PaymentIntentRequest;
use App\Forms\Billing\PaymentIntentResult;
use Doctrine\ORM\EntityManagerInterface;

/**
 * Epic-08's real `App\Forms\Billing\PaymentIntentGateway`: Stripe Checkout
 * only — BR-08-11 ("all payments are processed via Stripe Checkout; there is
 * no custom payment form"), so unlike `SchedulingPaymentIntentGateway`/
 * `ContentPaymentIntentGateway` there is no `token` branch here at all.
 *
 * Every `PaymentRecord` this class creates has `type = camp_registration`,
 * `paymentMethod = card`, **`payerAccount = null`** (A3/A4: a camp
 * registrant has no account), and `relatedFormSubmission` set. This is the
 * only code in the platform that constructs a `camp_registration`
 * `PaymentRecord` — Forms itself never does (module map: Forms "Must not:
 * ...write a payment record itself").
 *
 * @see specs/requirements-analyst-open-questions.md "A3", "A4"
 * @see specs/requirements-analyst-epic-08-forms-registration-spec.md BR-08-8, BR-08-11
 */
final readonly class FormsPaymentIntentGateway implements PaymentIntentGateway
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private StripeGateway $stripeGateway,
        private PaymentRecordRepository $paymentRecords,
    ) {
    }

    public function requestPayment(PaymentIntentRequest $request): PaymentIntentResult
    {
        $paymentRecord = new PaymentRecord(
            $request->trainer,
            PaymentRecord::TYPE_CAMP_REGISTRATION,
            PaymentRecord::METHOD_CARD,
            $request->amountMinorUnits,
            $request->contactName,
            $request->contactEmail,
            null,
            $request->contactPhone,
        );
        $paymentRecord->attachRelatedFormSubmission($request->submission);
        $this->stripeGateway->applyCurrentFee($paymentRecord);
        $this->paymentRecords->add($paymentRecord);
        $this->entityManager->flush();

        $session = $this->stripeGateway->createCheckoutSession(
            $paymentRecord,
            $request->successUrl,
            $request->cancelUrl,
        );

        $paymentRecordId = $paymentRecord->getId();
        \assert(null !== $paymentRecordId);

        return PaymentIntentResult::pending($paymentRecordId, $session->checkoutUrl);
    }
}
