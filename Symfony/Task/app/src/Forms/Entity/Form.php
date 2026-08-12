<?php

declare(strict_types=1);

namespace App\Forms\Entity;

use App\Forms\Dto\FormField;
use App\Forms\Repository\FormRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * A camp or evaluation — one shared shape, BR-08-1..5. `formType`
 * distinguishes the two; capacity and the on/off toggle apply to camps only
 * (evaluations "simply ignore" them — schema doc's own words).
 *
 * **Publish is not row-creation.** `shareableSlug` is generated here, at
 * construction, because the column is `NOT NULL` — but the code only
 * *resolves a tenant* once `FormService::publish()` issues the matching
 * `PublicTenantCode` row (mirroring `ShareLinkService` exactly). Before that,
 * the slug exists as a string but `/forms/{slug}` 404s like any unknown
 * code — there is no separate "draft" column; publish state is entirely
 * whether a `PublicTenantCode` row exists for this form, never stored here.
 * AC-08-6/7/11: this is what makes "preview before publishing" true without
 * a form the trainer is still drafting already being live.
 *
 * @see specs/database-designer-schema.md "`form`"
 * @see specs/requirements-analyst-epic-08-forms-registration-spec.md BR-08-1..5, AC-08-1..12
 */
#[ORM\Entity(repositoryClass: FormRepository::class)]
#[ORM\Table(name: 'form')]
#[ORM\UniqueConstraint(name: 'uniq_form_shareable_slug', columns: ['shareable_slug'])]
#[ORM\Index(name: 'idx_form_trainer_type', columns: ['trainer_id', 'form_type'])]
#[TrainerScoped]
class Form
{
    public const TYPE_CAMP = 'camp';
    public const TYPE_EVALUATION = 'evaluation';

    /** @var list<string> */
    public const TYPES = [self::TYPE_CAMP, self::TYPE_EVALUATION];

    /**
     * Q-08.01 default: "20 fields max (prevents overly complex forms)".
     */
    public const MAX_FIELDS = 20;

    public const MIN_CAPACITY = 1;
    public const MAX_CAPACITY = 1000;

    /**
     * AC-08-5: "$0 (free) or in the $1-$10,000 range" — 100 minor units ($1)
     * through 1,000,000 ($10,000). Zero/null both mean free; nothing between
     * 1 and 99 minor units is a legal price.
     */
    public const MIN_PRICE_MINOR_UNITS = 100;
    public const MAX_PRICE_MINOR_UNITS = 1_000_000;

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\Column(name: 'form_type', type: 'string', length: 16)]
    private string $formType;

    #[ORM\Column(type: 'string', length: 255)]
    private string $name;

    #[ORM\Column(type: 'text', nullable: true)]
    private ?string $description = null;

    #[ORM\Column(name: 'price_minor_units', type: 'integer', nullable: true)]
    private ?int $priceMinorUnits = null;

    /**
     * Camps only (BR-08-3) — structurally NULL for an evaluation, not just
     * conventionally ignored, matching the schema's own CHECK.
     */
    #[ORM\Column(name: 'capacity_limit', type: 'integer', nullable: true)]
    private ?int $capacityLimit = null;

    /**
     * Camps only — evaluations ignore this column entirely
     * (`isOpenForRegistration()` never reads it for an evaluation).
     */
    #[ORM\Column(name: 'is_active', type: 'boolean', options: ['default' => true])]
    private bool $isActive = true;

    /**
     * @var list<array{id: string, type: string, label: string, required: bool, options: ?list<string>}>
     */
    #[ORM\Column(name: 'field_definitions', type: 'json')]
    private array $fieldDefinitions;

    #[ORM\Column(name: 'shareable_slug', type: 'string', length: 64)]
    private string $shareableSlug;

    #[ORM\Column(name: 'created_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    #[ORM\Column(name: 'updated_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $updatedAt;

    /**
     * @param list<FormField> $fields
     */
    public function __construct(
        Trainer $trainer,
        string $formType,
        string $name,
        ?string $description,
        ?int $priceMinorUnits,
        ?int $capacityLimit,
        array $fields,
        string $shareableSlug,
    ) {
        self::guardType($formType);

        if ('' === trim($shareableSlug)) {
            throw new \InvalidArgumentException('A form requires a shareable slug.');
        }

        $this->trainer = $trainer;
        $this->formType = $formType;
        $this->shareableSlug = $shareableSlug;
        $this->createdAt = new \DateTimeImmutable();
        $this->updatedAt = $this->createdAt;

        $this->applyDetails($name, $description, $priceMinorUnits, $capacityLimit, $fields);
    }

    public static function guardType(string $formType): void
    {
        if (!\in_array($formType, self::TYPES, true)) {
            throw new \InvalidArgumentException(sprintf('Unknown form type "%s".', $formType));
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

    public function getFormType(): string
    {
        return $this->formType;
    }

    public function isCamp(): bool
    {
        return self::TYPE_CAMP === $this->formType;
    }

    public function isEvaluation(): bool
    {
        return self::TYPE_EVALUATION === $this->formType;
    }

    public function getName(): string
    {
        return $this->name;
    }

    public function getDescription(): ?string
    {
        return $this->description;
    }

    public function getPriceMinorUnits(): ?int
    {
        return $this->priceMinorUnits;
    }

    public function isFree(): bool
    {
        return null === $this->priceMinorUnits || 0 === $this->priceMinorUnits;
    }

    public function getCapacityLimit(): ?int
    {
        return $this->capacityLimit;
    }

    public function isActiveFlag(): bool
    {
        return $this->isActive;
    }

    /**
     * BR-08-5: evaluations are always on and structurally ignore the flag —
     * this is the one method every caller (voter, public controller) should
     * read instead of `isActiveFlag()` directly.
     */
    public function isOpenForRegistration(): bool
    {
        return $this->isEvaluation() || $this->isActive;
    }

    /**
     * AC-08-8/29: camps only — a no-op guard rather than a silent success,
     * so a caller that reaches this for an evaluation finds out immediately
     * rather than believing a toggle happened.
     */
    public function enable(): void
    {
        if (!$this->isCamp()) {
            throw new \LogicException('Only a camp\'s registration can be enabled or disabled.');
        }

        $this->isActive = true;
        $this->touch();
    }

    public function disable(): void
    {
        if (!$this->isCamp()) {
            throw new \LogicException('Only a camp\'s registration can be enabled or disabled.');
        }

        $this->isActive = false;
        $this->touch();
    }

    public function getShareableSlug(): string
    {
        return $this->shareableSlug;
    }

    /**
     * @return list<FormField>
     */
    public function getFields(): array
    {
        return array_map(static fn (array $definition): FormField => FormField::fromArray($definition), $this->fieldDefinitions);
    }

    public function findField(string $fieldId): ?FormField
    {
        foreach ($this->getFields() as $field) {
            if ($field->id === $fieldId) {
                return $field;
            }
        }

        return null;
    }

    public function getCreatedAt(): \DateTimeImmutable
    {
        return $this->createdAt;
    }

    public function getUpdatedAt(): \DateTimeImmutable
    {
        return $this->updatedAt;
    }

    /**
     * AC-08-28: editing fields/description/pricing. Capacity-below-current-
     * registrations (AC-08-30) is NOT checked here — it needs a submission
     * count, which is a repository concern; `FormService::update()` checks
     * it before calling this.
     *
     * @param list<FormField> $fields
     */
    public function updateDetails(string $name, ?string $description, ?int $priceMinorUnits, ?int $capacityLimit, array $fields): void
    {
        $this->applyDetails($name, $description, $priceMinorUnits, $capacityLimit, $fields);
        $this->touch();
    }

    /**
     * AC-08-15/BR-08-7: capacity is enforced by counting CONFIRMED
     * (free|paid) submissions only — a pending payment never holds a spot.
     */
    public function isFull(int $confirmedSubmissionCount): bool
    {
        return null !== $this->capacityLimit && $confirmedSubmissionCount >= $this->capacityLimit;
    }

    /**
     * AC-08-32: null for an evaluation (no capacity concept at all).
     */
    public function remainingSpots(int $confirmedSubmissionCount): ?int
    {
        if (null === $this->capacityLimit) {
            return null;
        }

        return max(0, $this->capacityLimit - $confirmedSubmissionCount);
    }

    /**
     * @param list<FormField> $fields
     */
    private function applyDetails(string $name, ?string $description, ?int $priceMinorUnits, ?int $capacityLimit, array $fields): void
    {
        if ('' === trim($name)) {
            throw new \InvalidArgumentException('A form requires a non-empty name.');
        }

        self::guardPrice($priceMinorUnits);

        // BR-08-3: evaluations carry no capacity at all, not just an
        // ignored one — forced here so the invariant cannot be bypassed by
        // a caller that forgets to null it out.
        $effectiveCapacity = $this->isEvaluation() ? null : $capacityLimit;

        if ($this->isCamp()) {
            self::guardCapacity($effectiveCapacity);
        }

        self::guardFields($fields);

        $this->name = $name;
        $this->description = '' !== trim((string) $description) ? $description : null;
        $this->priceMinorUnits = (null === $priceMinorUnits || 0 === $priceMinorUnits) ? null : $priceMinorUnits;
        $this->capacityLimit = $effectiveCapacity;
        $this->fieldDefinitions = array_map(static fn (FormField $field): array => $field->toArray(), $fields);
    }

    private static function guardPrice(?int $priceMinorUnits): void
    {
        if (null === $priceMinorUnits || 0 === $priceMinorUnits) {
            return;
        }

        if ($priceMinorUnits < self::MIN_PRICE_MINOR_UNITS || $priceMinorUnits > self::MAX_PRICE_MINOR_UNITS) {
            throw new \InvalidArgumentException(sprintf(
                'Price must be free ($0) or between $%d and $%d.',
                intdiv(self::MIN_PRICE_MINOR_UNITS, 100),
                intdiv(self::MAX_PRICE_MINOR_UNITS, 100),
            ));
        }
    }

    private static function guardCapacity(?int $capacityLimit): void
    {
        if (null === $capacityLimit || $capacityLimit < self::MIN_CAPACITY || $capacityLimit > self::MAX_CAPACITY) {
            throw new \InvalidArgumentException(sprintf(
                'A camp requires a capacity limit between %d and %d.',
                self::MIN_CAPACITY,
                self::MAX_CAPACITY,
            ));
        }
    }

    /**
     * @param list<FormField> $fields
     */
    private static function guardFields(array $fields): void
    {
        if ([] === $fields) {
            throw new \InvalidArgumentException('A form requires at least one field.');
        }

        if (\count($fields) > self::MAX_FIELDS) {
            throw new \InvalidArgumentException(sprintf('A form may not carry more than %d fields.', self::MAX_FIELDS));
        }

        $ids = array_map(static fn (FormField $field): string => $field->id, $fields);

        if (\count($ids) !== \count(array_unique($ids))) {
            throw new \InvalidArgumentException('Form field ids must be unique.');
        }

        // AC-08-2: "minimum of 1 field required (participant name)" made
        // structural — see this class's own docblock and FormField's.
        // Type-checked too, not just presence: BR-08-9's "email field must
        // be a valid format" and `FormSubmission::$contactEmail`'s own
        // denormalization both depend on the reserved email field actually
        // being an email-typed field, not a trainer having relabeled it
        // into a plain text one.
        $byId = [];

        foreach ($fields as $field) {
            $byId[$field->id] = $field;
        }

        $nameField = $byId[FormField::FIELD_PARTICIPANT_NAME] ?? null;

        if (null === $nameField || FormField::TYPE_TEXT !== $nameField->type) {
            throw new \InvalidArgumentException('A form must include the participant name field as a Text field.');
        }

        $emailField = $byId[FormField::FIELD_PARTICIPANT_EMAIL] ?? null;

        if (null === $emailField || FormField::TYPE_EMAIL !== $emailField->type) {
            throw new \InvalidArgumentException('A form must include the participant email field as an Email field.');
        }
    }

    private function touch(): void
    {
        $this->updatedAt = new \DateTimeImmutable();
    }
}
