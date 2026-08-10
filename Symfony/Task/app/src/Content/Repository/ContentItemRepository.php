<?php

declare(strict_types=1);

namespace App\Content\Repository;

use App\Content\Entity\ContentItem;
use App\Content\Entity\Drill;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * Queries the base `ContentItem` class — Doctrine's joined-table inheritance
 * hydrates each row as its real subtype (`ContentItem` for `type = 'video'`,
 * `Drill` for `type = 'drill'`) transparently, so a lookup by id here
 * correctly returns a `Drill` instance when the id names one, with no
 * discriminator handling needed at the call site.
 *
 * No `#[TrainerScoped]` on the entity (see its own docblock) — every method
 * here states its own tenant predicate explicitly. See `PlaylistRepository`'s
 * own docblock for the full reasoning, identical here.
 *
 * @extends ServiceEntityRepository<ContentItem>
 */
class ContentItemRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, ContentItem::class);
    }

    public function findOwnById(Trainer $trainer, int $id): ?ContentItem
    {
        return $this->createQueryBuilder('c')
            ->andWhere('c.id = :id')
            ->andWhere('IDENTITY(c.trainer) = :trainerId')
            ->andWhere('c.deletedAt IS NULL')
            ->setParameter('id', $id)
            ->setParameter('trainerId', $trainer->getId())
            ->getQuery()
            ->getOneOrNullResult();
    }

    /**
     * `specs/database-designer-schema.md` "Repository methods worth naming
     * now": "applies the widened publication predicate explicitly in
     * DQL/SQL for the one code path that legitimately needs it outside
     * RLS's own enforcement (defense in depth, mirroring why Layer 2 is
     * kept alongside Layer 5)."
     */
    public function findAccessible(Trainer $trainer, int $id): ?ContentItem
    {
        return $this->createQueryBuilder('c')
            ->andWhere('c.id = :id')
            ->andWhere('IDENTITY(c.trainer) = :trainerId OR c.everPublishedAt IS NOT NULL')
            ->andWhere('c.deletedAt IS NULL')
            ->setParameter('id', $id)
            ->setParameter('trainerId', $trainer->getId())
            ->getQuery()
            ->getOneOrNullResult();
    }

    /**
     * AC-04-1/AC-04-2: video items available to add to a Learn playlist —
     * "My Videos", this trainer's own, non-deleted content-item rows of type
     * video (drills are queried through `DrillRepository`, which has its
     * own richer filter set for the Drill Database).
     *
     * @return list<ContentItem>
     */
    public function findOwnVideosForActiveTenant(Trainer $trainer): array
    {
        /** @var list<ContentItem> $rows */
        $rows = $this->createQueryBuilder('c')
            ->andWhere('IDENTITY(c.trainer) = :trainerId')
            ->andWhere('c.deletedAt IS NULL')
            ->andWhere('c NOT INSTANCE OF '.Drill::class)
            ->setParameter('trainerId', $trainer->getId())
            ->orderBy('c.createdAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function add(ContentItem $item): void
    {
        $this->getEntityManager()->persist($item);
    }
}
