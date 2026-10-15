<?php

declare(strict_types=1);

namespace App\Identity\Entity;

use App\Identity\Repository\ParentChildLinkRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * The parent/child relationship the epic itself declines to model (see
 * PlayerProfile's own docblock). One row per child PlayerProfile.
 *
 * Global: a parent's children follow the parent across every trainer
 * relationship, so this cannot carry a tenant key without picking one
 * arbitrary trainer to own a fact that belongs to the whole family.
 *
 * @see specs/database-designer-schema.md "`parent_child_link`"
 * @see specs/requirements-analyst-epic-01-user-management-spec.md BR-01-16, BR-01-17, AC-01-16, AC-01-20, AC-01-27
 */
#[ORM\Entity(repositoryClass: ParentChildLinkRepository::class)]
#[ORM\Table(name: 'parent_child_link')]
#[ORM\UniqueConstraint(name: 'uniq_pcl_child_player', columns: ['child_player_id'])]
#[ORM\UniqueConstraint(name: 'uniq_pcl_child_account', columns: ['child_account_id'])]
#[ORM\Index(name: 'idx_pcl_parent', columns: ['parent_account_id'])]
class ParentChildLink
{
    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'parent_account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $parentAccount;

    #[ORM\ManyToOne(targetEntity: PlayerProfile::class)]
    #[ORM\JoinColumn(name: 'child_player_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private PlayerProfile $childPlayer;

    /**
     * Set only if the child also has their own login (AC-01-20).
     */
    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'child_account_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?Account $childAccount = null;

    /**
     * AC-01-27/28. Defaults OFF: token purchases require approval unless the
     * parent explicitly opts a specific child out of it.
     */
    #[ORM\Column(name: 'allow_token_spending_without_approval', type: 'boolean', options: ['default' => false])]
    private bool $allowTokenSpendingWithoutApproval = false;

    #[ORM\Column(type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    public function __construct(Account $parentAccount, PlayerProfile $childPlayer, ?Account $childAccount = null)
    {
        if (null !== $childAccount && $parentAccount === $childAccount) {
            throw new \InvalidArgumentException('A parent account cannot also be its own child\'s separate login.');
        }

        $this->parentAccount = $parentAccount;
        $this->childPlayer = $childPlayer;
        $this->childAccount = $childAccount;
        $this->createdAt = new \DateTimeImmutable();

        $childPlayer->markAsChild();
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getParentAccount(): Account
    {
        return $this->parentAccount;
    }

    public function getChildPlayer(): PlayerProfile
    {
        return $this->childPlayer;
    }

    public function getChildAccount(): ?Account
    {
        return $this->childAccount;
    }

    /**
     * AC-01-20: a child can optionally be given a separate login after the
     * profile already exists.
     */
    public function attachChildAccount(Account $childAccount): void
    {
        if ($this->parentAccount === $childAccount) {
            throw new \InvalidArgumentException('A parent account cannot also be its own child\'s separate login.');
        }

        $this->childAccount = $childAccount;
    }

    public function allowsTokenSpendingWithoutApproval(): bool
    {
        return $this->allowTokenSpendingWithoutApproval;
    }

    /**
     * AC-01-28: the parent can change this at any time from the child's
     * profile settings.
     */
    public function setAllowTokenSpendingWithoutApproval(bool $allowed): void
    {
        $this->allowTokenSpendingWithoutApproval = $allowed;
    }
}
