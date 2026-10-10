<?php

declare(strict_types=1);

namespace App\Forms\Service;

use App\Billing\Entity\PaymentRecord;
use App\Forms\Dto\FormField;
use App\Forms\Entity\Form;
use App\Forms\Entity\FormSubmission;
use App\Forms\Exception\FormHasSubmissionsException;
use App\Forms\Repository\FormRepository;
use App\Forms\Repository\FormSubmissionRepository;
use App\Platform\Entity\PublicTenantCode;
use App\Platform\Entity\Trainer;
use App\Platform\Repository\PublicTenantCodeRepository;
use App\Platform\Service\PublicTenantCodeRegistry;
use Doctrine\ORM\EntityManagerInterface;

/**
 * US-08.01/08.02/08.06: create, edit, publish, toggle and delete a camp or
 * evaluation form. Owns the one invariant that must never drift, matching
 * `ShareLinkService`'s own precedent exactly: every `Form` this publishes
 * gets a matching `PublicTenantCode` row, or `/forms/{code}` 404s for a code
 * that genuinely resolves to a form.
 *
 * @see specs/database-designer-schema.md "`form`"
 * @see specs/requirements-analyst-epic-08-forms-registration-spec.md US-08.01, US-08.02, US-08.06
 */
final readonly class FormService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private FormRepository $forms,
        private FormSubmissionRepository $submissions,
        private PublicTenantCodeRegistry $publicTenantCodes,
        private PublicTenantCodeRepository $publicTenantCodeRepository,
    ) {
    }

    /**
     * BR-08-1: pre-loaded template — the two reserved fields every form
     * needs (`FormField`'s own docblock) plus one illustrative extra field,
     * matching the epic's own example ("name, email, age, emergency
     * contact").
     *
     * @return list<FormField>
     */
    public static function defaultTemplateFields(): array
    {
        return [
            new FormField(FormField::FIELD_PARTICIPANT_NAME, FormField::TYPE_TEXT, 'Participant Name', true),
            new FormField(FormField::FIELD_PARTICIPANT_EMAIL, FormField::TYPE_EMAIL, 'Participant Email', true),
            new FormField('emergency_contact', FormField::TYPE_TEXT, 'Emergency Contact', false),
        ];
    }

    /**
     * AC-08-1..5: name, display-only dates folded into $description by the
     * caller (the epic states dates are "not linked to the calendar" —
     * display-only text, not a structured column — Data requirements names
     * no dedicated date field), capacity 1-1000, optional $0/$1-10,000
     * price.
     *
     * @param list<FormField> $fields
     */
    public function createCamp(
        Trainer $trainer,
        string $name,
        ?string $description,
        ?int $priceMinorUnits,
        int $capacityLimit,
        array $fields,
    ): Form {
        return $this->create($trainer, Form::TYPE_CAMP, $name, $description, $priceMinorUnits, $capacityLimit, $fields);
    }

    /**
     * AC-08-9/10: no capacity limit for an evaluation (BR-08-3).
     *
     * @param list<FormField> $fields
     */
    public function createEvaluation(
        Trainer $trainer,
        string $name,
        ?string $description,
        ?int $priceMinorUnits,
        array $fields,
    ): Form {
        return $this->create($trainer, Form::TYPE_EVALUATION, $name, $description, $priceMinorUnits, null, $fields);
    }

    /**
     * AC-08-28: editing fields/description/pricing. AC-08-30: a camp's
     * capacity may never drop below its current (confirmed) registration
     * count — checked here, against a repository count, because `Form`
     * itself has no access to its own submissions.
     *
     * @param list<FormField> $fields
     */
    public function update(Form $form, string $name, ?string $description, ?int $priceMinorUnits, ?int $capacityLimit, array $fields): void
    {
        if ($form->isCamp() && null !== $capacityLimit) {
            $confirmedCount = $this->submissions->countConfirmedForForm($form);

            if ($capacityLimit < $confirmedCount) {
                throw new \InvalidArgumentException(sprintf(
                    'Capacity cannot be reduced below the current registration count (%d).',
                    $confirmedCount,
                ));
            }
        }

        $form->updateDetails($name, $description, $priceMinorUnits, $capacityLimit, $fields);
        $this->entityManager->flush();
    }

    /**
     * AC-08-7: publishing produces the shareable link — see `Form`'s own
     * docblock for why the slug already exists and this only issues the
     * `PublicTenantCode` mapping that makes it resolvable. Idempotent
     * ("generates the shareable link on FIRST publish" — a later publish is
     * a no-op), checked via the registry's own lookup rather than a stored
     * "published" flag, since none exists on this schema.
     */
    public function publish(Form $form): void
    {
        $this->entityManager->wrapInTransaction(function () use ($form): void {
            \assert(null !== $form->getId());

            if ($this->isPublished($form)) {
                return;
            }

            $this->publicTenantCodes->issue($form->getShareableSlug(), $form->getTrainer(), PublicTenantCode::KIND_FORM, $form->getId());
        });
    }

    public function isPublished(Form $form): bool
    {
        $formId = $form->getId();
        \assert(null !== $formId);

        return null !== $this->publicTenantCodeRepository->findOneByReference(PublicTenantCode::KIND_FORM, $formId);
    }

    /**
     * AC-08-8/29: camps only — `Form::enable()`/`disable()` already refuse
     * an evaluation; this layer adds nothing beyond the flush.
     */
    public function enable(Form $form): void
    {
        $form->enable();
        $this->entityManager->flush();
    }

    public function disable(Form $form): void
    {
        $form->disable();
        $this->entityManager->flush();
    }

    /**
     * AC-08-31: blocked while ANY submission exists — see
     * `FormHasSubmissionsException`'s own docblock for why this is wider
     * than the AC's literal "paid registrations" wording (a schema-forced
     * `RESTRICT` FK, not a choice). Revokes the `PublicTenantCode` mapping
     * alongside the row itself, matching `ShareLinkService::revoke()`'s own
     * hygiene (schema doc: harmless either way, since a stale mapping
     * "grants a tenant and nothing else").
     *
     * @throws FormHasSubmissionsException
     */
    public function delete(Form $form): void
    {
        $allSubmissions = $this->submissions->findAllForForm($form);
        $paidUnrefundedCount = 0;

        foreach ($allSubmissions as $submission) {
            $paymentRecord = $submission->getPaymentRecord();

            if (FormSubmission::STATUS_PAID === $submission->getPaymentStatus()
                && null !== $paymentRecord
                && PaymentRecord::STATUS_REFUNDED !== $paymentRecord->getStatus()
            ) {
                ++$paidUnrefundedCount;
            }
        }

        if ([] !== $allSubmissions) {
            throw FormHasSubmissionsException::withPaidCount(\count($allSubmissions), $paidUnrefundedCount);
        }

        $this->entityManager->wrapInTransaction(function () use ($form): void {
            $formId = $form->getId();
            \assert(null !== $formId);

            $this->publicTenantCodes->revoke(PublicTenantCode::KIND_FORM, $formId);
            $this->forms->remove($form);
        });
    }

    /**
     * AC-08-32: current submission count and remaining spots.
     */
    public function submissionCountFor(Form $form): int
    {
        return $this->submissions->countConfirmedForForm($form);
    }

    public function remainingSpotsFor(Form $form): ?int
    {
        return $form->remainingSpots($this->submissionCountFor($form));
    }

    /**
     * High-entropy, URL-safe, 12 characters — identical construction to
     * `ShareLinkService::generateCode()`.
     */
    private function generateSlug(): string
    {
        return rtrim(strtr(base64_encode(random_bytes(9)), '+/', '-_'), '=');
    }

    /**
     * @param list<FormField> $fields
     */
    private function create(Trainer $trainer, string $formType, string $name, ?string $description, ?int $priceMinorUnits, ?int $capacityLimit, array $fields): Form
    {
        $form = new Form($trainer, $formType, $name, $description, $priceMinorUnits, $capacityLimit, $fields, $this->generateSlug());
        $this->forms->add($form);
        $this->entityManager->flush();

        return $form;
    }
}
