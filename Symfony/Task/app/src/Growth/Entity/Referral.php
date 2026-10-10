<?php

declare(strict_types=1);

namespace App\Growth\Entity;

use App\Billing\Entity\PaymentRecord;
use App\Identity\Entity\PlayerProfile;
use App\Growth\Repository\ReferralRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * One friend's referral journey: click -> registration ("Pending") -> first
 * purchase ("Converted"). BR-06-1/2/3.
 *
 * **No `rewarded` status**, even though the epic's own Data Requirements
 * lists Pending/Converted/Rewarded — whether a reward was actually granted
 * is a fact about the token ledger (a `TokenEntry` of kind
 * `referral_reward` referencing this row's id), not a fact about this row;
 * see `specs/database-designer-schema.md` "`referral`"'s own note, which
 * this entity follows exactly.
 *
 * The `status = 'pending'` -> `'converted'` transition IS this codebase's
 * "first purchase" detection: `referee_player_id` is unique (a player is
 * referred at most once, ever), so the FIRST successful purchase event
 * `ReferralRewardSubscriber` observes for a still-pending referral's referee
 * is, by construction, that player's first purchase — no separate query
 * against Billing's purchase history is needed (Growth may not read
 * Billing's repositories directly; see that subscriber's own docblock).
 *
 * @see specs/database-designer-schema.md "`referral`"
 * @see specs/requirements-analyst-epic-06-marketing-growth-spec.md AC-06-4..12, BR-06-1..3/5
 */
#[ORM\Entity(repositoryClass: ReferralRepository::class)]
#[ORM\Table(name: 'referral')]
#[ORM\UniqueConstraint(name: 'uniq_referral_referee', columns: ['referee_player_id'])]
#[ORM\Index(name: 'idx_referral_referrer_trainer', columns: ['referrer_player_id', 'trainer_id'])]
#[TrainerScoped]
class Referral
{
    public const STATUS_PENDING = 'pending';
    public const STATUS_CONVERTED = 'converted';

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\ManyToOne(targetEntity: ReferralLink::class)]
    #[ORM\JoinColumn(name: 'referral_link_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private ReferralLink $referralLink;

    #[ORM\ManyToOne(targetEntity: PlayerProfile::class)]
    #[ORM\JoinColumn(name: 'referrer_player_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private PlayerProfile $referrerPlayer;

    #[ORM\ManyToOne(targetEntity: PlayerProfile::class)]
    #[ORM\JoinColumn(name: 'referee_player_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private PlayerProfile $refereePlayer;

    /**
     * BR-06-1: read from the 30-day attribution cookie set when the friend
     * clicked the referral link, not "now" at registration time.
     */
    #[ORM\Column(name: 'clicked_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $clickedAt;

    #[ORM\Column(name: 'registered_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $registeredAt;

    #[ORM\Column(type: 'string', length: 16)]
    private string $status = self::STATUS_PENDING;

    #[ORM\ManyToOne(targetEntity: PaymentRecord::class)]
    #[ORM\JoinColumn(name: 'first_purchase_payment_record_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?PaymentRecord $firstPurchasePaymentRecord = null;

    #[ORM\Column(name: 'first_purchase_at', type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $firstPurchaseAt = null;

    public function __construct(
        Trainer $trainer,
        ReferralLink $referralLink,
        PlayerProfile $referrerPlayer,
        PlayerProfile $refereePlayer,
        \DateTimeImmutable $clickedAt,
        \DateTimeImmutable $registeredAt,
    ) {
        // BR-06-3: self-referral is blocked outright. The database CHECK
        // (referrer_player_id <> referee_player_id) is the authoritative
        // backstop under concurrency; this is the fail-fast application
        // guard, matching this codebase's usual belt-and-suspenders shape
        // (e.g. TokenEntry's own constructor guards alongside CHECK
        // constraints).
        if ($referrerPlayer->getId() === $refereePlayer->getId()) {
            throw new \InvalidArgumentException('BR-06-3: a player cannot refer themselves.');
        }

        $this->trainer = $trainer;
        $this->referralLink = $referralLink;
        $this->referrerPlayer = $referrerPlayer;
        $this->refereePlayer = $refereePlayer;
        $this->clickedAt = $clickedAt;
        $this->registeredAt = $registeredAt;
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getReferralLink(): ReferralLink
    {
        return $this->referralLink;
    }

    public function getReferrerPlayer(): PlayerProfile
    {
        return $this->referrerPlayer;
    }

    public function getRefereePlayer(): PlayerProfile
    {
        return $this->refereePlayer;
    }

    public function getClickedAt(): \DateTimeImmutable
    {
        return $this->clickedAt;
    }

    public function getRegisteredAt(): \DateTimeImmutable
    {
        return $this->registeredAt;
    }

    public function getStatus(): string
    {
        return $this->status;
    }

    public function isPending(): bool
    {
        return self::STATUS_PENDING === $this->status;
    }

    public function isConverted(): bool
    {
        return self::STATUS_CONVERTED === $this->status;
    }

    public function getFirstPurchasePaymentRecord(): ?PaymentRecord
    {
        return $this->firstPurchasePaymentRecord;
    }

    public function getFirstPurchaseAt(): ?\DateTimeImmutable
    {
        return $this->firstPurchaseAt;
    }

    /**
     * AC-06-8/12: fires exactly once, on the referee's first purchase.
     * Idempotent by construction — a referral already converted silently
     * stays converted, which is what lets `ReferralRewardSubscriber` call
     * this unconditionally without its own duplicate check (Messenger
     * redelivery safety).
     */
    public function markConverted(PaymentRecord $paymentRecord, \DateTimeImmutable $at): bool
    {
        if ($this->isConverted()) {
            return false;
        }

        $this->status = self::STATUS_CONVERTED;
        $this->firstPurchasePaymentRecord = $paymentRecord;
        $this->firstPurchaseAt = $at;

        return true;
    }
}
