<?php

declare(strict_types=1);

namespace App\Identity\Entity;

use App\Identity\Repository\UserDeletionRecordRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * GDPR compliance record, written in the same transaction as the anonymizing
 * UPDATE to `account`/`account_profile` (AC-01-56, AC-01-59). The account row
 * itself survives, anonymized; this row is the durable proof of who deleted
 * it, when, and why.
 *
 * @see specs/database-designer-schema.md "`user_deletion_record`"
 * @see specs/requirements-analyst-epic-01-user-management-spec.md BR-01-24, AC-01-59
 */
#[ORM\Entity(repositoryClass: UserDeletionRecordRepository::class)]
#[ORM\Table(name: 'user_deletion_record')]
#[ORM\Index(name: 'idx_udr_original_account', columns: ['original_account_id'])]
class UserDeletionRecord
{
    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'original_account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $originalAccount;

    #[ORM\Column(name: 'original_email', type: 'string', length: 255)]
    private string $originalEmail;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'deleted_by_account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $deletedByAccount;

    #[ORM\Column(type: 'text', nullable: true)]
    private ?string $reason = null;

    #[ORM\Column(name: 'deleted_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $deletedAt;

    public function __construct(Account $originalAccount, string $originalEmail, Account $deletedByAccount, ?string $reason = null)
    {
        $this->originalAccount = $originalAccount;
        $this->originalEmail = $originalEmail;
        $this->deletedByAccount = $deletedByAccount;
        $this->reason = $reason;
        $this->deletedAt = new \DateTimeImmutable();
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getOriginalAccount(): Account
    {
        return $this->originalAccount;
    }

    public function getOriginalEmail(): string
    {
        return $this->originalEmail;
    }

    public function getDeletedByAccount(): Account
    {
        return $this->deletedByAccount;
    }

    public function getReason(): ?string
    {
        return $this->reason;
    }

    public function getDeletedAt(): \DateTimeImmutable
    {
        return $this->deletedAt;
    }
}
