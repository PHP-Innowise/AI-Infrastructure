<?php

declare(strict_types=1);

namespace App\Billing\Repository;

use App\Billing\Entity\EntitlementCoverage;
use App\Identity\Entity\PlayerProfile;
use App\Scheduling\Entity\Rsvp;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<EntitlementCoverage>
 */
class EntitlementCoverageRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, EntitlementCoverage::class);
    }

    /**
     * BR-05-14/AC-02-63: "1 advance booking per day" — does $player already
     * hold an entitlement-covered RSVP for an event whose start falls on
     * the given calendar date (trainer timezone), joined through
     * `Rsvp`/`Event` exactly as
     * `RsvpRepository::findConfirmedForPlayerOnDate()`'s own docblock
     * anticipates. Only a CONFIRMED rsvp counts — a canceled one no longer
     * occupies the day.
     */
    public function existsConfirmedForPlayerOnDate(PlayerProfile $player, \DateTimeImmutable $dayStart, \DateTimeImmutable $dayEnd): bool
    {
        $count = $this->createQueryBuilder('c')
            ->select('COUNT(c.id)')
            ->join('c.rsvp', 'r')
            ->join('r.event', 'e')
            ->andWhere('r.player = :player')
            ->andWhere('r.status = :confirmed')
            ->andWhere('e.startsAt >= :dayStart')
            ->andWhere('e.startsAt < :dayEnd')
            ->setParameter('player', $player)
            ->setParameter('confirmed', Rsvp::STATUS_CONFIRMED)
            ->setParameter('dayStart', $dayStart)
            ->setParameter('dayEnd', $dayEnd)
            ->getQuery()
            ->getSingleScalarResult();

        return ((int) $count) > 0;
    }

    public function add(EntitlementCoverage $coverage): void
    {
        $this->getEntityManager()->persist($coverage);
    }
}
