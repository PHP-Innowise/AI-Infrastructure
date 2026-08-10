<?php

declare(strict_types=1);

namespace App\Platform\Repository;

use App\Platform\Entity\AuditLogEntry;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<AuditLogEntry>
 */
class AuditLogEntryRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, AuditLogEntry::class);
    }

    /**
     * AC-07-29..32: chronological, optionally filtered by action type and/or
     * subject. Global by design — the audit log spans every tenant.
     *
     * @return list<AuditLogEntry>
     */
    public function search(?string $actionType = null, ?int $relatedTrainerId = null, int $limit = 100): array
    {
        $qb = $this->createQueryBuilder('a')
            ->orderBy('a.occurredAt', 'DESC')
            ->setMaxResults($limit);

        if (null !== $actionType) {
            $qb->andWhere('a.actionType = :actionType')->setParameter('actionType', $actionType);
        }

        if (null !== $relatedTrainerId) {
            $qb->andWhere('IDENTITY(a.relatedTrainer) = :trainerId')->setParameter('trainerId', $relatedTrainerId);
        }

        /** @var list<AuditLogEntry> $rows */
        $rows = $qb->getQuery()->getResult();

        return $rows;
    }

    public function add(AuditLogEntry $entry): void
    {
        $this->getEntityManager()->persist($entry);
    }
}
