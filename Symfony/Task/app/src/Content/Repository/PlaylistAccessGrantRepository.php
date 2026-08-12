<?php

declare(strict_types=1);

namespace App\Content\Repository;

use App\Content\Entity\Playlist;
use App\Content\Entity\PlaylistAccessGrant;
use App\Identity\Entity\PlayerProfile;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<PlaylistAccessGrant>
 */
class PlaylistAccessGrantRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, PlaylistAccessGrant::class);
    }

    /**
     * BR-04-7: "does this player have access" — the paywall's hot read,
     * checked on every locked-content render.
     */
    public function findOneByPlaylistAndPlayer(Playlist $playlist, PlayerProfile $player): ?PlaylistAccessGrant
    {
        return $this->findOneBy(['playlist' => $playlist, 'player' => $player]);
    }

    public function add(PlaylistAccessGrant $grant): void
    {
        $this->getEntityManager()->persist($grant);
    }
}
