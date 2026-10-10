<?php

declare(strict_types=1);

namespace App\Content\Repository;

use App\Content\Entity\ContentItem;
use App\Content\Entity\ContentUsage;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<ContentUsage>
 */
class ContentUsageRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, ContentUsage::class);
    }

    public function findOneByTrainerAndContentItem(Trainer $trainer, ContentItem $contentItem): ?ContentUsage
    {
        return $this->findOneBy(['trainer' => $trainer, 'contentItem' => $contentItem]);
    }

    /**
     * BR-04-12: upserts the (reusing trainer, content item) usage row the
     * first time this trainer adds the item to any of their own playlists —
     * a no-op on every subsequent addition. Called by `PlaylistService`
     * whenever a `PlaylistItem` referencing another trainer's content is
     * created; harmless (and skipped) when the item is the caller's own.
     */
    public function recordFirstUse(Trainer $reusingTrainer, ContentItem $contentItem): void
    {
        if ($reusingTrainer->getId() === $contentItem->getTrainer()->getId()) {
            return;
        }

        if (null !== $this->findOneByTrainerAndContentItem($reusingTrainer, $contentItem)) {
            return;
        }

        $this->getEntityManager()->persist(new ContentUsage($reusingTrainer, $contentItem));
    }
}
