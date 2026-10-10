<?php

declare(strict_types=1);

namespace App\Identity\Repository;

use App\Identity\Entity\AvailabilityWindow;
use App\Identity\Entity\CoachMembership;
use App\Identity\Entity\PlayerProfile;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<AvailabilityWindow>
 */
class AvailabilityWindowRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, AvailabilityWindow::class);
    }

    /**
     * @return list<AvailabilityWindow>
     */
    public function findForCoach(CoachMembership $coachMembership): array
    {
        /** @var list<AvailabilityWindow> $rows */
        $rows = $this->createQueryBuilder('w')
            ->andWhere('w.coachMembership = :coach')
            ->setParameter('coach', $coachMembership)
            ->orderBy('w.dayOfWeek', 'ASC')
            ->addOrderBy('w.startTime', 'ASC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * @return list<AvailabilityWindow>
     */
    public function findForPlayer(PlayerProfile $player): array
    {
        /** @var list<AvailabilityWindow> $rows */
        $rows = $this->createQueryBuilder('w')
            ->andWhere('w.player = :player')
            ->setParameter('player', $player)
            ->orderBy('w.dayOfWeek', 'ASC')
            ->addOrderBy('w.startTime', 'ASC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * AC-01-45: filter players by availability at a selected day/time, within
     * the active tenant.
     *
     * @return list<int> player profile ids
     */
    public function findPlayerIdsAvailableAt(int $dayOfWeek, \DateTimeImmutable $at): array
    {
        /** @var list<array{playerId: int|string}> $rows */
        $rows = $this->createQueryBuilder('w')
            ->select('IDENTITY(w.player) AS playerId')
            ->andWhere('w.ownerType = :owner')
            ->andWhere('w.dayOfWeek = :day')
            ->andWhere('w.isAvailable = true')
            ->andWhere('w.startTime <= :at')
            ->andWhere('w.endTime > :at')
            ->setParameter('owner', AvailabilityWindow::OWNER_PLAYER)
            ->setParameter('day', $dayOfWeek)
            ->setParameter('at', $at)
            ->getQuery()
            ->getArrayResult();

        return array_values(array_unique(array_map(static fn (array $r): int => (int) $r['playerId'], $rows)));
    }

    public function removeAllForCoach(CoachMembership $coachMembership): void
    {
        foreach ($this->findForCoach($coachMembership) as $window) {
            $this->getEntityManager()->remove($window);
        }
    }

    public function removeAllForPlayer(PlayerProfile $player): void
    {
        foreach ($this->findForPlayer($player) as $window) {
            $this->getEntityManager()->remove($window);
        }
    }

    public function add(AvailabilityWindow $window): void
    {
        $this->getEntityManager()->persist($window);
    }
}
