<?php

declare(strict_types=1);

namespace App\Forms\Service;

use App\Billing\Entity\PaymentRecord;
use App\Forms\Billing\PaymentIntentGateway;
use App\Forms\Billing\PaymentIntentOutcome;
use App\Forms\Billing\PaymentIntentRequest;
use App\Forms\Dto\FormField;
use App\Forms\Dto\SubmissionOutcome;
use App\Forms\Entity\Form;
use App\Forms\Entity\FormSubmission;
use App\Forms\Exception\CampFullException;
use App\Forms\Exception\DuplicateSubmissionException;
use App\Forms\Repository\FormRepository;
use App\Forms\Repository\FormSubmissionRepository;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Component\Routing\Generator\UrlGeneratorInterface;

/**
 * US-08.03: the public, unauthenticated submission itself. The only place in
 * this platform an anonymous request writes a trainer-scoped row (schema
 * doc, `form_submission`'s own heading).
 *
 * **Lock ordering, mirroring `RsvpService::rsvp()` exactly**: capacity is
 * locked, counted and the row inserted inside ONE transaction that commits
 * before any external network call — the Stripe Checkout Session request
 * (a real network round-trip) happens AFTER that transaction commits, never
 * while the form row's lock is held. A free registration's confirmation
 * email is sent INSIDE the transaction instead, matching
 * `PlayerRegistrationService`'s own precedent for a same-request, no-network
 * side effect.
 *
 * @see specs/database-designer-schema.md "`form_submission`"
 * @see specs/requirements-analyst-epic-08-forms-registration-spec.md US-08.03, AC-08-13..16, BR-08-6..11
 */
final readonly class FormSubmissionService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private FormRepository $forms,
        private FormSubmissionRepository $submissions,
        private PaymentIntentGateway $paymentGateway,
        private FormsMailer $mailer,
        private UrlGeneratorInterface $urlGenerator,
        private SubmissionTokenFactory $tokens,
    ) {
    }

    /**
     * @param array<string, mixed> $answers field-id -> answer map, already
     *                                       validated against $form->getFields() by the caller's dynamic
     *                                       Symfony Form (honeypot field excluded by the caller before this
     *                                       is ever reached)
     *
     * @throws DuplicateSubmissionException
     * @throws CampFullException
     */
    public function submit(Form $form, array $answers, ?string $contactPhone): SubmissionOutcome
    {
        $formId = $form->getId();
        \assert(null !== $formId);

        $contactName = $this->stringAnswer($answers, FormField::FIELD_PARTICIPANT_NAME);
        $contactEmail = $this->stringAnswer($answers, FormField::FIELD_PARTICIPANT_EMAIL);

        $submission = $this->entityManager->wrapInTransaction(
            function () use ($formId, $answers, $contactEmail): FormSubmission {
                // "Risks & Mitigations — Capacity Overselling": the form row
                // is locked first, so a concurrent submission serializes
                // against the same count this transaction is about to read.
                $lockedForm = $this->forms->lockForUpdate($formId) ?? throw new \LogicException('Form no longer exists.');

                if (null !== $this->submissions->findOneByFormAndEmail($lockedForm, $contactEmail)) {
                    throw DuplicateSubmissionException::forEmail($contactEmail);
                }

                if ($lockedForm->isFull($this->submissions->countConfirmedForForm($lockedForm))) {
                    throw CampFullException::forForm($formId);
                }

                $status = $lockedForm->isFree() ? FormSubmission::STATUS_FREE : FormSubmission::STATUS_PENDING;
                $submission = new FormSubmission($lockedForm->getTrainer(), $lockedForm, $answers, $contactEmail, $status);
                $this->submissions->add($submission);
                $this->entityManager->flush();

                if ($lockedForm->isFree()) {
                    // AC-08-16: immediate confirmation — no external network
                    // call involved, so this stays inside the transaction
                    // that already holds the form lock, matching
                    // PlayerRegistrationService's own same-request mail
                    // precedent.
                    $this->mailer->sendConfirmation($submission);
                }

                return $submission;
            },
        );

        if (FormSubmission::STATUS_FREE === $submission->getPaymentStatus()) {
            return SubmissionOutcome::confirmed($submission);
        }

        return $this->startPayment($submission, $contactName, $contactPhone);
    }

    /**
     * BR-08-8: for a paid camp/evaluation, payment must complete before
     * registration is confirmed — this only STARTS the Checkout Session;
     * confirmation happens exclusively on the async webhook
     * (`App\Forms\EventSubscriber\PaymentOutcomeSubscriber`, BR-08-14).
     */
    private function startPayment(FormSubmission $submission, string $contactName, ?string $contactPhone): SubmissionOutcome
    {
        $form = $submission->getForm();
        $priceMinorUnits = $form->getPriceMinorUnits();
        \assert(null !== $priceMinorUnits);

        $slug = $form->getShareableSlug();
        $token = $this->tokens->tokenFor($submission);

        $result = $this->paymentGateway->requestPayment(new PaymentIntentRequest(
            $form->getTrainer(),
            $submission,
            $contactName,
            $submission->getContactEmail(),
            $contactPhone,
            $priceMinorUnits,
            $this->urlGenerator->generate('forms_public_checkout_success', ['code' => $slug, 'submission' => $token], UrlGeneratorInterface::ABSOLUTE_URL),
            $this->urlGenerator->generate('forms_public_checkout_cancel', ['code' => $slug, 'submission' => $token], UrlGeneratorInterface::ABSOLUTE_URL),
        ));

        if (PaymentIntentOutcome::Pending !== $result->outcome || null === $result->redirectUrl || null === $result->paymentRecordId) {
            throw new \RuntimeException('Unable to start payment for this registration.');
        }

        $this->attachPaymentRecord($submission, $result->paymentRecordId);

        return SubmissionOutcome::pendingPayment($submission, $result->redirectUrl);
    }

    /**
     * A lazy reference, not a query — `FormSubmission` only points AT
     * `PaymentRecord`, it never reads or mutates one of its fields, so this
     * stays within "Forms... must not write a payment record itself"
     * (module map) exactly as `Rsvp::attachPaymentRecord()`'s own call site
     * (`RsvpService`) already establishes the precedent for.
     */
    private function attachPaymentRecord(FormSubmission $submission, int $paymentRecordId): void
    {
        /** @var PaymentRecord $reference */
        $reference = $this->entityManager->getReference(PaymentRecord::class, $paymentRecordId);
        $submission->attachPaymentRecord($reference);
        $this->entityManager->flush();
    }

    /**
     * @param array<string, mixed> $answers
     */
    private function stringAnswer(array $answers, string $fieldId): string
    {
        $value = $answers[$fieldId] ?? null;

        if (!\is_string($value) || '' === trim($value)) {
            throw new \InvalidArgumentException(sprintf('Missing required answer for "%s".', $fieldId));
        }

        return $value;
    }
}
