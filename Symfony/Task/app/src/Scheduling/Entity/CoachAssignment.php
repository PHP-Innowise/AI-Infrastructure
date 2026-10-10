<?php

declare(strict_types=1);

namespace App\Scheduling\Entity;

use App\Identity\Entity\CoachMembership;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use App\Scheduling\Repository\CoachAssignmentRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * A coach's assignment to one event. BR-02-13: "Pending" until the coach
 * confirms — Q-02.02's resolved default is always explicit, no time-based
 * auto-confirm (see CoachAssignmentService's own docblock).
 *
 * Unique per (event, coach): re-assigning the SAME coach after they declined
 * reuses this row (CoachAssignmentService::assign()) rather than violating
 * the unique index with a second insert. Reassigning to a DIFFERENT coach
 * while a row is still pending/confirmed marks the superseded row Declined
 * with a system-authored reason — a deliberate, documented reuse of the
 * existing three-value status vocabulary (the settled schema names no
 * fourth "superseded" state) so that coach correctly stops seeing the event
 * in "Events to Confirm"; distinguishable from a genuine decline by its
 * reason text. See CoachAssignmentService::assign() for where this happens.
 *
 * @see specs/database-designer-schema.md "`coach_assignment`"
 * @see specs/requirements-analyst-epic-02-event-management-spec.md BR-02-13..15, AC-02-8..11, AC-02-34..37
 */
#[ORM\Entity(repositoryClass: CoachAssignmentRepository::class)]
#[ORM\Table(name: 'coach_assignment')]
#[ORM\UniqueConstraint(name: 'uniq_coach_assignment_event_coach', columns: ['event_id', 'coach_membership_id'])]
#[ORM\Index(name: 'idx_coach_assignment_coach_status', columns: ['coach_membership_id', 'status'])]
#[TrainerScoped]
class CoachAssignment
{
    public const STATUS_PENDING = 'pending';
    public const STATUS_CONFIRMED = 'confirmed';
    public const STATUS_DECLINED = 'declined';

    /**
     * AC-02-36/BR-02-13: the system-authored reason used when a different
     * coach supersedes this row — see the class docblock.
     */
    public const REASON_REASSIGNED = 'Reassigned to a different coach by the trainer.';

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

    #[ORM\ManyToOne(targetEntity: CoachMembership::class)]
    #[ORM\JoinColumn(name: 'coach_membership_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private CoachMembership $coachMembership;

    #[ORM\Column(type: 'string', length: 16, options: ['default' => self::STATUS_PENDING])]
    private string $status = self::STATUS_PENDING;

    #[ORM\Column(name: 'decline_reason', type: 'text', nullable: true)]
    private ?string $declineReason = null;

    #[ORM\Column(name: 'confirmed_at', type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $confirmedAt = null;

    #[ORM\Column(name: 'assigned_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $assignedAt;

    public function __construct(
        Trainer $trainer,
        Event $event,
        CoachMembership $coachMembership,
        \DateTimeImmutable $assignedAt,
    ) {
        $this->trainer = $trainer;
        $this->event = $event;
        $this->coachMembership = $coachMembership;
        $this->assignedAt = $assignedAt;
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

    public function getCoachMembership(): CoachMembership
    {
        return $this->coachMembership;
    }

    public function getStatus(): string
    {
        return $this->status;
    }

    public function isPending(): bool
    {
        return self::STATUS_PENDING === $this->status;
    }

    public function isConfirmed(): bool
    {
        return self::STATUS_CONFIRMED === $this->status;
    }

    public function isDeclined(): bool
    {
        return self::STATUS_DECLINED === $this->status;
    }

    public function getDeclineReason(): ?string
    {
        return $this->declineReason;
    }

    public function getConfirmedAt(): ?\DateTimeImmutable
    {
        return $this->confirmedAt;
    }

    public function getAssignedAt(): \DateTimeImmutable
    {
        return $this->assignedAt;
    }

    /**
     * AC-02-35: moves to "Assigned Sessions", status Confirmed.
     */
    public function confirm(\DateTimeImmutable $now): void
    {
        $this->status = self::STATUS_CONFIRMED;
        $this->confirmedAt = $now;
        $this->declineReason = null;
    }

    /**
     * AC-02-36: optional reason.
     */
    public function decline(?string $reason): void
    {
        $this->status = self::STATUS_DECLINED;
        $this->declineReason = $reason;
        $this->confirmedAt = null;
    }

    /**
     * Resets a previously-declined (or superseded) row back to Pending for
     * a fresh assignment cycle — reused rather than duplicated, since
     * (event, coach) is unique.
     */
    public function reassign(\DateTimeImmutable $assignedAt): void
    {
        $this->status = self::STATUS_PENDING;
        $this->declineReason = null;
        $this->confirmedAt = null;
        $this->assignedAt = $assignedAt;
    }
}
