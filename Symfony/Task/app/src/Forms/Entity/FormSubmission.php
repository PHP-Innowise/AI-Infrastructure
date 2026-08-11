<?php

declare(strict_types=1);

namespace App\Forms\Entity;

use App\Billing\Entity\PaymentRecord;
use App\Forms\Dto\FormField;
use App\Forms\Repository\FormSubmissionRepository;
use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * One registrant's answers to one `Form`. **The only public, unauthenticated
 * write in the platform** (schema doc, its own heading) — trainer-scoped
 * like every other CRM-adjacent row, but the trainer is resolved from
 * `form.shareableSlug` via `PublicTenantCode`, never from a session.
 *
 * **A3/A4, made concrete.** A registrant who never converts leaves
 * `convertedAccount` NULL forever; their `PaymentRecord.payerAccount` is
 * also NULL — this row is fully functional (visible in the trainer's
 * participant list, refundable via its payment record) with no `Account`
 * anywhere in the chain. **A5**: on conversion, `convertedAccount`/
 * `convertedAt` are set and the EXISTING `payment_record.payer_account_id`
 * is additively updated (a plain UPDATE) to point at the new account — see
 * `FormSubmissionConversionService`.
 *
 * **`attended` is a deliberate addition beyond the schema doc's own six-plus-
 * timestamps column list for this table.** AC-08-20 ("Trainer marks
 * attendance per participant via checkboxes") names a real, testable
 * requirement with nowhere on the settled schema to persist it — matching
 * the precedent Version20260811100000 already set for `stripe_event_receipt.
 * raw_payload`: an addition forced by a concrete requirement the schema
 * text omitted, not a silent, undocumented one. A plain boolean, not a
 * richer state machine — the epic says "checkboxes," not
 * present/absent/late/excused (contrast Scheduling's own, deliberately
 * richer `AttendanceRecord`).
 *
 * @see specs/database-designer-schema.md "`form_submission`"
 * @see specs/requirements-analyst-open-questions.md "A3", "A4", "A5"
 */
#[ORM\Entity(repositoryClass: FormSubmissionRepository::class)]
#[ORM\Table(name: 'form_submission')]
#[ORM\UniqueConstraint(name: 'uniq_form_submission_form_email', columns: ['form_id', 'contact_email'])]
#[ORM\Index(name: 'idx_form_submission_trainer_form_status', columns: ['trainer_id', 'form_id', 'payment_status'])]
#[ORM\Index(name: 'idx_form_submission_form_converted', columns: ['form_id', 'converted_account_id'])]
#[TrainerScoped]
class FormSubmission
{
    public const STATUS_FREE = 'free';
    public const STATUS_PAID = 'paid';
    public const STATUS_PENDING = 'pending';

    /** @var list<string> */
    public const STATUSES = [self::STATUS_FREE, self::STATUS_PAID, self::STATUS_PENDING];

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\ManyToOne(targetEntity: Form::class)]
    #[ORM\JoinColumn(name: 'form_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Form $form;

    /**
     * @var array<string, mixed> Field-id -> answer map, shape driven by
     *                           `form.fieldDefinitions` at submission time
     */
    #[ORM\Column(name: 'submission_data', type: 'json')]
    private array $submissionData;

    /**
     * Denormalized off `submissionData[FormField::FIELD_PARTICIPANT_EMAIL]`
     * — BR-08-10's uniqueness check and `payment_record.contact_email`
     * linkage both need a real column, not a JSON lookup.
     */
    #[ORM\Column(name: 'contact_email', type: 'string', length: 255, columnDefinition: 'CITEXT NOT NULL')]
    private string $contactEmail;

    #[ORM\Column(name: 'payment_status', type: 'string', length: 16, options: ['default' => self::STATUS_FREE])]
    private string $paymentStatus;

    /**
     * `ON DELETE NO ACTION`, not `RESTRICT` — `payment_record.
     * related_form_submission_id` points back at this table (the same
     * mutual-FK cycle `Rsvp::$paymentRecord`/`PaymentRecord::$relatedRsvp`
     * already document), so the migration DDL for both constraints is
     * `DEFERRABLE INITIALLY DEFERRED`. See `PaymentRecord::$relatedFormSubmission`'s
     * own docblock and this table's migration.
     */
    #[ORM\ManyToOne(targetEntity: PaymentRecord::class)]
    #[ORM\JoinColumn(name: 'payment_record_id', referencedColumnName: 'id', nullable: true, onDelete: 'NO ACTION')]
    private ?PaymentRecord $paymentRecord = null;

    #[ORM\Column(name: 'submitted_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $submittedAt;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'converted_account_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?Account $convertedAccount = null;

    #[ORM\Column(name: 'converted_at', type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $convertedAt = null;

    /**
     * AC-08-20. See this class's own docblock for why this column exists
     * beyond the schema doc's literal list.
     */
    #[ORM\Column(type: 'boolean', options: ['default' => false])]
    private bool $attended = false;

    /**
     * @param array<string, mixed> $submissionData
     */
    public function __construct(
        Trainer $trainer,
        Form $form,
        array $submissionData,
        string $contactEmail,
        string $paymentStatus,
    ) {
        self::guardStatus($paymentStatus);

        if ('' === trim($contactEmail)) {
            throw new \InvalidArgumentException('A form submission requires a contact email.');
        }

        $this->trainer = $trainer;
        $this->form = $form;
        $this->submissionData = $submissionData;
        $this->contactEmail = $contactEmail;
        $this->paymentStatus = $paymentStatus;
        $this->submittedAt = new \DateTimeImmutable();
    }

    public static function guardStatus(string $status): void
    {
        if (!\in_array($status, self::STATUSES, true)) {
            throw new \InvalidArgumentException(sprintf('Unknown payment status "%s".', $status));
        }
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getForm(): Form
    {
        return $this->form;
    }

    /**
     * @return array<string, mixed>
     */
    public function getSubmissionData(): array
    {
        return $this->submissionData;
    }

    public function answerFor(string $fieldId): mixed
    {
        return $this->submissionData[$fieldId] ?? null;
    }

    /**
     * AC-08-23: the submitter's name, pre-filling account conversion —
     * always answered, since `FormField::FIELD_PARTICIPANT_NAME` is a
     * reserved, non-removable field on every form (`Form::guardFields()`).
     */
    public function participantName(): string
    {
        $value = $this->answerFor(FormField::FIELD_PARTICIPANT_NAME);

        return \is_string($value) ? $value : '';
    }

    public function getContactEmail(): string
    {
        return $this->contactEmail;
    }

    public function getPaymentStatus(): string
    {
        return $this->paymentStatus;
    }

    /**
     * AC-08-15/18: only free|paid ever "counts" as a registration — pending
     * never holds a spot and is excluded from the participant list.
     */
    public function isConfirmed(): bool
    {
        return self::STATUS_FREE === $this->paymentStatus || self::STATUS_PAID === $this->paymentStatus;
    }

    public function getPaymentRecord(): ?PaymentRecord
    {
        return $this->paymentRecord;
    }

    public function attachPaymentRecord(PaymentRecord $paymentRecord): void
    {
        $this->paymentRecord = $paymentRecord;
    }

    /**
     * BR-08-14: the webhook confirms payment, which marks the registration
     * Paid.
     */
    public function markPaid(): void
    {
        $this->paymentStatus = self::STATUS_PAID;
    }

    public function getSubmittedAt(): \DateTimeImmutable
    {
        return $this->submittedAt;
    }

    public function getConvertedAccount(): ?Account
    {
        return $this->convertedAccount;
    }

    public function isConverted(): bool
    {
        return null !== $this->convertedAccount;
    }

    public function getConvertedAt(): ?\DateTimeImmutable
    {
        return $this->convertedAt;
    }

    /**
     * AC-08-24/A5: sets the conversion marker. Attaching the earlier
     * payment record to the new account is `PaymentRecord`'s own concern
     * (`FormSubmissionConversionService` calls both in one transaction).
     */
    public function markConverted(Account $account): void
    {
        if ($this->isConverted()) {
            throw new \LogicException('This submission has already been converted.');
        }

        $this->convertedAccount = $account;
        $this->convertedAt = new \DateTimeImmutable();
    }

    public function isAttended(): bool
    {
        return $this->attended;
    }

    /**
     * AC-08-20: a plain checkbox toggle — set directly to whatever the
     * trainer's form submitted, not a one-way "mark present" action.
     */
    public function setAttended(bool $attended): void
    {
        $this->attended = $attended;
    }
}
