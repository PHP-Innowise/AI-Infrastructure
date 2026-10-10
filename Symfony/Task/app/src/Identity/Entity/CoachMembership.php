<?php

declare(strict_types=1);

namespace App\Identity\Entity;

use App\Identity\Repository\CoachMembershipRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * A coach's relationship with the one trainer they currently work for.
 *
 * BR-01-11: strictly one active trainer at a time. That invariant is not
 * enforced here — a CHECK constraint cannot see other rows — but by a unique
 * partial index with no `trainer_id` in its key
 * (`uniq_coach_membership_active_account`), which reaches across tenants from
 * inside an RLS-protected table on purpose. See the migration for the note on
 * why that is safe and deliberate.
 *
 * @see specs/database-designer-schema.md "`coach_membership`"
 * @see specs/requirements-analyst-epic-01-user-management-spec.md BR-01-11, BR-01-15, AC-01-39..41
 */
#[ORM\Entity(repositoryClass: CoachMembershipRepository::class)]
#[ORM\Table(name: 'coach_membership')]
#[ORM\Index(name: 'idx_coach_membership_trainer_status', columns: ['trainer_id', 'status'])]
#[ORM\Index(name: 'idx_coach_membership_account', columns: ['account_id'])]
#[TrainerScoped]
class CoachMembership
{
    public const STATUS_PENDING = 'pending';
    public const STATUS_ACTIVE = 'active';
    public const STATUS_DECLINED = 'declined';

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $account;

    #[ORM\Column(type: 'string', length: 16, options: ['default' => self::STATUS_PENDING])]
    private string $status = self::STATUS_PENDING;

    #[ORM\Column(type: 'text', nullable: true)]
    private ?string $bio = null;

    #[ORM\Column(type: 'text', nullable: true)]
    private ?string $credentials = null;

    #[ORM\Column(type: 'text', nullable: true)]
    private ?string $certifications = null;

    #[ORM\Column(name: 'is_public_profile', type: 'boolean', options: ['default' => false])]
    private bool $isPublicProfile = false;

    #[ORM\ManyToOne(targetEntity: ShareLink::class)]
    #[ORM\JoinColumn(name: 'invited_via_share_link_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?ShareLink $invitedViaShareLink = null;

    #[ORM\Column(name: 'joined_at', type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $joinedAt = null;

    #[ORM\Column(type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    public function __construct(
        Trainer $trainer,
        Account $account,
        string $status = self::STATUS_PENDING,
        ?ShareLink $invitedViaShareLink = null,
    ) {
        if (!\in_array($status, self::statuses(), true)) {
            throw new \InvalidArgumentException(sprintf('Unknown coach membership status "%s".', $status));
        }

        $this->trainer = $trainer;
        $this->account = $account;
        $this->status = $status;
        $this->invitedViaShareLink = $invitedViaShareLink;
        $this->createdAt = new \DateTimeImmutable();

        if (self::STATUS_ACTIVE === $status) {
            $this->joinedAt = $this->createdAt;
        }
    }

    /**
     * @return list<string>
     */
    public static function statuses(): array
    {
        return [self::STATUS_PENDING, self::STATUS_ACTIVE, self::STATUS_DECLINED];
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getAccount(): Account
    {
        return $this->account;
    }

    public function getStatus(): string
    {
        return $this->status;
    }

    public function isActive(): bool
    {
        return self::STATUS_ACTIVE === $this->status;
    }

    public function isPending(): bool
    {
        return self::STATUS_PENDING === $this->status;
    }

    public function getInvitedViaShareLink(): ?ShareLink
    {
        return $this->invitedViaShareLink;
    }

    public function getJoinedAt(): ?\DateTimeImmutable
    {
        return $this->joinedAt;
    }

    /**
     * AC-01-40: appears in the trainer's Coaches list once accepted.
     */
    public function accept(): void
    {
        $this->status = self::STATUS_ACTIVE;
        $this->joinedAt = new \DateTimeImmutable();
    }

    public function decline(): void
    {
        $this->status = self::STATUS_DECLINED;
    }

    public function setBio(?string $bio): void
    {
        $this->bio = $bio;
    }

    public function getBio(): ?string
    {
        return $this->bio;
    }

    public function setCredentials(?string $credentials): void
    {
        $this->credentials = $credentials;
    }

    public function getCredentials(): ?string
    {
        return $this->credentials;
    }

    public function setCertifications(?string $certifications): void
    {
        $this->certifications = $certifications;
    }

    public function getCertifications(): ?string
    {
        return $this->certifications;
    }

    public function isPublicProfile(): bool
    {
        return $this->isPublicProfile;
    }

    public function setPublicProfile(bool $isPublicProfile): void
    {
        $this->isPublicProfile = $isPublicProfile;
    }
}
