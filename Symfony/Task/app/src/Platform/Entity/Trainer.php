<?php

declare(strict_types=1);

namespace App\Platform\Entity;

use App\Identity\Entity\Account;
use App\Platform\Repository\TrainerRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * The tenant registry itself.
 *
 * Global, and necessarily so: scoping the tenant discriminator to itself is
 * circular, and it is read cross-tenant (the player's trainer switcher, public
 * content attribution) and pre-authentication (public form pages). Sensitive
 * billing fields — Connect account, fee rate, prices — live on the
 * trainer-scoped TrainerBillingSettings for exactly that reason.
 *
 * @see specs/architect-architecture.md Decisions, "`Trainer` registry placement"
 * @see specs/database-designer-schema.md "`trainer` — Platform"
 */
#[ORM\Entity(repositoryClass: TrainerRepository::class)]
#[ORM\Table(name: 'trainer')]
#[ORM\UniqueConstraint(name: 'uniq_trainer_slug', columns: ['slug'])]
#[ORM\UniqueConstraint(name: 'uniq_trainer_owner', columns: ['owner_account_id'])]
class Trainer
{
    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'owner_account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $ownerAccount;

    #[ORM\Column(type: 'string', length: 255)]
    private string $businessName;

    #[ORM\Column(type: 'string', length: 100)]
    private string $slug;

    /**
     * Not named by any epic, and required anyway: the 24-hour refund boundary
     * (BR-05-10) and the one-advance-booking-per-day rule (BR-05-14) are both
     * meaningless without knowing whose day is being measured. The architecture
     * flags this gap; this column closes it.
     */
    #[ORM\Column(type: 'string', length: 64, options: ['default' => 'America/New_York'])]
    private string $timezone = 'America/New_York';

    #[ORM\Column(type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    #[ORM\Column(type: 'datetimetz_immutable')]
    private \DateTimeImmutable $updatedAt;

    public function __construct(Account $ownerAccount, string $businessName, string $slug)
    {
        $this->ownerAccount = $ownerAccount;
        $this->businessName = $businessName;
        $this->slug = $slug;
        $this->createdAt = new \DateTimeImmutable();
        $this->updatedAt = $this->createdAt;
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getOwnerAccount(): Account
    {
        return $this->ownerAccount;
    }

    public function getBusinessName(): string
    {
        return $this->businessName;
    }

    public function getSlug(): string
    {
        return $this->slug;
    }

    public function getTimezone(): \DateTimeZone
    {
        return new \DateTimeZone($this->timezone);
    }
}
