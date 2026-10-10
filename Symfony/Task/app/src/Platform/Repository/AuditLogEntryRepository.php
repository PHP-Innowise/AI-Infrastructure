<?php

declare(strict_types=1);

namespace App\Platform\Repository;

use App\Platform\Entity\AuditLogEntry;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\ORM\QueryBuilder;
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
     * AC-07-29..32: chronological, optionally filtered by action type,
     * subject (trainer or related-trainer id), a free-text subject search
     * (AC-07-32 "searched by subject — user or trainer name"), and a date
     * range. Global by design — the audit log spans every tenant, so this
     * never needs `CrossTenantReadService`; `account`/`trainer` are global
     * tables too.
     *
     * @return list<AuditLogEntry>
     */
    public function search(
        ?string $actionType = null,
        ?int $relatedTrainerId = null,
        int $limit = 100,
        int $offset = 0,
        ?\DateTimeImmutable $occurredFrom = null,
        ?\DateTimeImmutable $occurredTo = null,
        ?string $subjectQuery = null,
    ): array {
        $qb = $this->filteredQuery($actionType, $relatedTrainerId, $occurredFrom, $occurredTo, $subjectQuery)
            ->orderBy('a.occurredAt', 'DESC')
            ->setMaxResults($limit)
            ->setFirstResult($offset);

        /** @var list<AuditLogEntry> $rows */
        $rows = $qb->getQuery()->getResult();

        return $rows;
    }

    /**
     * Same filters as search(), for pagination (AC-07-29's page navigation
     * mirrors every other Administration list's own 50/page convention).
     * Named `countMatching()`, not `count()` — `ServiceEntityRepository`
     * already declares `count(array $criteria = []): int`, an incompatible
     * signature this class must not collide with.
     */
    public function countMatching(
        ?string $actionType = null,
        ?int $relatedTrainerId = null,
        ?\DateTimeImmutable $occurredFrom = null,
        ?\DateTimeImmutable $occurredTo = null,
        ?string $subjectQuery = null,
    ): int {
        $qb = $this->filteredQuery($actionType, $relatedTrainerId, $occurredFrom, $occurredTo, $subjectQuery)
            ->select('COUNT(a.id)');

        return (int) $qb->getQuery()->getSingleScalarResult();
    }

    public function add(AuditLogEntry $entry): void
    {
        $this->getEntityManager()->persist($entry);
    }

    private function filteredQuery(
        ?string $actionType,
        ?int $relatedTrainerId,
        ?\DateTimeImmutable $occurredFrom,
        ?\DateTimeImmutable $occurredTo,
        ?string $subjectQuery,
    ): QueryBuilder {
        $qb = $this->createQueryBuilder('a');

        if (null !== $actionType) {
            $qb->andWhere('a.actionType = :actionType')->setParameter('actionType', $actionType);
        }

        if (null !== $relatedTrainerId) {
            $qb->andWhere('IDENTITY(a.relatedTrainer) = :trainerId')->setParameter('trainerId', $relatedTrainerId);
        }

        if (null !== $occurredFrom) {
            $qb->andWhere('a.occurredAt >= :occurredFrom')->setParameter('occurredFrom', $occurredFrom);
        }

        if (null !== $occurredTo) {
            $qb->andWhere('a.occurredAt < :occurredTo')->setParameter('occurredTo', $occurredTo);
        }

        if (null !== $subjectQuery && '' !== trim($subjectQuery)) {
            // `subject_id`/`subject_type` are a deliberate soft reference
            // (AuditLogEntry's own docblock) with no FK to join through, so
            // "search by subject name" is resolved in two small native
            // queries against the global `trainer`/`account` tables first,
            // then folded into this DQL query as plain id lists — avoiding
            // an arbitrary (non-association) DQL join for a pattern used
            // nowhere else in this codebase.
            $trainerIds = $this->trainerIdsMatching($subjectQuery);
            $accountIds = $this->accountIdsMatching($subjectQuery);

            $or = $qb->expr()->orX();

            // A related trainer matching by name — covers every entry type
            // that names one (trainer_created, feature_toggled, fee edits,
            // scheduling-conflict overrides, etc.), regardless of subjectType.
            if ([] !== $trainerIds) {
                $or->add($qb->expr()->in('IDENTITY(a.relatedTrainer)', ':trainerIds'));
                $qb->setParameter('trainerIds', $trainerIds);
            }

            // A subject that IS a trainer (subjectType = 'trainer') matching
            // by name, for the rare entry with no relatedTrainer set.
            if ([] !== $trainerIds) {
                $or->add($qb->expr()->andX(
                    $qb->expr()->eq('a.subjectType', ':subjectTypeTrainer'),
                    $qb->expr()->in('a.subjectId', ':trainerIds'),
                ));
                $qb->setParameter('subjectTypeTrainer', 'trainer');
                $qb->setParameter('trainerIds', $trainerIds);
            }

            // A subject that is a user account, matching by email or name.
            if ([] !== $accountIds) {
                $or->add($qb->expr()->andX(
                    $qb->expr()->eq('a.subjectType', ':subjectTypeAccount'),
                    $qb->expr()->in('a.subjectId', ':accountIds'),
                ));
                $qb->setParameter('subjectTypeAccount', 'account');
                $qb->setParameter('accountIds', $accountIds);
            }

            // No match on either table: the filter must exclude everything,
            // never silently ignore itself and return an unfiltered page.
            $qb->andWhere($or->count() > 0 ? $or : '1 = 0');
        }

        return $qb;
    }

    /**
     * @return list<int>
     */
    private function trainerIdsMatching(string $query): array
    {
        $rows = $this->getEntityManager()->getConnection()->fetchFirstColumn(
            'SELECT id FROM trainer WHERE business_name ILIKE :q',
            ['q' => '%'.$query.'%'],
        );

        return array_map('intval', $rows);
    }

    /**
     * @return list<int>
     */
    private function accountIdsMatching(string $query): array
    {
        $rows = $this->getEntityManager()->getConnection()->fetchFirstColumn(
            <<<'SQL'
                SELECT a.id
                FROM account a
                LEFT JOIN account_profile ap ON ap.account_id = a.id
                WHERE a.email::text ILIKE :q OR ap.first_name ILIKE :q OR ap.last_name ILIKE :q
                SQL,
            ['q' => '%'.$query.'%'],
        );

        return array_map('intval', $rows);
    }
}
