<?php

declare(strict_types=1);

namespace App\Scheduling\Entity;

use App\Identity\Entity\CoachMembership;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use App\Scheduling\Repository\AttendanceRecordRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * One player's attendance fact for one event. Holds the CURRENT status only;
 * `AttendanceEdit` is the append-only history of how it got there
 * (BR-02-18).
 *
 * `recordedByCoachMembership` is NOT NULL per the settled schema even though
 * BR-02-18 lets a trainer (not necessarily also a coach) record or override
 * an entry. Resolution: this column names *whose session* the attendance
 * belongs to (the event's assigned coach), a domain fact independent of who
 * clicked Save — attributing the literal actor for every write, trainer
 * overrides included, is `AttendanceEdit.editedByAccount`'s job, not this
 * column's. Recorded as an implementation decision in the coder's final
 * report, not silently assumed.
 *
 * @see specs/database-designer-schema.md "`attendance_record`"
 * @see specs/requirements-analyst-epic-02-event-management-spec.md BR-02-16..18, AC-02-38..42
 */
#[ORM\Entity(repositoryClass: AttendanceRecordRepository::class)]
#[ORM\Table(name: 'attendance_record')]
#[ORM\UniqueConstraint(name: 'uniq_attendance_record_event_player', columns: ['event_id', 'player_id'])]
#[ORM\Index(name: 'idx_attendance_record_trainer_player_status', columns: ['trainer_id', 'player_id', 'status', 'recorded_at'])]
#[TrainerScoped]
class AttendanceRecord
{
    public const STATUS_PRESENT = 'present';
    public const STATUS_ABSENT = 'absent';
    public const STATUS_LATE = 'late';
    public const STATUS_EXCUSED = 'excused';

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\ManyToOne(targetEntity: Event::class)]
    #[ORM\JoinColumn(name: 'event_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Event $event;

    #[ORM\ManyToOne(targetEntity: PlayerProfile::class)]
    #[ORM\JoinColumn(name: 'player_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private PlayerProfile $player;

    #[ORM\ManyToOne(targetEntity: Rsvp::class)]
    #[ORM\JoinColumn(name: 'rsvp_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Rsvp $rsvp;

    #[ORM\Column(type: 'string', length: 16)]
    private string $status;

    #[ORM\ManyToOne(targetEntity: CoachMembership::class)]
    #[ORM\JoinColumn(name: 'recorded_by_coach_membership_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private CoachMembership $recordedByCoachMembership;

    #[ORM\Column(name: 'recorded_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $recordedAt;

    public function __construct(
        Trainer $trainer,
        Event $event,
        PlayerProfile $player,
        Rsvp $rsvp,
        string $status,
        CoachMembership $recordedByCoachMembership,
        \DateTimeImmutable $recordedAt,
    ) {
        self::guardStatus($status);

        $this->trainer = $trainer;
        $this->event = $event;
        $this->player = $player;
        $this->rsvp = $rsvp;
        $this->status = $status;
        $this->recordedByCoachMembership = $recordedByCoachMembership;
        $this->recordedAt = $recordedAt;
    }

    /**
     * @return list<string>
     */
    public static function statuses(): array
    {
        return [self::STATUS_PRESENT, self::STATUS_ABSENT, self::STATUS_LATE, self::STATUS_EXCUSED];
    }

    public static function guardStatus(string $status): void
    {
        if (!\in_array($status, self::statuses(), true)) {
            throw new \InvalidArgumentException(sprintf('Unknown attendance status "%s".', $status));
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

    public function getEvent(): Event
    {
        return $this->event;
    }

    public function getPlayer(): PlayerProfile
    {
        return $this->player;
    }

    public function getRsvp(): Rsvp
    {
        return $this->rsvp;
    }

    public function getStatus(): string
    {
        return $this->status;
    }

    public function getRecordedByCoachMembership(): CoachMembership
    {
        return $this->recordedByCoachMembership;
    }

    public function getRecordedAt(): \DateTimeImmutable
    {
        return $this->recordedAt;
    }

    /**
     * BR-02-18: the caller (AttendanceService) is responsible for deciding
     * whether this change needs an AttendanceEdit row first — this only
     * applies the new value.
     */
    public function changeStatus(string $status): void
    {
        self::guardStatus($status);
        $this->status = $status;
    }
}
