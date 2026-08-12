<?php

declare(strict_types=1);

namespace App\Forms\EventSubscriber;

use App\Billing\Entity\PaymentRecord;
use App\Billing\Event\PaymentRecordSettled;
use App\Forms\Repository\FormSubmissionRepository;
use App\Forms\Service\FormsMailer;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Component\EventDispatcher\Attribute\AsEventListener;

/**
 * BR-08-14: "A Stripe webhook confirms payment, which marks the
 * registration as 'Paid'." Forms' own half of the "domain event dispatched
 * from Billing and subscribed to by the caller" contract — see
 * `App\Scheduling\EventSubscriber\PaymentOutcomeSubscriber`'s own docblock
 * for the full reasoning (Billing never calls Forms directly: module map,
 * "Billing... Must not: Call Scheduling, Content, Forms or Growth"). Filters
 * on `PaymentRecord::TYPE_CAMP_REGISTRATION`; every other type is ignored.
 *
 * Only `OUTCOME_SUCCEEDED` is handled. `OUTCOME_FAILED` needs nothing here:
 * `ProcessStripeWebhookEventHandler::handlePaymentIntentFailed()` already
 * calls `BillingMailer::sendPaymentFailed()` generically for every payment
 * record type via `PaymentRecord::$contactEmail` (never `$payerAccount`,
 * which a camp registration never has — A3/A4), so a failed camp payment is
 * already fully handled before this subscriber ever runs.
 * `OUTCOME_REFUNDED` needs nothing here either: `form_submission.
 * payment_status` has no "refunded" state (schema doc: free|paid|pending
 * only) — "Refunded" is derived from the `PaymentRecord` refund row's own
 * existence, exactly as it already is for every other payment type
 * (`PaymentRecord`'s own docblock), and AC-08-18's filter list never names
 * a "Refunded" option for the trainer's participant view.
 *
 * @see specs/requirements-analyst-epic-08-forms-registration-spec.md BR-08-14
 */
final readonly class PaymentOutcomeSubscriber
{
    public function __construct(
        private FormSubmissionRepository $submissions,
        private EntityManagerInterface $entityManager,
        private FormsMailer $mailer,
    ) {
    }

    #[AsEventListener]
    public function onPaymentRecordSettled(PaymentRecordSettled $event): void
    {
        if (PaymentRecord::TYPE_CAMP_REGISTRATION !== $event->type
            || PaymentRecordSettled::OUTCOME_SUCCEEDED !== $event->outcome
            || null === $event->relatedFormSubmissionId
        ) {
            return;
        }

        $submission = $this->submissions->find($event->relatedFormSubmissionId);

        if (null === $submission || $submission->isConfirmed()) {
            // Already handled (redelivery) or somehow gone — idempotent
            // no-op, matching handlePaymentIntentSucceeded()'s own guard.
            return;
        }

        $submission->markPaid();
        $this->entityManager->flush();

        // AC-08-16: the confirmation email only goes out once payment is
        // actually confirmed — a free registration's own confirmation was
        // already sent synchronously at submission time
        // (FormSubmissionService::submit()); this is the paid path's only
        // send.
        $this->mailer->sendConfirmation($submission);
    }
}
