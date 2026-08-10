<?php

declare(strict_types=1);

namespace App\Identity\Entity;

use App\Identity\Repository\ShareLinkRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * A trainer's invitation code: a static, unlimited-use link for players, or a
 * unique, one-time, 7-day-expiry link for a coach (or a coach-issued player
 * invite, Epic-03).
 *
 * Trainer-scoped, and deliberately reached in two different ways: through the
 * ordinary trainer-scoped repository once a tenant is resolved, and through
 * `PublicTenantCode` (Platform, global, no RLS) for the anonymous click that
 * has to resolve the tenant in the first place. Creating a ShareLink MUST
 * also create the matching `PublicTenantCode` row in the same transaction —
 * see `ShareLinkService`.
 *
 * @see specs/database-designer-schema.md "`share_link`"
 * @see specs/requirements-analyst-epic-01-user-management-spec.md BR-01-14, BR-01-15, BR-01-27
 */
#[ORM\Entity(repositoryClass: ShareLinkRepository::class)]
#[ORM\Table(name: 'share_link')]
#[ORM\UniqueConstraint(name: 'uniq_share_link_code', columns: ['code'])]
#[ORM\Index(name: 'idx_share_link_trainer', columns: ['trainer_id'])]
#[TrainerScoped]
class ShareLink
{
    public const TYPE_STATIC_PLAYER = 'static_player';
    public const TYPE_UNIQUE_COACH = 'unique_coach';

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    /**
     * Globally unique, high-entropy, URL-safe. Shared with the matching
     * `PublicTenantCode.code` value.
     */
    #[ORM\Column(type: 'string', length: 64)]
    private string $code;

    #[ORM\Column(name: 'link_type', type: 'string', length: 16)]
    private string $linkType;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'created_by_account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $createdByAccount;

    /**
     * Coach links only — who the invite was addressed to.
     */
    #[ORM\Column(type: 'string', length: 255, nullable: true, columnDefinition: 'CITEXT DEFAULT NULL')]
    private ?string $targetEmail = null;

    #[ORM\Column(type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $expiresAt = null;

    #[ORM\Column(type: 'integer', nullable: true)]
    private ?int $maxUses = null;

    #[ORM\Column(type: 'integer', options: ['default' => 0])]
    private int $useCount = 0;

    #[ORM\Column(type: 'boolean', options: ['default' => true])]
    private bool $isActive = true;

    #[ORM\Column(type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    public function __construct(
        Trainer $trainer,
        string $code,
        string $linkType,
        Account $createdByAccount,
        ?string $targetEmail = null,
    ) {
        if (!\in_array($linkType, [self::TYPE_STATIC_PLAYER, self::TYPE_UNIQUE_COACH], true)) {
            throw new \InvalidArgumentException(sprintf('Unknown ShareLink type "%s".', $linkType));
        }

        if ('' === trim($code)) {
            throw new \InvalidArgumentException('A ShareLink requires a non-empty code.');
        }

        $this->trainer = $trainer;
        $this->code = $code;
        $this->linkType = $linkType;
        $this->createdByAccount = $createdByAccount;
        $this->createdAt = new \DateTimeImmutable();

        if (self::TYPE_UNIQUE_COACH === $linkType) {
            // BR-01-15: one-time use, 7-day expiry.
            if ('' === trim((string) $targetEmail)) {
                throw new \InvalidArgumentException('A unique coach ShareLink requires a target email.');
            }

            $this->targetEmail = $targetEmail;
            $this->expiresAt = $this->createdAt->modify('+7 days');
            $this->maxUses = 1;
        }
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getCode(): string
    {
        return $this->code;
    }

    public function getLinkType(): string
    {
        return $this->linkType;
    }

    public function isCoachLink(): bool
    {
        return self::TYPE_UNIQUE_COACH === $this->linkType;
    }

    public function getCreatedByAccount(): Account
    {
        return $this->createdByAccount;
    }

    public function getTargetEmail(): ?string
    {
        return $this->targetEmail;
    }

    public function getExpiresAt(): ?\DateTimeImmutable
    {
        return $this->expiresAt;
    }

    public function getUseCount(): int
    {
        return $this->useCount;
    }

    public function getMaxUses(): ?int
    {
        return $this->maxUses;
    }

    public function isRevoked(): bool
    {
        return !$this->isActive;
    }

    /**
     * AC-01-42: an expired invitation shows a clear message with an option to
     * resend, rather than silently failing.
     */
    public function isExpired(\DateTimeImmutable $now): bool
    {
        return null !== $this->expiresAt && $this->expiresAt < $now;
    }

    public function isExhausted(): bool
    {
        return null !== $this->maxUses && $this->useCount >= $this->maxUses;
    }

    /**
     * AC-01-9, AC-01-31, AC-03-50: the single predicate every acceptance path
     * consults before honouring a code.
     */
    public function isUsable(\DateTimeImmutable $now): bool
    {
        return $this->isActive && !$this->isExpired($now) && !$this->isExhausted();
    }

    /**
     * BR-01-27: usage count and timing are tracked per link.
     */
    public function recordUse(): void
    {
        ++$this->useCount;
    }

    public function revoke(): void
    {
        $this->isActive = false;
    }

    /**
     * AC-01-42: reissuing a fresh 7-day window on resend, rather than forcing
     * a brand-new code (and therefore a new PublicTenantCode row) for what is
     * conceptually the same invitation.
     */
    public function renew(\DateTimeImmutable $now): void
    {
        if (!$this->isCoachLink()) {
            throw new \LogicException('Only unique coach links can be renewed.');
        }

        $this->expiresAt = $now->modify('+7 days');
        $this->isActive = true;
    }
}
