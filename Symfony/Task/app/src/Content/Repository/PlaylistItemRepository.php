<?php

declare(strict_types=1);

namespace App\Content\Repository;

use App\Content\Entity\ContentItem;
use App\Content\Entity\Playlist;
use App\Content\Entity\PlaylistItem;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<PlaylistItem>
 */
class PlaylistItemRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, PlaylistItem::class);
    }

    /**
     * AC-04-22/23/24: a playlist's items in playback order.
     *
     * @return list<PlaylistItem>
     */
    public function findForPlaylist(Playlist $playlist): array
    {
        /** @var list<PlaylistItem> $rows */
        $rows = $this->createQueryBuilder('pi')
            ->andWhere('pi.playlist = :playlist')
            ->setParameter('playlist', $playlist)
            ->orderBy('pi.sequenceOrder', 'ASC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function nextSequenceOrder(Playlist $playlist): int
    {
        $max = $this->createQueryBuilder('pi')
            ->select('MAX(pi.sequenceOrder)')
            ->andWhere('pi.playlist = :playlist')
            ->setParameter('playlist', $playlist)
            ->getQuery()
            ->getSingleScalarResult();

        return null === $max ? 1 : ((int) $max + 1);
    }

    /**
     * AC-04-35: "This drill is used in [N] playlists" — scoped to the
     * DELETING trainer's own tenant only (their own playlists that
     * reference it), never a crossing read; other trainers' usage of the
     * same content item is a separate, genuinely cross-tenant question
     * (`ContentUsageRepository`).
     */
    public function countPlaylistsUsingContentItem(Trainer $trainer, ContentItem $contentItem): int
    {
        $count = $this->createQueryBuilder('pi')
            ->select('COUNT(DISTINCT pi.playlist)')
            ->andWhere('pi.contentItem = :contentItem')
            ->andWhere('IDENTITY(pi.trainer) = :trainerId')
            ->setParameter('contentItem', $contentItem)
            ->setParameter('trainerId', $trainer->getId())
            ->getQuery()
            ->getSingleScalarResult();

        return (int) $count;
    }

    /**
     * BR-04-12: which of THIS tenant's own playlists reference a given
     * content item — regardless of whether the item's own `trainer` (its
     * original creator, BR-04-11) is this tenant or another one entirely.
     * This is the reference-resolution query `ContentItemVoter` needs: a
     * reused public item has no `trainer_id` matching the reusing tenant at
     * all, only a `PlaylistItem` row pointing at it.
     *
     * @return list<Playlist>
     */
    public function findPlaylistsForContentItemInTenant(Trainer $trainer, ContentItem $contentItem): array
    {
        // DQL requires the root alias (pi) in the SELECT list alongside a
        // joined one (p) — "SELECT p FROM PlaylistItem pi JOIN pi.playlist
        // p" alone is rejected ("Cannot select entity through
        // identification variables without choosing at least one root
        // entity alias"). Selecting both and dereferencing in PHP also
        // sidesteps DISTINCT-over-a-joined-entity semantics entirely;
        // de-duplication happens here instead.
        /** @var list<PlaylistItem> $rows */
        $rows = $this->createQueryBuilder('pi')
            ->addSelect('p')
            ->join('pi.playlist', 'p')
            ->andWhere('pi.contentItem = :contentItem')
            ->andWhere('IDENTITY(pi.trainer) = :trainerId')
            ->andWhere('p.deletedAt IS NULL')
            ->setParameter('contentItem', $contentItem)
            ->setParameter('trainerId', $trainer->getId())
            ->getQuery()
            ->getResult();

        $playlists = [];
        foreach ($rows as $row) {
            $playlists[(int) $row->getPlaylist()->getId()] = $row->getPlaylist();
        }

        return array_values($playlists);
    }

    public function add(PlaylistItem $item): void
    {
        $this->getEntityManager()->persist($item);
    }

    public function remove(PlaylistItem $item): void
    {
        $this->getEntityManager()->remove($item);
    }
}
