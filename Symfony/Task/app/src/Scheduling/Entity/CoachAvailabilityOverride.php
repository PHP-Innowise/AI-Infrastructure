<?php

declare(strict_types=1);

namespace App\Scheduling\Entity;

use App\Identity\Entity\Account;
use App\Identity\Entity\CoachMembership;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use App\Scheduling\Repository\CoachAvailabilityOverrideRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * The audit trail for AC-02-9/BR-02-13/15: a trainer assigning a coach to a
 * time that conflicts with the coach's stated "My Times" availability, or to
 * an overlapping event, after supplying a required reason.
 *
 * Kept independent of `CoachAssignment` on purpose — a re-assignment does
 * not erase the audited override (architect-architecture.md "Entity
 * population — Trainer-scoped"). No FK to `CoachAssignment`: referencing
 * `(event, coach)` directly keeps the audited fact intact even if the
 * assignment is later reassigned to a different coach — see the schema's
 * own note under "`coach_availability_override`". Create-only; nothing ever
 * updates a row here.
 *
 * @see specs/database-designer-schema.md "`coach_availability_override`"
 * @see specs/requirements-analyst-epic-01-user-management-spec.md BR-01-26, AC-01-47
 * @see specs/requirements-analyst-epic-02-event-management-spec.md AC-02-9, BR-02-13, BR-02-15
 */
#[ORM\Entity(repositoryClass: CoachAvailabilityOverrideRepository::class)]
#[ORM\Table(name: 'coach_availability_override')]
#[ORM\Index(name: 'idx_coach_override_event_coach', columns: ['event_id', 'coach_membership_id'])]
#[TrainerScoped]
class CoachAvailabilityOverride
{
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

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'overridden_by_account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $overriddenByAccount;

    #[ORM\Column(type: 'text')]
    private string $reason;

    #[ORM\Column(name: 'created_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    public function __construct(
        Trainer $trainer,
        Event $event,
        CoachMembership $coachMembership,
        Account $overriddenByAccount,
        string $reason,
    ) {
        if ('' === trim($reason)) {
            throw new \InvalidArgumentException('An availability-conflict override requires a reason.');
        }

        $this->trainer = $trainer;
        $this->event = $event;
        $this->coachMembership = $coachMembership;
        $this->overriddenByAccount = $overriddenByAccount;
        $this->reason = $reason;
        $this->createdAt = new \DateTimeImmutable();
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

    public function getOverriddenByAccount(): Account
    {
        return $this->overriddenByAccount;
    }

    public function getReason(): string
    {
        return $this->reason;
    }

    public function getCreatedAt(): \DateTimeImmutable
    {
        return $this->createdAt;
    }
}
