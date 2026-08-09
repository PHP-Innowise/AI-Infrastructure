<?php

declare(strict_types=1);

namespace App\Identity\Entity;

use Doctrine\ORM\Mapping as ORM;

/**
 * The person being trained — self or child — independent of any trainer.
 *
 * Global, because a player may train with several trainers and the person does
 * not belong to any one of them. Everything trainer-specific (skill level,
 * association source, status) lives on the trainer-scoped
 * PlayerTrainerMembership.
 *
 * Epic-01 states outright that it does not resolve how player and parent
 * should be modelled. This is the engineering resolution: `selfAccountId` is
 * the optional own-login link, and ParentChildLink is the parent relationship.
 * The business rule is already settled — owner decision A1 makes every
 * under-18 player parent-managed.
 *
 * @see specs/database-designer-schema.md "`player_profile` — Identity"
 */
#[ORM\Entity]
#[ORM\Table(name: 'player_profile')]
#[ORM\UniqueConstraint(name: 'uniq_player_self_account', columns: ['self_account_id'])]
class PlayerProfile
{
    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'self_account_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?Account $selfAccount = null;

    #[ORM\Column(type: 'string', length: 100)]
    private string $firstName;

    /**
     * Date of birth is stored; age and age-group are derived at query time.
     * Q-01.02's safe default — storing a group would go stale every birthday.
     */
    #[ORM\Column(name: 'date_of_birth', type: 'date_immutable')]
    private \DateTimeImmutable $dateOfBirth;

    #[ORM\Column(type: 'string', length: 32, nullable: true)]
    private ?string $gender = null;

    /**
     * Denormalized and service-maintained, not database-generated: it depends
     * on ParentChildLink, a different table, and PostgreSQL generated columns
     * may only read the same row.
     */
    #[ORM\Column(name: 'is_child', type: 'boolean', options: ['default' => false])]
    private bool $isChild = false;

    #[ORM\Column(type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    #[ORM\Column(type: 'datetimetz_immutable')]
    private \DateTimeImmutable $updatedAt;

    public function __construct(string $firstName, \DateTimeImmutable $dateOfBirth, ?Account $selfAccount = null)
    {
        $this->firstName = $firstName;
        $this->dateOfBirth = $dateOfBirth;
        $this->selfAccount = $selfAccount;
        $this->createdAt = new \DateTimeImmutable();
        $this->updatedAt = $this->createdAt;
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getFirstName(): string
    {
        return $this->firstName;
    }

    public function getSelfAccount(): ?Account
    {
        return $this->selfAccount;
    }

    public function getDateOfBirth(): \DateTimeImmutable
    {
        return $this->dateOfBirth;
    }

    public function ageOn(\DateTimeImmutable $on): int
    {
        return $this->dateOfBirth->diff($on)->y;
    }

    public function isChild(): bool
    {
        return $this->isChild;
    }

    /**
     * Set when a ParentChildLink is created. Never legitimately flips back —
     * a child does not become un-parented by growing up; the account is
     * migrated instead.
     */
    public function markAsChild(): void
    {
        $this->isChild = true;
        $this->updatedAt = new \DateTimeImmutable();
    }
}
