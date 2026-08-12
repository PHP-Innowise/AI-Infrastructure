<?php

declare(strict_types=1);

namespace App\Growth\Repository;

use App\Growth\Entity\Referral;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<Referral>
 */
class ReferralRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, Referral::class);
    }

    /**
     * BR-06-3: a referee is credited to exactly one referral, ever.
     */
    public function findOneByRefereePlayer(PlayerProfile $refereePlayer): ?Referral
    {
        return $this->findOneBy(['refereePlayer' => $refereePlayer]);
    }

    public function add(Referral $referral): void
    {
        $this->getEntityManager()->persist($referral);
    }

    /**
     * AC-06-2: "You've referred 3 players (2 converted)" — one player's own
     * stats for their referral link with this trainer.
     *
     * @return array{total: int, converted: int}
     */
    public function statsForReferrer(Trainer $trainer, PlayerProfile $referrer): array
    {
        $row = $this->createQueryBuilder('r')
            ->select('COUNT(r.id) AS total')
            ->addSelect('SUM(CASE WHEN r.status = :status THEN 1 ELSE 0 END) AS converted')
            ->andWhere('r.trainer = :trainer')
            ->andWhere('r.referrerPlayer = :referrer')
            ->setParameter('trainer', $trainer)
            ->setParameter('referrer', $referrer)
            ->setParameter('status', Referral::STATUS_CONVERTED)
            ->getQuery()
            ->getSingleResult();

        return [
            'total' => (int) $row['total'],
            'converted' => (int) ($row['converted'] ?? 0),
        ];
    }

    /**
     * AC-06-14: "total referrals this month" — counted by registration
     * date, the moment the Referral row is actually created.
     */
    public function countRegisteredBetween(Trainer $trainer, \DateTimeImmutable $start, \DateTimeImmutable $end): int
    {
        return (int) $this->createQueryBuilder('r')
            ->select('COUNT(r.id)')
            ->andWhere('r.trainer = :trainer')
            ->andWhere('r.registeredAt >= :start')
            ->andWhere('r.registeredAt < :end')
            ->setParameter('trainer', $trainer)
            ->setParameter('start', $start)
            ->setParameter('end', $end)
            ->getQuery()
            ->getSingleScalarResult();
    }

    /**
     * AC-06-14: "total conversions this month" — counted by the moment the
     * referral actually converted (first_purchase_at), not registration
     * date; a friend who registered last month and converts this month
     * counts as this month's conversion.
     */
    public function countConvertedBetween(Trainer $trainer, \DateTimeImmutable $start, \DateTimeImmutable $end): int
    {
        return (int) $this->createQueryBuilder('r')
            ->select('COUNT(r.id)')
            ->andWhere('r.trainer = :trainer')
            ->andWhere('r.status = :status')
            ->andWhere('r.firstPurchaseAt >= :start')
            ->andWhere('r.firstPurchaseAt < :end')
            ->setParameter('trainer', $trainer)
            ->setParameter('status', Referral::STATUS_CONVERTED)
            ->setParameter('start', $start)
            ->setParameter('end', $end)
            ->getQuery()
            ->getSingleScalarResult();
    }

    /**
     * AC-06-14: "total referral revenue (the sum of first purchases from
     * referrals)" — stated with no "this month" qualifier (unlike its two
     * sibling metrics), so summed all-time. Reads through this entity's own
     * `firstPurchasePaymentRecord` relation — a same-module repository join
     * over Growth's own entity, not a cross-module service call.
     */
    public function sumFirstPurchaseRevenue(Trainer $trainer): int
    {
        $sum = $this->createQueryBuilder('r')
            ->select('SUM(pr.amountMinorUnits)')
            ->join('r.firstPurchasePaymentRecord', 'pr')
            ->andWhere('r.trainer = :trainer')
            ->andWhere('r.status = :status')
            ->setParameter('trainer', $trainer)
            ->setParameter('status', Referral::STATUS_CONVERTED)
            ->getQuery()
            ->getSingleScalarResult();

        return null !== $sum ? (int) $sum : 0;
    }

    /**
     * AC-06-15: Top Referrers leaderboard — per referrer, total referrals
     * sent, total conversions, and last referral date. Conversion rate and
     * ordering are computed by the caller (`ReferralDashboardService`) from
     * these two counts, matching `PlayerSegmentationRepository`'s own
     * "repository returns raw counts, service shapes the view model" split.
     *
     * @return list<array{referrerPlayerId: int, totalReferrals: int, totalConversions: int, lastReferralAt: \DateTimeImmutable}>
     */
    public function topReferrers(Trainer $trainer, int $limit): array
    {
        $rows = $this->createQueryBuilder('r')
            ->select('IDENTITY(r.referrerPlayer) AS referrerPlayerId')
            ->addSelect('COUNT(r.id) AS totalReferrals')
            ->addSelect('SUM(CASE WHEN r.status = :status THEN 1 ELSE 0 END) AS totalConversions')
            ->addSelect('MAX(r.registeredAt) AS lastReferralAt')
            ->andWhere('r.trainer = :trainer')
            ->setParameter('trainer', $trainer)
            ->setParameter('status', Referral::STATUS_CONVERTED)
            ->groupBy('r.referrerPlayer')
            ->orderBy('totalConversions', 'DESC')
            ->addOrderBy('totalReferrals', 'DESC')
            ->setMaxResults($limit)
            ->getQuery()
            ->getArrayResult();

        return array_values(array_map(static fn (array $row): array => [
            'referrerPlayerId' => (int) $row['referrerPlayerId'],
            'totalReferrals' => (int) $row['totalReferrals'],
            'totalConversions' => (int) $row['totalConversions'],
            'lastReferralAt' => $row['lastReferralAt'] instanceof \DateTimeImmutable
                ? $row['lastReferralAt']
                : new \DateTimeImmutable((string) $row['lastReferralAt']),
        ], $rows));
    }

    /**
     * `ReferralRewardSubscriber`'s own token-purchase case: a token
     * purchase funds the (payer account, trainer) balance, not any one
     * child specifically, so matching it to a referral means scanning
     * every still-pending referee under this trainer rather than resolving
     * one player directly — see that subscriber's own docblock.
     *
     * @return list<Referral>
     */
    public function findPendingForTrainer(Trainer $trainer): array
    {
        /** @var list<Referral> $rows */
        $rows = $this->createQueryBuilder('r')
            ->andWhere('r.trainer = :trainer')
            ->andWhere('r.status = :status')
            ->setParameter('trainer', $trainer)
            ->setParameter('status', Referral::STATUS_PENDING)
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * AC-06-16: Referral Activity Log, newest first.
     *
     * @return list<Referral>
     */
    public function activityLog(Trainer $trainer, int $limit = 50): array
    {
        return $this->createQueryBuilder('r')
            ->andWhere('r.trainer = :trainer')
            ->setParameter('trainer', $trainer)
            ->orderBy('r.registeredAt', 'DESC')
            ->setMaxResults($limit)
            ->getQuery()
            ->getResult();
    }
}
