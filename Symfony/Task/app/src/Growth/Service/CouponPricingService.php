<?php

declare(strict_types=1);

namespace App\Growth\Service;

use App\Billing\Entity\PaymentRecord;
use App\Growth\Dto\CouponQuote;
use App\Growth\Entity\Coupon;
use App\Growth\Entity\CouponRedemption;
use App\Growth\Repository\CouponRepository;
use App\Growth\Repository\CouponRedemptionRepository;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use Doctrine\ORM\EntityManagerInterface;

/**
 * US-06.06: validates and prices a coupon code against one purchase
 * (`quote()`), and records a redemption once the discounted payment has
 * actually succeeded (`redeem()`). This is the seam
 * `architect-architecture.md`'s module map means by "Scheduling... may
 * call... Growth (to price a coupon)" — `RsvpService` and
 * `PurchasePlaylistAccessService` both call `quote()` before ever
 * constructing a `PaymentIntentRequest`, so Billing/Stripe only ever sees
 * the already-discounted amount (AC-06-22).
 *
 * @see specs/requirements-analyst-epic-06-marketing-growth-spec.md AC-06-20..25, BR-06-8..10
 */
final readonly class CouponPricingService
{
    private const INVALID_MESSAGE = 'Invalid or expired code';

    public function __construct(
        private EntityManagerInterface $entityManager,
        private CouponRepository $coupons,
        private CouponRedemptionRepository $redemptions,
        private CouponEligibilityChecker $eligibility,
    ) {
    }

    /**
     * AC-06-21/23, BR-06-8: exists for this trainer, active, unexpired,
     * under its usage limit, applies to this purchase type, and the player
     * is eligible. Never mutates anything — safe to call from the JSON
     * preview endpoint and again at real checkout, any number of times.
     */
    public function quote(Trainer $trainer, string $code, PlayerProfile $player, string $purchaseType, int $originalAmountMinorUnits): CouponQuote
    {
        $trimmedCode = trim($code);

        if ('' === $trimmedCode) {
            return CouponQuote::invalid(self::INVALID_MESSAGE);
        }

        // Coupon codes are case-sensitive (see Coupon's own docblock) — an
        // exact-match lookup, deliberately not case-folded.
        $coupon = $this->coupons->findOneByTrainerAndCode($trainer, $trimmedCode);

        if (null === $coupon) {
            return CouponQuote::invalid(self::INVALID_MESSAGE);
        }

        if (!$coupon->isRedeemable(new \DateTimeImmutable())) {
            return CouponQuote::invalid(self::INVALID_MESSAGE);
        }

        if (!$coupon->appliesToPurchaseType($purchaseType)) {
            return CouponQuote::invalid(self::INVALID_MESSAGE);
        }

        if (!$this->eligibility->isEligible($coupon, $player, $trainer)) {
            return CouponQuote::invalid(self::INVALID_MESSAGE);
        }

        return CouponQuote::valid($coupon, $originalAmountMinorUnits);
    }

    /**
     * AC-06-25: logs the use (player, transaction, discount amount,
     * timestamp) and increments the coupon's usage count, auto-deactivating
     * it once the limit is reached (`Coupon::recordRedemption()`). Called
     * once a payment funded by this coupon has actually succeeded — never
     * at quote time, and never for a coupon-free purchase.
     *
     * Idempotent on `payment_record_id`: a Stripe webhook redelivery for a
     * payment already redeemed against is a silent no-op returning the
     * existing row, the same "check first" idiom
     * `TokenLedgerService::purchase()` uses for its own I6 invariant. The
     * database's own UNIQUE constraint on `payment_record_id` is the
     * authoritative backstop under a genuine concurrent race.
     */
    public function redeem(
        Coupon $coupon,
        PlayerProfile $player,
        PaymentRecord $paymentRecord,
        int $originalAmountMinorUnits,
        int $discountAmountMinorUnits,
        int $finalAmountMinorUnits,
    ): CouponRedemption {
        $existing = $this->redemptions->findOneByPaymentRecord($paymentRecord);

        if (null !== $existing) {
            return $existing;
        }

        return $this->entityManager->wrapInTransaction(function () use ($coupon, $player, $paymentRecord, $originalAmountMinorUnits, $discountAmountMinorUnits, $finalAmountMinorUnits): CouponRedemption {
            $redemption = new CouponRedemption(
                $coupon->getTrainer(),
                $coupon,
                $player,
                $paymentRecord,
                $originalAmountMinorUnits,
                $discountAmountMinorUnits,
                $finalAmountMinorUnits,
            );
            $this->redemptions->add($redemption);
            $coupon->recordRedemption();
            $this->entityManager->flush();

            return $redemption;
        });
    }
}
