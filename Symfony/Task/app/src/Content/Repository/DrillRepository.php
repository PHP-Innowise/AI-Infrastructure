<?php

declare(strict_types=1);

namespace App\Content\Repository;

use App\Content\Entity\Drill;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * The Drill Database's own repository — "My Drills" and public
 * search/filter/discovery (US-04.02/US-04.03). No `#[TrainerScoped]` (see
 * `ContentItem`'s own docblock) — every method states its own tenant
 * predicate explicitly.
 *
 * @extends ServiceEntityRepository<Drill>
 */
class DrillRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, Drill::class);
    }

    public function findOwnById(Trainer $trainer, int $id): ?Drill
    {
        return $this->createQueryBuilder('d')
            ->andWhere('d.id = :id')
            ->andWhere('IDENTITY(d.trainer) = :trainerId')
            ->andWhere('d.deletedAt IS NULL')
            ->setParameter('id', $id)
            ->setParameter('trainerId', $trainer->getId())
            ->getQuery()
            ->getOneOrNullResult();
    }

    public function findAccessible(Trainer $trainer, int $id): ?Drill
    {
        return $this->createQueryBuilder('d')
            ->andWhere('d.id = :id')
            ->andWhere('IDENTITY(d.trainer) = :trainerId OR d.everPublishedAt IS NOT NULL')
            ->andWhere('d.deletedAt IS NULL')
            ->setParameter('id', $id)
            ->setParameter('trainerId', $trainer->getId())
            ->getQuery()
            ->getOneOrNullResult();
    }

    /**
     * AC-04-2/AC-04-4: "My Drills" — this trainer's own drill database.
     *
     * @return list<Drill>
     */
    public function findOwnForActiveTenant(Trainer $trainer): array
    {
        /** @var list<Drill> $rows */
        $rows = $this->createQueryBuilder('d')
            ->andWhere('IDENTITY(d.trainer) = :trainerId')
            ->andWhere('d.deletedAt IS NULL')
            ->setParameter('trainerId', $trainer->getId())
            ->orderBy('d.createdAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * AC-04-7/9, BR-04-19: "Public Drills" search/filter, excluding the
     * searching trainer's own (already visible under "My Drills" — the
     * epic's own user scenario frames discovery as finding drills "from
     * other trainers"). Filters combine with AND logic (AC-04-9); each is
     * applied only when supplied.
     *
     * Free-text `$query` matching against `tags` (a native `TEXT[]` column)
     * is native SQL, not DQL — Postgres's `LIKE`/`ILIKE` operators are not
     * defined for array operands, and DQL has no portable "does any array
     * element match this substring" construct. Per
     * specs/architect-architecture.md "Deliberate exceptions... Native SQL
     * in reporting repositories": "permitted only because RLS covers native
     * SQL" — this runs over the ordinary RLS-bound connection (never the
     * `pp_crossing` connection), so the widened publication policy still
     * governs exactly which rows can be returned; this method only narrows
     * further. IDs are resolved first, then hydrated through the ORM so
     * every caller still receives real, identity-mapped `Drill` entities.
     *
     * @return list<Drill>
     */
    public function searchPublic(
        Trainer $excludingTrainer,
        ?string $query,
        ?string $category,
        ?string $difficulty,
        ?string $equipment,
        ?int $maxDurationMinutes,
    ): array {
        $conditions = [
            'ci.is_public = true',
            'ci.deleted_at IS NULL',
            'ci.trainer_id != :excludingTrainerId',
        ];
        $params = ['excludingTrainerId' => $excludingTrainer->getId()];

        // AC-04-9: matches drill name, description (instructions), and tags.
        if (null !== $query && '' !== trim($query)) {
            $conditions[] = '(ci.title ILIKE :query OR ci.instructions ILIKE :query OR EXISTS (SELECT 1 FROM unnest(ci.tags) t WHERE t ILIKE :query))';
            $params['query'] = '%'.$query.'%';
        }

        if (null !== $category && '' !== $category) {
            $conditions[] = ':category = ANY(dd.categories)';
            $params['category'] = $category;
        }

        if (null !== $difficulty && '' !== $difficulty) {
            $conditions[] = 'dd.difficulty_level = :difficulty';
            $params['difficulty'] = $difficulty;
        }

        if (null !== $equipment && '' !== $equipment) {
            $conditions[] = ':equipment = ANY(dd.equipment)';
            $params['equipment'] = $equipment;
        }

        if (null !== $maxDurationMinutes) {
            $conditions[] = '(dd.duration_min_minutes IS NULL OR dd.duration_min_minutes <= :maxDuration)';
            $params['maxDuration'] = $maxDurationMinutes;
        }

        $ids = array_map(
            'intval',
            $this->getEntityManager()->getConnection()->fetchFirstColumn(
                sprintf(
                    'SELECT ci.id FROM content_item ci JOIN drill_detail dd ON dd.id = ci.id WHERE %s ORDER BY ci.created_at DESC',
                    implode(' AND ', $conditions),
                ),
                $params,
            ),
        );

        if ([] === $ids) {
            return [];
        }

        /** @var list<Drill> $rows */
        $rows = $this->createQueryBuilder('d')
            ->andWhere('d.id IN (:ids)')
            ->setParameter('ids', $ids)
            ->getQuery()
            ->getResult();

        // Native query already established the order (newest first); the
        // ORM's IN() hydration does not preserve it.
        usort($rows, static fn (Drill $a, Drill $b): int => array_search((int) $b->getId(), $ids, true) <=> array_search((int) $a->getId(), $ids, true));

        return $rows;
    }

    public function add(Drill $drill): void
    {
        $this->getEntityManager()->persist($drill);
    }
}
