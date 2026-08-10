<?php

declare(strict_types=1);

namespace App\Scheduling\Repository;

use App\Identity\Entity\PlayerProfile;
use App\Scheduling\Entity\AttendanceRecord;
use App\Scheduling\Entity\Event;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<AttendanceRecord>
 */
class AttendanceRecordRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, AttendanceRecord::class);
    }

    public function findOneByEventAndPlayer(Event $event, PlayerProfile $player): ?AttendanceRecord
    {
        return $this->findOneBy(['event' => $event, 'player' => $player]);
    }

    /**
     * @return list<AttendanceRecord>
     */
    public function findForEvent(Event $event): array
    {
        /** @var list<AttendanceRecord> $rows */
        $rows = $this->createQueryBuilder('a')
            ->andWhere('a.event = :event')
            ->setParameter('event', $event)
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function add(AttendanceRecord $record): void
    {
        $this->getEntityManager()->persist($record);
    }
}
