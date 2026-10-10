<?php

declare(strict_types=1);

namespace App\Growth\Entity;

use App\Billing\Entity\PaymentRecord;
use App\Growth\Repository\CouponRedemptionRepository;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * One coupon use, tied 1:1 to the payment it discounted. BR-06-9/10.
 *
 * `paymentRecord` is UNIQUE: BR-06-10's "only one coupon per transaction" is
 * enforced structurally — a payment can fund at most one redemption row, the
 * same shape `PlaylistAccessGrant`/`EntitlementCoverage` already use for
 * their own one-payment-funds-one-thing invariants.
 *
 * @see specs/database-designer-schema.md "`coupon_redemption`"
 * @see specs/requirements-analyst-epic-06-marketing-growth-spec.md AC-06-22/24/25, BR-06-9/10
 */
#[ORM\Entity(repositoryClass: CouponRedemptionRepository::class)]
#[ORM\Table(name: 'coupon_redemption')]
#[ORM\UniqueConstraint(name: 'uniq_coupon_redemption_payment_record', columns: ['payment_record_id'])]
#[ORM\Index(name: 'idx_coupon_redemption_coupon', columns: ['coupon_id'])]
#[TrainerScoped]
class CouponRedemption
{
    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\ManyToOne(targetEntity: Coupon::class)]
    #[ORM\JoinColumn(name: 'coupon_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Coupon $coupon;

    #[ORM\ManyToOne(targetEntity: PlayerProfile::class)]
    #[ORM\JoinColumn(name: 'player_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private PlayerProfile $player;

    #[ORM\ManyToOne(targetEntity: PaymentRecord::class)]
    #[ORM\JoinColumn(name: 'payment_record_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private PaymentRecord $paymentRecord;

    #[ORM\Column(name: 'original_price_minor_units', type: 'integer')]
    private int $originalPriceMinorUnits;

    #[ORM\Column(name: 'discount_amount_minor_units', type: 'integer')]
    private int $discountAmountMinorUnits;

    #[ORM\Column(name: 'final_price_minor_units', type: 'integer')]
    private int $finalPriceMinorUnits;

    #[ORM\Column(name: 'redeemed_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $redeemedAt;

    public function __construct(
        Trainer $trainer,
        Coupon $coupon,
        PlayerProfile $player,
        PaymentRecord $paymentRecord,
        int $originalPriceMinorUnits,
        int $discountAmountMinorUnits,
        int $finalPriceMinorUnits,
    ) {
        // BR-06-9: the floor-at-$0 rule, enforced structurally by the
        // database CHECK too — this is the fail-fast application guard.
        if ($discountAmountMinorUnits < 0) {
            throw new \InvalidArgumentException('A coupon discount amount cannot be negative.');
        }

        if ($finalPriceMinorUnits !== $originalPriceMinorUnits - $discountAmountMinorUnits) {
            throw new \InvalidArgumentException('A coupon redemption\'s final price must equal original minus discount.');
        }

        if ($finalPriceMinorUnits < 0) {
            throw new \InvalidArgumentException('BR-06-9: a coupon-discounted price cannot go negative.');
        }

        $this->trainer = $trainer;
        $this->coupon = $coupon;
        $this->player = $player;
        $this->paymentRecord = $paymentRecord;
        $this->originalPriceMinorUnits = $originalPriceMinorUnits;
        $this->discountAmountMinorUnits = $discountAmountMinorUnits;
        $this->finalPriceMinorUnits = $finalPriceMinorUnits;
        $this->redeemedAt = new \DateTimeImmutable();
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getCoupon(): Coupon
    {
        return $this->coupon;
    }

    public function getPlayer(): PlayerProfile
    {
        return $this->player;
    }

    public function getPaymentRecord(): PaymentRecord
    {
        return $this->paymentRecord;
    }

    public function getOriginalPriceMinorUnits(): int
    {
        return $this->originalPriceMinorUnits;
    }

    public function getDiscountAmountMinorUnits(): int
    {
        return $this->discountAmountMinorUnits;
    }

    public function getFinalPriceMinorUnits(): int
    {
        return $this->finalPriceMinorUnits;
    }

    public function getRedeemedAt(): \DateTimeImmutable
    {
        return $this->redeemedAt;
    }
}
