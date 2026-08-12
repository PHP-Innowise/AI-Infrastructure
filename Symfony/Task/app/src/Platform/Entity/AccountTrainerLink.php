<?php

declare(strict_types=1);

namespace App\Platform\Entity;

use App\Identity\Entity\Account;
use App\Platform\Repository\AccountTrainerLinkRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * The tenant resolver's own source.
 *
 * This table MUST stay global and MUST never carry a Row-Level Security
 * policy. The resolver reads it to work out which tenant applies, so a policy
 * here would be evaluated with no tenant set, match nothing, and make every
 * login silently conclude the user belongs nowhere. The container startup gate
 * asserts its absence — see config/tenancy/resolver_global_tables.txt.
 *
 * `roleInTenant` is denormalized off Account::role so the resolver's hot-path
 * query never has to join `account`.
 *
 * @see specs/architect-architecture.md "Layer 3 — the mandatory tenant context"
 * @see specs/council-sharelink-tenant-resolution.md
 */
#[ORM\Entity(repositoryClass: AccountTrainerLinkRepository::class)]
#[ORM\Table(name: 'account_trainer_link')]
#[ORM\UniqueConstraint(name: 'uniq_account_trainer', columns: ['account_id', 'trainer_id'])]
#[ORM\Index(name: 'idx_atl_account', columns: ['account_id'])]
#[ORM\Index(name: 'idx_atl_trainer', columns: ['trainer_id'])]
class AccountTrainerLink
{
    public const STATUS_ACTIVE = 'active';
    public const STATUS_INACTIVE = 'inactive';

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $account;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\Column(name: 'role_in_tenant', type: 'string', length: 16)]
    private string $roleInTenant;

    #[ORM\Column(type: 'string', length: 16, options: ['default' => self::STATUS_ACTIVE])]
    private string $status = self::STATUS_ACTIVE;

    #[ORM\Column(name: 'linked_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $linkedAt;

    public function __construct(Account $account, Trainer $trainer, string $roleInTenant)
    {
        $this->account = $account;
        $this->trainer = $trainer;
        $this->roleInTenant = $roleInTenant;
        $this->linkedAt = new \DateTimeImmutable();
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getAccount(): Account
    {
        return $this->account;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getRoleInTenant(): string
    {
        return $this->roleInTenant;
    }

    public function getLinkedAt(): \DateTimeImmutable
    {
        return $this->linkedAt;
    }

    public function isActive(): bool
    {
        return self::STATUS_ACTIVE === $this->status;
    }

    /**
     * Written only by MembershipService/CoachMembership's owning workflow and
     * trainer creation, per this entity's own docblock — never directly by a
     * controller.
     */
    public function reactivate(): void
    {
        $this->status = self::STATUS_ACTIVE;
        $this->linkedAt = new \DateTimeImmutable();
    }

    public function deactivate(): void
    {
        $this->status = self::STATUS_INACTIVE;
    }
}
