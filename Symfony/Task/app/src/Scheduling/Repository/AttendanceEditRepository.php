<?php

declare(strict_types=1);

namespace App\Scheduling\Repository;

use App\Scheduling\Entity\AttendanceEdit;
use App\Scheduling\Entity\AttendanceRecord;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<AttendanceEdit>
 */
class AttendanceEditRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, AttendanceEdit::class);
    }

    /**
     * @return list<AttendanceEdit>
     */
    public function findForRecord(AttendanceRecord $record): array
    {
        /** @var list<AttendanceEdit> $rows */
        $rows = $this->createQueryBuilder('e')
            ->andWhere('e.attendanceRecord = :record')
            ->setParameter('record', $record)
            ->orderBy('e.editedAt', 'ASC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function add(AttendanceEdit $edit): void
    {
        $this->getEntityManager()->persist($edit);
    }
}
