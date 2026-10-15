<?php

declare(strict_types=1);

namespace App\Identity\Entity;

use App\Identity\Repository\ShareLinkRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * A trainer's invitation code: a static, unlimited-use link for players, a
 * unique, one-time, 7-day-expiry link for a coach, or (Epic-03) a unique,
 * one-time, no-expiry link a COACH issues to invite a player.
 *
 * Trainer-scoped, and deliberately reached in two different ways: through the
 * ordinary trainer-scoped repository once a tenant is resolved, and through
 * `PublicTenantCode` (Platform, global, no RLS) for the anonymous click that
 * has to resolve the tenant in the first place. Creating a ShareLink MUST
 * also create the matching `PublicTenantCode` row in the same transaction —
 * see `ShareLinkService`.
 *
 * `TYPE_COACH_PLAYER_INVITE` resolves an open question the epic itself flags
 * as unsettled (`specs/requirements-analyst-epic-03-crm-players-spec.md`
 * Open questions, "Cross-epic gap, new ShareLink type") the way
 * `specs/api-designer-spec.md`'s "Identity module" Decisions table settles
 * it: `/invite/{code}` is "One route serving both Epic-01's coach invitation
 * and Epic-03's coach-issued player invitation, branching on
 * `ShareLink.type`" — which requires a third, distinct type value, not a
 * reuse of `TYPE_UNIQUE_COACH` under a different issuer, since "only the
 * stored `ShareLink.type` differs" between the two flows sharing that URL.
 * Unlike `TYPE_UNIQUE_COACH` (BR-01-15: mandatory target email, mandatory
 * 7-day expiry), AC-03-50's recipient email is optional
 * (`specs/api-designer-spec.md:421`, `InvitePlayerType`) and no expiry is
 * stated anywhere in the epic — modeled as single-use, no expiry, optional
 * email. See the coder's final report.
 *
 * @see specs/database-designer-schema.md "`share_link`"
 * @see specs/requirements-analyst-epic-01-user-management-spec.md BR-01-14, BR-01-15, BR-01-27
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md AC-03-50..53, Open questions
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
    public const TYPE_COACH_PLAYER_INVITE = 'coach_player_invite';
    public const TYPE_UNIQUE_PLAYER_INVITE = 'unique_player_invite';

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

    #[ORM\Column(name: 'link_type', type: 'string', length: 24)]
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
        if (!\in_array($linkType, [self::TYPE_STATIC_PLAYER, self::TYPE_UNIQUE_COACH, self::TYPE_COACH_PLAYER_INVITE, self::TYPE_UNIQUE_PLAYER_INVITE], true)) {
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

        if (self::TYPE_COACH_PLAYER_INVITE === $linkType) {
            // AC-03-50/52: single-use, no stated expiry, recipient email
            // optional (unlike the coach-invite type above) — see this
            // class's own docblock for why this is a third type rather than
            // a reuse of TYPE_UNIQUE_COACH.
            $this->targetEmail = '' === trim((string) $targetEmail) ? null : $targetEmail;
            $this->maxUses = 1;
        }

        if (self::TYPE_UNIQUE_PLAYER_INVITE === $linkType) {
            // AC-03-61 (optional MVP): a TRAINER's own unique, one-time link
            // per player/parent — the same single-use/no-expiry/optional-email
            // shape as the coach's own player invite above, kept as a
            // separate type (not a reuse of TYPE_COACH_PLAYER_INVITE) purely
            // for issuer clarity: nothing downstream currently branches on
            // this distinction (the landing flow at /join/{code} already
            // handles any usable ShareLink type generically), but a link
            // literally named "coach_player_invite" created by a trainer
            // would be a misleading audit trail.
            $this->targetEmail = '' === trim((string) $targetEmail) ? null : $targetEmail;
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

    /**
     * AC-03-50..53: a coach-issued invitation to a specific player, reached
     * through the same `/invite/{code}` URL as `isCoachLink()`'s type — see
     * this class's own docblock.
     */
    public function isCoachPlayerInviteLink(): bool
    {
        return self::TYPE_COACH_PLAYER_INVITE === $this->linkType;
    }

    /**
     * AC-03-61 (optional MVP): a trainer's own unique, one-time invite to a
     * specific player/parent, reached through `/join/{code}` — the same
     * landing flow as the static mass link, which does not branch on type.
     */
    public function isUniquePlayerInviteLink(): bool
    {
        return self::TYPE_UNIQUE_PLAYER_INVITE === $this->linkType;
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

    /**
     * AC-03-63 (optional MVP): the tracking report's own "date" column.
     */
    public function getCreatedAt(): \DateTimeImmutable
    {
        return $this->createdAt;
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
