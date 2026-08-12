<?php

declare(strict_types=1);

namespace App\Scheduling\Entity;

use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use App\Scheduling\Repository\EventDuplicationRecordRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * BR-02-19: "a reference to the original event is stored for analytics"
 * whenever a trainer duplicates one. Written once per duplication, never
 * updated.
 *
 * @see specs/database-designer-schema.md "`event_duplication_record`"
 * @see specs/requirements-analyst-epic-02-event-management-spec.md BR-02-19, AC-02-14..17
 */
#[ORM\Entity(repositoryClass: EventDuplicationRecordRepository::class)]
#[ORM\Table(name: 'event_duplication_record')]
#[ORM\UniqueConstraint(name: 'uniq_event_duplication_new_event', columns: ['new_event_id'])]
#[ORM\Index(name: 'idx_event_duplication_original', columns: ['original_event_id'])]
#[TrainerScoped]
class EventDuplicationRecord
{
    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\ManyToOne(targetEntity: Event::class)]
    #[ORM\JoinColumn(name: 'original_event_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Event $originalEvent;

    #[ORM\ManyToOne(targetEntity: Event::class)]
    #[ORM\JoinColumn(name: 'new_event_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Event $newEvent;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'duplicated_by_account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $duplicatedByAccount;

    #[ORM\Column(name: 'duplicated_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $duplicatedAt;

    public function __construct(Trainer $trainer, Event $originalEvent, Event $newEvent, Account $duplicatedByAccount)
    {
        $this->trainer = $trainer;
        $this->originalEvent = $originalEvent;
        $this->newEvent = $newEvent;
        $this->duplicatedByAccount = $duplicatedByAccount;
        $this->duplicatedAt = new \DateTimeImmutable();
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getOriginalEvent(): Event
    {
        return $this->originalEvent;
    }

    public function getNewEvent(): Event
    {
        return $this->newEvent;
    }

    public function getDuplicatedByAccount(): Account
    {
        return $this->duplicatedByAccount;
    }

    public function getDuplicatedAt(): \DateTimeImmutable
    {
        return $this->duplicatedAt;
    }
}
