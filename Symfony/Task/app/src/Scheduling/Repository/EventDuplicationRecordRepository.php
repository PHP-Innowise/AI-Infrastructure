<?php

declare(strict_types=1);

namespace App\Scheduling\Repository;

use App\Scheduling\Entity\Event;
use App\Scheduling\Entity\EventDuplicationRecord;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<EventDuplicationRecord>
 */
class EventDuplicationRecordRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, EventDuplicationRecord::class);
    }

    /**
     * BR-02-19: the analytics reference from an original event to whatever
     * it has been duplicated into.
     *
     * @return list<EventDuplicationRecord>
     */
    public function findByOriginalEvent(Event $original): array
    {
        /** @var list<EventDuplicationRecord> $rows */
        $rows = $this->createQueryBuilder('r')
            ->andWhere('r.originalEvent = :original')
            ->setParameter('original', $original)
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function add(EventDuplicationRecord $record): void
    {
        $this->getEntityManager()->persist($record);
    }
}
