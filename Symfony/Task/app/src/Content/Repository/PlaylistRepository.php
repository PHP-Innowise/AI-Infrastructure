<?php

declare(strict_types=1);

namespace App\Content\Repository;

use App\Content\Entity\Playlist;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * `Playlist` carries no `#[TrainerScoped]` attribute (see that entity's own
 * docblock), so Doctrine's automatic tenant filter never applies here —
 * every method below states its own tenant predicate explicitly, which is
 * the "defense in depth, mirroring why Layer 2 is kept alongside Layer 5"
 * `specs/database-designer-schema.md` names for exactly this repository.
 * PostgreSQL Row-Level Security (layer 5) remains the actual authority
 * underneath every query here regardless of which predicate is written.
 *
 * @extends ServiceEntityRepository<Playlist>
 */
class PlaylistRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, Playlist::class);
    }

    /**
     * Strict, OWN-tenant-only lookup — for every write path (edit, delete,
     * assign, publish-toggle). Deliberately narrower than
     * `findAccessible()`: BR-04-11/the security design's "the publication
     * exception is a read-predicate widening, never a write one" means a
     * trainer must never be able to edit another trainer's playlist merely
     * because it is published.
     */
    public function findOwnById(Trainer $trainer, int $id): ?Playlist
    {
        return $this->createQueryBuilder('p')
            ->andWhere('p.id = :id')
            ->andWhere('IDENTITY(p.trainer) = :trainerId')
            ->andWhere('p.deletedAt IS NULL')
            ->setParameter('id', $id)
            ->setParameter('trainerId', $trainer->getId())
            ->getQuery()
            ->getOneOrNullResult();
    }

    /**
     * Widened lookup: this trainer's own row, OR any row that has ever been
     * published — mirrors the RLS SELECT policy's own predicate exactly
     * (`specs/database-designer-schema.md` "Widened policy"). This is the
     * one repository method that legitimately needs it, matching the schema
     * doc's named `ContentItemRepository::findAccessible()` extension point.
     */
    public function findAccessible(Trainer $trainer, int $id): ?Playlist
    {
        return $this->createQueryBuilder('p')
            ->andWhere('p.id = :id')
            ->andWhere('IDENTITY(p.trainer) = :trainerId OR p.everPublishedAt IS NOT NULL')
            ->andWhere('p.deletedAt IS NULL')
            ->setParameter('id', $id)
            ->setParameter('trainerId', $trainer->getId())
            ->getQuery()
            ->getOneOrNullResult();
    }

    /**
     * AC-04-1/AC-04-11: the trainer's own Learn/Practice tab list — every
     * one of their own playlists (public, private, or coaches-only alike;
     * this is the OWNER's admin view, not the player-facing library).
     *
     * @return list<Playlist>
     */
    public function findAllForActiveTenant(Trainer $trainer, string $pillar): array
    {
        /** @var list<Playlist> $rows */
        $rows = $this->createQueryBuilder('p')
            ->andWhere('IDENTITY(p.trainer) = :trainerId')
            ->andWhere('p.pillar = :pillar')
            ->andWhere('p.deletedAt IS NULL')
            ->setParameter('trainerId', $trainer->getId())
            ->setParameter('pillar', $pillar)
            ->orderBy('p.createdAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * AC-04-20/21: the player portal's Content Library for the CURRENT
     * trainer context only (BR-04-10) — this trainer's own playlists, minus
     * the ones marked coaches-only (AC-04-41: a player must never see
     * coach-only content). Cross-tenant published content from OTHER
     * trainers is never shown here — a player's library is per-trainer, not
     * a cross-tenant public feed (BR-04-10's "separated views").
     *
     * @return list<Playlist>
     */
    public function findPlayerLibraryForActiveTenant(Trainer $trainer, string $pillar): array
    {
        /** @var list<Playlist> $rows */
        $rows = $this->createQueryBuilder('p')
            ->andWhere('IDENTITY(p.trainer) = :trainerId')
            ->andWhere('p.pillar = :pillar')
            ->andWhere('p.audience = :audience')
            ->andWhere('p.deletedAt IS NULL')
            ->setParameter('trainerId', $trainer->getId())
            ->setParameter('pillar', $pillar)
            ->setParameter('audience', Playlist::AUDIENCE_PLAYERS_AND_COACHES)
            ->orderBy('p.createdAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * AC-04-37: total playlists created, across every trainer — read via
     * the crossing connection by `CrossTenantReadService` in production; this
     * method exists for completeness/tests operating within one tenant's own
     * RLS-bound connection and is not itself the cross-tenant source.
     */
    public function countForActiveTenant(Trainer $trainer): int
    {
        $count = $this->createQueryBuilder('p')
            ->select('COUNT(p.id)')
            ->andWhere('IDENTITY(p.trainer) = :trainerId')
            ->andWhere('p.deletedAt IS NULL')
            ->setParameter('trainerId', $trainer->getId())
            ->getQuery()
            ->getSingleScalarResult();

        return (int) $count;
    }

    public function add(Playlist $playlist): void
    {
        $this->getEntityManager()->persist($playlist);
    }
}
