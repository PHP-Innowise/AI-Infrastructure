<?php

declare(strict_types=1);

namespace App\Scheduling\Entity;

use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use App\Scheduling\Repository\EventRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * A training event a trainer creates, players/parents discover and RSVP to,
 * and a coach is assigned to.
 *
 * No coach column lives here by design — "assigned coach(es)" is entirely
 * represented by `CoachAssignment` rows referencing this event, matching the
 * settled schema exactly (specs/database-designer-schema.md "`event`" lists
 * no coach FK).
 *
 * "Past" is defined uniformly as `startsAt <= now` — the same cutoff
 * AC-02-49 states explicitly for cancellation ("cannot be canceled after it
 * has already started") is reused here for the edit guard (AC-02-54) and for
 * the derived "Completed" display status (AC-02-49's "it is marked
 * 'Completed' instead"), since the epic never states two different cutoffs
 * for the two guards. `status` itself stays `active` in the database once an
 * event's time passes — nothing ever writes `completed` — the same
 * "time-based state is derived at read time" pattern
 * `ChildApprovalRequest::displayStatus()` uses (architect-architecture.md
 * "Synchronous and asynchronous work").
 *
 * @see specs/database-designer-schema.md "`event`"
 * @see specs/requirements-analyst-epic-02-event-management-spec.md BR-02-1..6, AC-02-1..3, AC-02-49, AC-02-50..54
 */
#[ORM\Entity(repositoryClass: EventRepository::class)]
#[ORM\Table(name: 'event')]
#[ORM\Index(name: 'idx_event_trainer_starts', columns: ['trainer_id', 'starts_at'])]
#[ORM\Index(name: 'idx_event_trainer_status_starts', columns: ['trainer_id', 'status', 'starts_at'])]
#[ORM\Index(name: 'idx_event_trainer_type', columns: ['trainer_id', 'event_type'])]
#[TrainerScoped]
class Event
{
    public const TYPE_TRAINING_SESSION = 'training_session';
    public const TYPE_PRIVATE_SESSION = 'private_session';
    public const TYPE_SMALL_GROUP = 'small_group';

    public const VISIBILITY_PUBLIC = 'public';
    public const VISIBILITY_PRIVATE = 'private';

    public const STATUS_ACTIVE = 'active';
    public const STATUS_CANCELED = 'canceled';
    public const STATUS_COMPLETED = 'completed';

    public const PAYMENT_FREE = 'free';
    public const PAYMENT_USD = 'usd';
    public const PAYMENT_TOKEN = 'token';

    /**
     * BR-02-2: a hard maximum. Enforced at the application layer (no CHECK
     * constraint expresses a duration derived from two other columns
     * portably) — see the constructor guard.
     */
    private const MAX_DURATION_HOURS = 24;

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\Column(type: 'string', length: 100)]
    private string $title;

    #[ORM\Column(type: 'text', nullable: true)]
    private ?string $description = null;

    #[ORM\Column(name: 'event_type', type: 'string', length: 24)]
    private string $eventType;

    #[ORM\Column(name: 'starts_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $startsAt;

    #[ORM\Column(name: 'ends_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $endsAt;

    #[ORM\Column(type: 'string', length: 255)]
    private string $location;

    #[ORM\Column(type: 'smallint')]
    private int $capacity;

    #[ORM\Column(type: 'string', length: 16, options: ['default' => self::VISIBILITY_PUBLIC])]
    private string $visibility = self::VISIBILITY_PUBLIC;

    #[ORM\Column(name: 'usd_pricing_enabled', type: 'boolean', options: ['default' => false])]
    private bool $usdPricingEnabled = false;

    #[ORM\Column(name: 'usd_price_minor_units', type: 'integer', options: ['default' => 0])]
    private int $usdPriceMinorUnits = 0;

    /**
     * AC-02-58: token pricing defaults ON, at 1 token (Q-02.x default,
     * distinct from usdPricingEnabled's default OFF).
     */
    #[ORM\Column(name: 'token_pricing_enabled', type: 'boolean', options: ['default' => true])]
    private bool $tokenPricingEnabled = true;

    #[ORM\Column(name: 'token_price', type: 'integer', options: ['default' => 1])]
    private int $tokenPrice = 1;

    #[ORM\Column(name: 'min_age', type: 'smallint', nullable: true)]
    private ?int $minAge = null;

    #[ORM\Column(name: 'max_age', type: 'smallint', nullable: true)]
    private ?int $maxAge = null;

    /**
     * @var list<string>|null
     */
    #[ORM\Column(name: 'skill_levels', type: 'text_array', nullable: true)]
    private ?array $skillLevels = null;

    /**
     * @var list<string>|null
     */
    #[ORM\Column(type: 'text_array', nullable: true)]
    private ?array $genders = null;

    #[ORM\Column(type: 'string', length: 16, options: ['default' => self::STATUS_ACTIVE])]
    private string $status = self::STATUS_ACTIVE;

    #[ORM\Column(name: 'canceled_reason', type: 'text', nullable: true)]
    private ?string $canceledReason = null;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'canceled_by_account_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?Account $canceledByAccount = null;

    #[ORM\Column(name: 'canceled_at', type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $canceledAt = null;

    #[ORM\Column(type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    #[ORM\Column(type: 'datetimetz_immutable')]
    private \DateTimeImmutable $updatedAt;

    /**
     * @param list<string>|null $skillLevels
     * @param list<string>|null $genders
     */
    public function __construct(
        Trainer $trainer,
        string $title,
        string $eventType,
        \DateTimeImmutable $startsAt,
        \DateTimeImmutable $endsAt,
        string $location,
        int $capacity,
        string $visibility = self::VISIBILITY_PUBLIC,
        ?string $description = null,
        ?int $minAge = null,
        ?int $maxAge = null,
        ?array $skillLevels = null,
        ?array $genders = null,
    ) {
        $this->trainer = $trainer;
        $this->guardTitle($title);
        $this->guardEventType($eventType);
        $this->guardDates($startsAt, $endsAt);
        $this->guardCapacity($capacity);
        $this->guardVisibility($visibility);
        $this->guardLocation($location);

        $this->title = $title;
        $this->eventType = $eventType;
        $this->startsAt = $startsAt;
        $this->endsAt = $endsAt;
        $this->location = $location;
        $this->capacity = $capacity;
        $this->visibility = $visibility;
        $this->description = $description;
        $this->minAge = $minAge;
        $this->maxAge = $maxAge;
        $this->skillLevels = $skillLevels;
        $this->genders = $genders;
        $this->createdAt = new \DateTimeImmutable();
        $this->updatedAt = $this->createdAt;
    }

    /**
     * @return list<string>
     */
    public static function types(): array
    {
        return [self::TYPE_TRAINING_SESSION, self::TYPE_PRIVATE_SESSION, self::TYPE_SMALL_GROUP];
    }

    /**
     * @return list<string>
     */
    public static function visibilities(): array
    {
        return [self::VISIBILITY_PUBLIC, self::VISIBILITY_PRIVATE];
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getTitle(): string
    {
        return $this->title;
    }

    public function getDescription(): ?string
    {
        return $this->description;
    }

    public function getEventType(): string
    {
        return $this->eventType;
    }

    public function getStartsAt(): \DateTimeImmutable
    {
        return $this->startsAt;
    }

    public function getEndsAt(): \DateTimeImmutable
    {
        return $this->endsAt;
    }

    public function getLocation(): string
    {
        return $this->location;
    }

    public function getCapacity(): int
    {
        return $this->capacity;
    }

    public function getVisibility(): string
    {
        return $this->visibility;
    }

    public function isPrivate(): bool
    {
        return self::VISIBILITY_PRIVATE === $this->visibility;
    }

    public function isUsdPricingEnabled(): bool
    {
        return $this->usdPricingEnabled;
    }

    public function getUsdPriceMinorUnits(): int
    {
        return $this->usdPriceMinorUnits;
    }

    public function isTokenPricingEnabled(): bool
    {
        return $this->tokenPricingEnabled;
    }

    public function getTokenPrice(): int
    {
        return $this->tokenPrice;
    }

    /**
     * BR-02-7/8: a Free event is one with neither pricing option enabled.
     */
    public function isFree(): bool
    {
        return !$this->usdPricingEnabled && !$this->tokenPricingEnabled;
    }

    public function getMinAge(): ?int
    {
        return $this->minAge;
    }

    public function getMaxAge(): ?int
    {
        return $this->maxAge;
    }

    /**
     * @return list<string>|null
     */
    public function getSkillLevels(): ?array
    {
        return $this->skillLevels;
    }

    /**
     * @return list<string>|null
     */
    public function getGenders(): ?array
    {
        return $this->genders;
    }

    public function getStatus(): string
    {
        return $this->status;
    }

    public function getCanceledReason(): ?string
    {
        return $this->canceledReason;
    }

    public function getCanceledByAccount(): ?Account
    {
        return $this->canceledByAccount;
    }

    public function getCanceledAt(): ?\DateTimeImmutable
    {
        return $this->canceledAt;
    }

    public function getCreatedAt(): \DateTimeImmutable
    {
        return $this->createdAt;
    }

    public function getUpdatedAt(): \DateTimeImmutable
    {
        return $this->updatedAt;
    }

    public function isCanceled(): bool
    {
        return self::STATUS_CANCELED === $this->status;
    }

    /**
     * AC-02-49: the cutoff for "cannot be canceled/edited any more" and for
     * the derived "Completed" label alike — see the class docblock.
     */
    public function hasStarted(\DateTimeImmutable $now): bool
    {
        return $this->startsAt <= $now;
    }

    /**
     * AC-02-49/BR-02-13: read-time-derived, never written back.
     */
    public function displayStatus(\DateTimeImmutable $now): string
    {
        if (self::STATUS_ACTIVE === $this->status && $this->hasStarted($now)) {
            return self::STATUS_COMPLETED;
        }

        return $this->status;
    }

    /**
     * AC-02-54/US-02.14 "Validation": a canceled event can never be edited
     * (a new event must be created instead), and neither can one that has
     * already started or completed.
     */
    public function isEditable(\DateTimeImmutable $now): bool
    {
        return self::STATUS_ACTIVE === $this->status && !$this->hasStarted($now);
    }

    /**
     * AC-02-49: cannot cancel an already-started or completed event.
     */
    public function isCancelable(\DateTimeImmutable $now): bool
    {
        return self::STATUS_ACTIVE === $this->status && !$this->hasStarted($now);
    }

    /**
     * BR-02-12/AC-02-47: marks Canceled rather than deleting — AC-02-68's
     * "preserved as history."
     */
    public function cancel(string $reason, Account $canceledBy, \DateTimeImmutable $now): void
    {
        if ('' === trim($reason)) {
            throw new \InvalidArgumentException('A cancellation reason is required.');
        }

        $this->status = self::STATUS_CANCELED;
        $this->canceledReason = $reason;
        $this->canceledByAccount = $canceledBy;
        $this->canceledAt = $now;
        $this->touch();
    }

    /**
     * @param list<string>|null $skillLevels
     * @param list<string>|null $genders
     */
    public function update(
        string $title,
        string $eventType,
        \DateTimeImmutable $startsAt,
        \DateTimeImmutable $endsAt,
        string $location,
        int $capacity,
        string $visibility,
        ?string $description,
        ?int $minAge,
        ?int $maxAge,
        ?array $skillLevels,
        ?array $genders,
    ): void {
        $this->guardTitle($title);
        $this->guardEventType($eventType);
        $this->guardDates($startsAt, $endsAt);
        $this->guardCapacity($capacity);
        $this->guardVisibility($visibility);
        $this->guardLocation($location);

        $this->title = $title;
        $this->eventType = $eventType;
        $this->startsAt = $startsAt;
        $this->endsAt = $endsAt;
        $this->location = $location;
        $this->capacity = $capacity;
        $this->visibility = $visibility;
        $this->description = $description;
        $this->minAge = $minAge;
        $this->maxAge = $maxAge;
        $this->skillLevels = $skillLevels;
        $this->genders = $genders;
        $this->touch();
    }

    /**
     * AC-02-53: capacity may only be decreased down to the current
     * confirmed-RSVP count — the caller supplies that count (capacity checks
     * are never cached, see the schema's own Conventions), this method only
     * enforces the arithmetic.
     */
    public function canReduceCapacityTo(int $newCapacity, int $currentConfirmedRsvpCount): bool
    {
        return $newCapacity >= $currentConfirmedRsvpCount;
    }

    /**
     * BR-02-3/AC-02-58: the trainer can toggle either or both pricing
     * options on; a paid amount must be > 0 when its toggle is on.
     */
    public function setUsdPricing(bool $enabled, int $minorUnits): void
    {
        if ($enabled && $minorUnits <= 0) {
            throw new \InvalidArgumentException('A USD-priced event requires an amount greater than zero.');
        }

        $this->usdPricingEnabled = $enabled;
        $this->usdPriceMinorUnits = $enabled ? $minorUnits : 0;
        $this->touch();
    }

    /**
     * AC-02-59: token pricing is not fixed at 1:1 — trainer-configurable.
     */
    public function setTokenPricing(bool $enabled, int $tokenPrice): void
    {
        if ($tokenPrice < 1) {
            throw new \InvalidArgumentException('Token price must be at least 1.');
        }

        $this->tokenPricingEnabled = $enabled;
        $this->tokenPrice = $tokenPrice;
        $this->touch();
    }

    /**
     * AC-02-60: the payment methods a player may choose between at RSVP.
     *
     * @return list<string>
     */
    public function availablePaymentMethods(): array
    {
        $methods = [];

        if ($this->usdPricingEnabled) {
            $methods[] = self::PAYMENT_USD;
        }

        if ($this->tokenPricingEnabled) {
            $methods[] = self::PAYMENT_TOKEN;
        }

        if ([] === $methods) {
            $methods[] = self::PAYMENT_FREE;
        }

        return $methods;
    }

    /**
     * The amount due for a given, already-validated payment method — `usd`
     * in minor units, `token` in whole tokens, `free` always 0.
     */
    public function priceForMethod(string $paymentMethod): int
    {
        return match ($paymentMethod) {
            self::PAYMENT_USD => $this->usdPriceMinorUnits,
            self::PAYMENT_TOKEN => $this->tokenPrice,
            self::PAYMENT_FREE => 0,
            default => throw new \InvalidArgumentException(sprintf('Unknown payment method "%s".', $paymentMethod)),
        };
    }

    private function guardTitle(string $title): void
    {
        if ('' === trim($title)) {
            throw new \InvalidArgumentException('An event requires a non-empty title.');
        }

        if (mb_strlen($title) > 100) {
            throw new \InvalidArgumentException('An event title cannot exceed 100 characters.');
        }
    }

    private function guardLocation(string $location): void
    {
        if ('' === trim($location)) {
            throw new \InvalidArgumentException('An event requires a location.');
        }
    }

    private function guardEventType(string $eventType): void
    {
        if (!\in_array($eventType, self::types(), true)) {
            throw new \InvalidArgumentException(sprintf('Unknown event type "%s".', $eventType));
        }
    }

    private function guardVisibility(string $visibility): void
    {
        if (!\in_array($visibility, self::visibilities(), true)) {
            throw new \InvalidArgumentException(sprintf('Unknown event visibility "%s".', $visibility));
        }
    }

    /**
     * BR-02-2: end after start, hard cap of 24 hours. The "warning above 8
     * hours" half of BR-02-2 is advisory copy for the UI, not a refusal —
     * nothing to enforce here.
     */
    private function guardDates(\DateTimeImmutable $startsAt, \DateTimeImmutable $endsAt): void
    {
        if ($endsAt <= $startsAt) {
            throw new \InvalidArgumentException('An event\'s end time must be after its start time.');
        }

        $hours = ($endsAt->getTimestamp() - $startsAt->getTimestamp()) / 3600;

        if ($hours > self::MAX_DURATION_HOURS) {
            throw new \InvalidArgumentException('An event cannot last longer than 24 hours.');
        }
    }

    private function guardCapacity(int $capacity): void
    {
        if ($capacity < 1 || $capacity > 999) {
            throw new \InvalidArgumentException('Capacity must be between 1 and 999.');
        }
    }

    private function touch(): void
    {
        $this->updatedAt = new \DateTimeImmutable();
    }
}
