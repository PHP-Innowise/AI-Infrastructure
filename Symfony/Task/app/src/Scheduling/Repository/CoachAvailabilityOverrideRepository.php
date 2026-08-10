<?php

declare(strict_types=1);

namespace App\Scheduling\Repository;

use App\Scheduling\Entity\CoachAvailabilityOverride;
use App\Scheduling\Entity\Event;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<CoachAvailabilityOverride>
 */
class CoachAvailabilityOverrideRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, CoachAvailabilityOverride::class);
    }

    /**
     * AC-02-9: the audit trail for a given event's conflict overrides.
     *
     * @return list<CoachAvailabilityOverride>
     */
    public function findForEvent(Event $event): array
    {
        /** @var list<CoachAvailabilityOverride> $rows */
        $rows = $this->createQueryBuilder('o')
            ->andWhere('o.event = :event')
            ->setParameter('event', $event)
            ->orderBy('o.createdAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function add(CoachAvailabilityOverride $override): void
    {
        $this->getEntityManager()->persist($override);
    }
}
