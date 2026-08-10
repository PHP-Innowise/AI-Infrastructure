<?php

declare(strict_types=1);

namespace App\Content\Repository;

use App\Content\Entity\ContentItem;
use App\Content\Entity\ContentProgress;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<ContentProgress>
 */
class ContentProgressRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, ContentProgress::class);
    }

    public function findOneByPlayerAndContentItem(PlayerProfile $player, ContentItem $contentItem): ?ContentProgress
    {
        return $this->findOneBy(['player' => $player, 'contentItem' => $contentItem]);
    }

    /**
     * AC-04-22..24/27..30: every content item this player has ANY progress
     * on, within the active tenant — completion checkmarks, watch time
     * totals, and the Progress dashboard are all derived from this set.
     * Used both for a player's own dashboard and for a trainer's read of a
     * player's progress from the CRM (AC-04-30) — the same query, two
     * different callers.
     *
     * @return list<ContentProgress>
     */
    public function findForPlayer(Trainer $trainer, PlayerProfile $player): array
    {
        /** @var list<ContentProgress> $rows */
        $rows = $this->createQueryBuilder('cp')
            ->andWhere('IDENTITY(cp.trainer) = :trainerId')
            ->andWhere('cp.player = :player')
            ->setParameter('trainerId', $trainer->getId())
            ->setParameter('player', $player)
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function add(ContentProgress $progress): void
    {
        $this->getEntityManager()->persist($progress);
    }
}
