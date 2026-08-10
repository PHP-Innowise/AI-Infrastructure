<?php

declare(strict_types=1);

namespace App\Identity\Entity;

use App\Identity\Repository\AvailabilityWindowRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * "Best Times" — one row per available-or-not time range on one weekday, for
 * either a coach or a player.
 *
 * Trainer-scoped, not global: architecture Decisions "Player availability"
 * chooses isolation over convenience — a player's Best Times under Trainer A
 * say nothing about their availability for Trainer B.
 *
 * @see specs/database-designer-schema.md "`availability_window`"
 * @see specs/requirements-analyst-epic-01-user-management-spec.md BR-01-25, BR-01-26, AC-01-43..47
 */
#[ORM\Entity(repositoryClass: AvailabilityWindowRepository::class)]
#[ORM\Table(name: 'availability_window')]
#[ORM\Index(name: 'idx_availability_coach_day', columns: ['coach_membership_id', 'day_of_week'])]
#[ORM\Index(name: 'idx_availability_player_day', columns: ['player_id', 'day_of_week'])]
#[TrainerScoped]
class AvailabilityWindow
{
    public const OWNER_COACH = 'coach';
    public const OWNER_PLAYER = 'player';

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\Column(name: 'owner_type', type: 'string', length: 16)]
    private string $ownerType;

    #[ORM\ManyToOne(targetEntity: CoachMembership::class)]
    #[ORM\JoinColumn(name: 'coach_membership_id', referencedColumnName: 'id', nullable: true, onDelete: 'CASCADE')]
    private ?CoachMembership $coachMembership = null;

    #[ORM\ManyToOne(targetEntity: PlayerProfile::class)]
    #[ORM\JoinColumn(name: 'player_id', referencedColumnName: 'id', nullable: true, onDelete: 'CASCADE')]
    private ?PlayerProfile $player = null;

    #[ORM\Column(name: 'day_of_week', type: 'smallint')]
    private int $dayOfWeek;

    #[ORM\Column(name: 'start_time', type: 'time_immutable')]
    private \DateTimeImmutable $startTime;

    #[ORM\Column(name: 'end_time', type: 'time_immutable')]
    private \DateTimeImmutable $endTime;

    #[ORM\Column(name: 'is_available', type: 'boolean', options: ['default' => true])]
    private bool $isAvailable = true;

    private function __construct(
        Trainer $trainer,
        string $ownerType,
        int $dayOfWeek,
        \DateTimeImmutable $startTime,
        \DateTimeImmutable $endTime,
        bool $isAvailable,
    ) {
        if ($dayOfWeek < 0 || $dayOfWeek > 6) {
            throw new \InvalidArgumentException('dayOfWeek must be between 0 (Sunday) and 6 (Saturday).');
        }

        if ($endTime <= $startTime) {
            throw new \InvalidArgumentException('endTime must be after startTime.');
        }

        $this->trainer = $trainer;
        $this->ownerType = $ownerType;
        $this->dayOfWeek = $dayOfWeek;
        $this->startTime = $startTime;
        $this->endTime = $endTime;
        $this->isAvailable = $isAvailable;
    }

    public static function forCoach(
        Trainer $trainer,
        CoachMembership $coachMembership,
        int $dayOfWeek,
        \DateTimeImmutable $startTime,
        \DateTimeImmutable $endTime,
        bool $isAvailable = true,
    ): self {
        $window = new self($trainer, self::OWNER_COACH, $dayOfWeek, $startTime, $endTime, $isAvailable);
        $window->coachMembership = $coachMembership;

        return $window;
    }

    public static function forPlayer(
        Trainer $trainer,
        PlayerProfile $player,
        int $dayOfWeek,
        \DateTimeImmutable $startTime,
        \DateTimeImmutable $endTime,
        bool $isAvailable = true,
    ): self {
        $window = new self($trainer, self::OWNER_PLAYER, $dayOfWeek, $startTime, $endTime, $isAvailable);
        $window->player = $player;

        return $window;
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getOwnerType(): string
    {
        return $this->ownerType;
    }

    public function getCoachMembership(): ?CoachMembership
    {
        return $this->coachMembership;
    }

    public function getPlayer(): ?PlayerProfile
    {
        return $this->player;
    }

    public function getDayOfWeek(): int
    {
        return $this->dayOfWeek;
    }

    public function getStartTime(): \DateTimeImmutable
    {
        return $this->startTime;
    }

    public function getEndTime(): \DateTimeImmutable
    {
        return $this->endTime;
    }

    public function isAvailable(): bool
    {
        return $this->isAvailable;
    }

    /**
     * Two ranges on the same weekday overlap. Used by
     * CoachAvailabilityConflictChecker (AC-01-47) — overlap itself is a
     * legitimate input (AC-01-46 explicitly allows multiple slots per day),
     * so this is a building block for the conflict warning, not a
     * self-validation rule.
     */
    public function overlaps(int $dayOfWeek, \DateTimeImmutable $start, \DateTimeImmutable $end): bool
    {
        if ($dayOfWeek !== $this->dayOfWeek) {
            return false;
        }

        return $start < $this->endTime && $end > $this->startTime;
    }
}
