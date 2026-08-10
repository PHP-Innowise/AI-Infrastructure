<?php

declare(strict_types=1);

namespace App\Content\Repository;

use App\Content\Entity\Playlist;
use App\Content\Entity\PlaylistAssignment;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<PlaylistAssignment>
 */
class PlaylistAssignmentRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, PlaylistAssignment::class);
    }

    /**
     * AC-04-15/16: every assignment ACT ever made for this playlist — the
     * raw rows `ContentAssignmentResolver` resolves against current
     * membership, never itself a per-player list.
     *
     * @return list<PlaylistAssignment>
     */
    public function findForPlaylist(Playlist $playlist): array
    {
        /** @var list<PlaylistAssignment> $rows */
        $rows = $this->createQueryBuilder('a')
            ->andWhere('a.playlist = :playlist')
            ->setParameter('playlist', $playlist)
            ->orderBy('a.assignedAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * Every assignment row in the active tenant — small-N by construction
     * (one row per assignment ACT, not per covered player), so
     * `ContentAssignmentResolver` fetches this once and resolves coverage
     * in PHP rather than issuing a bespoke query per player, mirroring this
     * codebase's existing "fetch the tenant's rows, filter in memory"
     * convention for similarly-scaled reads (e.g. `EventRepository::
     * findAllForActiveTenant()`).
     *
     * @return list<PlaylistAssignment>
     */
    public function findAllForActiveTenant(Trainer $trainer): array
    {
        /** @var list<PlaylistAssignment> $rows */
        $rows = $this->createQueryBuilder('a')
            ->andWhere('IDENTITY(a.trainer) = :trainerId')
            ->setParameter('trainerId', $trainer->getId())
            ->orderBy('a.assignedAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function add(PlaylistAssignment $assignment): void
    {
        $this->getEntityManager()->persist($assignment);
    }
}
