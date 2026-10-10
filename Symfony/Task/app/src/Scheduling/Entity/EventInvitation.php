<?php

declare(strict_types=1);

namespace App\Scheduling\Entity;

use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use App\Scheduling\Repository\EventInvitationRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * A private event's guest list. BR-02-6: individual player selection only —
 * group-based invitation is not available in MVP (AC-02-5).
 *
 * @see specs/database-designer-schema.md "`event_invitation`"
 * @see specs/requirements-analyst-epic-02-event-management-spec.md BR-02-6, AC-02-4..7
 */
#[ORM\Entity(repositoryClass: EventInvitationRepository::class)]
#[ORM\Table(name: 'event_invitation')]
#[ORM\UniqueConstraint(name: 'uniq_event_invitation_event_player', columns: ['event_id', 'player_id'])]
#[ORM\Index(name: 'idx_event_invitation_player', columns: ['player_id'])]
#[TrainerScoped]
class EventInvitation
{
    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\ManyToOne(targetEntity: Event::class)]
    #[ORM\JoinColumn(name: 'event_id', referencedColumnName: 'id', nullable: false, onDelete: 'CASCADE')]
    private Event $event;

    #[ORM\ManyToOne(targetEntity: PlayerProfile::class)]
    #[ORM\JoinColumn(name: 'player_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private PlayerProfile $player;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'invited_by_account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $invitedByAccount;

    #[ORM\Column(name: 'invited_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $invitedAt;

    public function __construct(Trainer $trainer, Event $event, PlayerProfile $player, Account $invitedByAccount)
    {
        $this->trainer = $trainer;
        $this->event = $event;
        $this->player = $player;
        $this->invitedByAccount = $invitedByAccount;
        $this->invitedAt = new \DateTimeImmutable();
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

    public function getInvitedByAccount(): Account
    {
        return $this->invitedByAccount;
    }

    public function getInvitedAt(): \DateTimeImmutable
    {
        return $this->invitedAt;
    }
}
