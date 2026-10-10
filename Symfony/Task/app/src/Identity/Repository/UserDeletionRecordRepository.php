<?php

declare(strict_types=1);

namespace App\Identity\Repository;

use App\Identity\Entity\UserDeletionRecord;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<UserDeletionRecord>
 */
class UserDeletionRecordRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, UserDeletionRecord::class);
    }

    public function add(UserDeletionRecord $record): void
    {
        $this->getEntityManager()->persist($record);
    }
}
