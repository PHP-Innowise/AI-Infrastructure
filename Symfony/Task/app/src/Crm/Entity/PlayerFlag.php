<?php

declare(strict_types=1);

namespace App\Crm\Entity;

use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Crm\Repository\PlayerFlagRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * One of the platform's 8 fixed system-defined flags, applied to a player.
 * BR-03-6: exactly 8 flags, no custom flags — enforced by `guardType()` here
 * AND by the database CHECK constraint (the same closed-vocabulary pattern
 * `Event`/`Rsvp`/`CoachAssignment` already use throughout this codebase:
 * `VARCHAR` + class constants + a guard, not a native PHP enum column — see
 * the coder's final report for why that convention is followed here rather
 * than the schema doc's more aspirational "every enum-shaped column becomes
 * a PHP enum" language).
 *
 * BR-03-8: resolving hides a flag from the active view while preserving its
 * history — modeled as a status transition on the SAME row (never deleted),
 * with a partial unique index (`(player_id, flag_type) WHERE status =
 * 'active'`) enforcing "at most one active instance" while still permitting
 * reapplication after resolution (a fresh row, once this one is resolved).
 *
 * @see specs/database-designer-schema.md "`player_flag`"
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md BR-03-6/7/8, AC-03-15..18
 */
#[ORM\Entity(repositoryClass: PlayerFlagRepository::class)]
#[ORM\Table(name: 'player_flag')]
#[TrainerScoped]
class PlayerFlag
{
    public const TYPE_BEHAVIOR = 'behavior';
    public const TYPE_HIGH_NO_SHOW_RATE = 'high_no_show_rate';
    public const TYPE_INJURED = 'injured';
    public const TYPE_MEDICAL_RESTRICTION = 'medical_restriction';
    public const TYPE_SCHOLARSHIP = 'scholarship';
    public const TYPE_FINANCIAL_AID = 'financial_aid';
    public const TYPE_CONTACT_PRIORITY = 'contact_priority';
    public const TYPE_ATTENDANCE_RISK = 'attendance_risk';

    public const STATUS_ACTIVE = 'active';
    public const STATUS_RESOLVED = 'resolved';

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\ManyToOne(targetEntity: PlayerProfile::class)]
    #[ORM\JoinColumn(name: 'player_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private PlayerProfile $player;

    #[ORM\Column(name: 'flag_type', type: 'string', length: 24)]
    private string $flagType;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'applied_by_account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $appliedByAccount;

    #[ORM\Column(name: 'applied_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $appliedAt;

    #[ORM\Column(type: 'text', nullable: true)]
    private ?string $note = null;

    #[ORM\Column(type: 'string', length: 16, options: ['default' => self::STATUS_ACTIVE])]
    private string $status = self::STATUS_ACTIVE;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'resolved_by_account_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?Account $resolvedByAccount = null;

    #[ORM\Column(name: 'resolved_at', type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $resolvedAt = null;

    public function __construct(
        Trainer $trainer,
        PlayerProfile $player,
        string $flagType,
        Account $appliedByAccount,
        ?string $note = null,
    ) {
        self::guardType($flagType);

        $this->trainer = $trainer;
        $this->player = $player;
        $this->flagType = $flagType;
        $this->appliedByAccount = $appliedByAccount;
        $this->appliedAt = new \DateTimeImmutable();
        $this->note = $note;
    }

    /**
     * @return list<string>
     */
    public static function types(): array
    {
        return [
            self::TYPE_BEHAVIOR,
            self::TYPE_HIGH_NO_SHOW_RATE,
            self::TYPE_INJURED,
            self::TYPE_MEDICAL_RESTRICTION,
            self::TYPE_SCHOLARSHIP,
            self::TYPE_FINANCIAL_AID,
            self::TYPE_CONTACT_PRIORITY,
            self::TYPE_ATTENDANCE_RISK,
        ];
    }

    /**
     * US-03.04 "Acceptance Criteria - Apply Flag" (lines 227-235): the short
     * definition paired with each flag name, shown alongside the flag when a
     * trainer/coach picks one to apply.
     *
     * @return array<string, string>
     */
    public static function labelsWithDefinitions(): array
    {
        return [
            self::TYPE_BEHAVIOR => 'Behavior — behavioral issues',
            self::TYPE_HIGH_NO_SHOW_RATE => 'High no-show rate — attendance concern',
            self::TYPE_INJURED => 'Injured — medical, cannot participate',
            self::TYPE_MEDICAL_RESTRICTION => 'Medical restriction — can participate with restrictions',
            self::TYPE_SCHOLARSHIP => 'Scholarship — financial aid recipient',
            self::TYPE_FINANCIAL_AID => 'Financial aid — payment assistance',
            self::TYPE_CONTACT_PRIORITY => 'Contact priority — requires special attention',
            self::TYPE_ATTENDANCE_RISK => 'Attendance risk — at risk of dropping out',
        ];
    }

    public static function guardType(string $flagType): void
    {
        if (!\in_array($flagType, self::types(), true)) {
            throw new \InvalidArgumentException(sprintf('Unknown flag type "%s".', $flagType));
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

    public function getPlayer(): PlayerProfile
    {
        return $this->player;
    }

    public function getFlagType(): string
    {
        return $this->flagType;
    }

    public function getAppliedByAccount(): Account
    {
        return $this->appliedByAccount;
    }

    public function getAppliedAt(): \DateTimeImmutable
    {
        return $this->appliedAt;
    }

    public function getNote(): ?string
    {
        return $this->note;
    }

    public function getStatus(): string
    {
        return $this->status;
    }

    public function isActive(): bool
    {
        return self::STATUS_ACTIVE === $this->status;
    }

    public function getResolvedByAccount(): ?Account
    {
        return $this->resolvedByAccount;
    }

    public function getResolvedAt(): ?\DateTimeImmutable
    {
        return $this->resolvedAt;
    }

    /**
     * BR-03-8: hides from the active view; history (this row) is preserved.
     */
    public function resolve(Account $resolvedBy): void
    {
        if (!$this->isActive()) {
            throw new \LogicException('Only an active flag can be resolved.');
        }

        $this->status = self::STATUS_RESOLVED;
        $this->resolvedByAccount = $resolvedBy;
        $this->resolvedAt = new \DateTimeImmutable();
    }
}
