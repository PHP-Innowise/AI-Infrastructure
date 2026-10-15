<?php

declare(strict_types=1);

namespace App\Growth\Repository;

use App\Billing\Entity\PaymentRecord;
use App\Growth\Entity\Coupon;
use App\Growth\Entity\CouponRedemption;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<CouponRedemption>
 */
class CouponRedemptionRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, CouponRedemption::class);
    }

    /**
     * BR-06-10 / idempotency: a webhook redelivery for the same payment
     * record must not double-record a redemption or double-increment the
     * coupon's usage count — the same "check-first" idiom
     * `TokenLedgerService::purchase()` uses for I6.
     */
    public function findOneByPaymentRecord(PaymentRecord $paymentRecord): ?CouponRedemption
    {
        return $this->findOneBy(['paymentRecord' => $paymentRecord]);
    }

    /**
     * AC-06-27: "view usage details (the list of players who used it)."
     *
     * @return list<CouponRedemption>
     */
    public function findAllForCoupon(Coupon $coupon): array
    {
        /** @var list<CouponRedemption> $rows */
        $rows = $this->createQueryBuilder('cr')
            ->andWhere('cr.coupon = :coupon')
            ->setParameter('coupon', $coupon)
            ->orderBy('cr.redeemedAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * AC-06-26: per-coupon, all-time totals — discount given and revenue
     * generated (the final, discounted amount actually charged).
     *
     * @return array{uses: int, discountGiven: int, revenueGenerated: int}
     */
    public function totalsForCoupon(Coupon $coupon): array
    {
        $row = $this->createQueryBuilder('cr')
            ->select('COUNT(cr.id) AS uses')
            ->addSelect('COALESCE(SUM(cr.discountAmountMinorUnits), 0) AS discountGiven')
            ->addSelect('COALESCE(SUM(cr.finalPriceMinorUnits), 0) AS revenueGenerated')
            ->andWhere('cr.coupon = :coupon')
            ->setParameter('coupon', $coupon)
            ->getQuery()
            ->getSingleResult();

        return [
            'uses' => (int) $row['uses'],
            'discountGiven' => (int) $row['discountGiven'],
            'revenueGenerated' => (int) $row['revenueGenerated'],
        ];
    }

    /**
     * AC-06-28: "Analytics Summary (all coupons combined)" — this month's
     * uses, discount given, and revenue from coupon users.
     *
     * @return array{uses: int, discountGiven: int, revenueGenerated: int}
     */
    public function totalsBetween(Trainer $trainer, \DateTimeImmutable $start, \DateTimeImmutable $end): array
    {
        $row = $this->createQueryBuilder('cr')
            ->select('COUNT(cr.id) AS uses')
            ->addSelect('COALESCE(SUM(cr.discountAmountMinorUnits), 0) AS discountGiven')
            ->addSelect('COALESCE(SUM(cr.finalPriceMinorUnits), 0) AS revenueGenerated')
            ->andWhere('cr.trainer = :trainer')
            ->andWhere('cr.redeemedAt >= :start')
            ->andWhere('cr.redeemedAt < :end')
            ->setParameter('trainer', $trainer)
            ->setParameter('start', $start)
            ->setParameter('end', $end)
            ->getQuery()
            ->getSingleResult();

        return [
            'uses' => (int) $row['uses'],
            'discountGiven' => (int) $row['discountGiven'],
            'revenueGenerated' => (int) $row['revenueGenerated'],
        ];
    }

    /**
     * Q-06.10 (this codebase's own operational reading of "new_players_only"
     * — see `CouponEligibilityChecker`'s own docblock): has this player ever
     * redeemed any coupon with this trainer before?
     */
    public function hasAnyRedemptionForPlayer(Trainer $trainer, PlayerProfile $player): bool
    {
        $count = $this->createQueryBuilder('cr')
            ->select('COUNT(cr.id)')
            ->andWhere('cr.trainer = :trainer')
            ->andWhere('cr.player = :player')
            ->setParameter('trainer', $trainer)
            ->setParameter('player', $player)
            ->setMaxResults(1)
            ->getQuery()
            ->getSingleScalarResult();

        return (int) $count > 0;
    }

    public function add(CouponRedemption $redemption): void
    {
        $this->getEntityManager()->persist($redemption);
    }
}
