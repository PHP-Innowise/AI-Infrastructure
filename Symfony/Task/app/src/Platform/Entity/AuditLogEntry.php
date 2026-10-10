<?php

declare(strict_types=1);

namespace App\Platform\Entity;

use App\Identity\Entity\Account;
use App\Platform\Repository\AuditLogEntryRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * Append-only, spans tenants by design. Written exclusively through
 * `AuditLogger` (AC-01-76) — never constructed by a controller directly.
 *
 * Privilege, not convention, makes this append-only: the migration that
 * creates this table revokes UPDATE/DELETE from the application role, so "no
 * entry is ever changed" is a database privilege, not a code convention.
 *
 * `subjectId`/`subjectType` are a deliberate soft reference, unlike a typed
 * FK: audit subjects span nearly every entity in the system, so a typed FK
 * per possible subject would mean dozens of always-null columns on a table
 * whose entire point is to log everything.
 *
 * @see specs/database-designer-schema.md "`audit_log_entry`"
 */
#[ORM\Entity(repositoryClass: AuditLogEntryRepository::class)]
#[ORM\Table(name: 'audit_log_entry')]
#[ORM\Index(name: 'idx_audit_occurred_at', columns: ['occurred_at'])]
#[ORM\Index(name: 'idx_audit_trainer_occurred_at', columns: ['related_trainer_id', 'occurred_at'])]
#[ORM\Index(name: 'idx_audit_action_type', columns: ['action_type'])]
#[ORM\Index(name: 'idx_audit_actor', columns: ['actor_account_id'])]
class AuditLogEntry
{
    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\Column(name: 'occurred_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $occurredAt;

    /**
     * Nullable only for the rare system-initiated entry.
     */
    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'actor_account_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?Account $actorAccount = null;

    #[ORM\Column(name: 'action_type', type: 'string', length: 64)]
    private string $actionType;

    #[ORM\Column(name: 'subject_type', type: 'string', length: 64)]
    private string $subjectType;

    #[ORM\Column(name: 'subject_id', type: 'bigint', nullable: true)]
    private ?int $subjectId = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'related_trainer_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?Trainer $relatedTrainer = null;

    /**
     * @var array<string, mixed>
     */
    #[ORM\Column(type: 'json', options: ['default' => '{}'])]
    private array $details = [];

    #[ORM\Column(type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    /**
     * @param array<string, mixed> $details
     */
    public function __construct(
        ?Account $actorAccount,
        string $actionType,
        string $subjectType,
        ?int $subjectId,
        ?Trainer $relatedTrainer,
        array $details = [],
    ) {
        if ('' === trim($actionType)) {
            throw new \InvalidArgumentException('An audit log entry requires a non-empty action type.');
        }

        $this->actorAccount = $actorAccount;
        $this->actionType = $actionType;
        $this->subjectType = $subjectType;
        $this->subjectId = $subjectId;
        $this->relatedTrainer = $relatedTrainer;
        $this->details = $details;
        $this->occurredAt = new \DateTimeImmutable();
        $this->createdAt = $this->occurredAt;
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getOccurredAt(): \DateTimeImmutable
    {
        return $this->occurredAt;
    }

    public function getActorAccount(): ?Account
    {
        return $this->actorAccount;
    }

    public function getActionType(): string
    {
        return $this->actionType;
    }

    public function getSubjectType(): string
    {
        return $this->subjectType;
    }

    public function getSubjectId(): ?int
    {
        return $this->subjectId;
    }

    public function getRelatedTrainer(): ?Trainer
    {
        return $this->relatedTrainer;
    }

    /**
     * @return array<string, mixed>
     */
    public function getDetails(): array
    {
        return $this->details;
    }
}
