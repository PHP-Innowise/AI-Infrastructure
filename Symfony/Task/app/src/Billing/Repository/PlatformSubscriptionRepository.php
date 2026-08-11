<?php

declare(strict_types=1);

namespace App\Billing\Repository;

use App\Billing\Entity\PlatformSubscription;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * Global — see PlatformSubscription's own docblock.
 *
 * @extends ServiceEntityRepository<PlatformSubscription>
 */
class PlatformSubscriptionRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, PlatformSubscription::class);
    }

    public function findForTrainer(Trainer $trainer): ?PlatformSubscription
    {
        return $this->findOneBy(['trainer' => $trainer]);
    }

    /**
     * Lazily provisions, same reasoning as
     * TrainerBillingSettingsRepository::getOrCreateForTrainer().
     */
    public function getOrCreateForTrainer(Trainer $trainer): PlatformSubscription
    {
        $existing = $this->findForTrainer($trainer);

        if (null !== $existing) {
            return $existing;
        }

        $subscription = new PlatformSubscription($trainer);
        $this->add($subscription);

        return $subscription;
    }

    /**
     * BR-07-7 (Epic-07, currently unbuilt): "active trainers" = active
     * Stripe subscription. Exposed here as the prepared extension point,
     * mirroring RsvpRepository::findConfirmedForPlayerOnDate()'s own
     * precedent for a not-yet-built consumer.
     */
    public function countActive(): int
    {
        $count = $this->createQueryBuilder('s')
            ->select('COUNT(s.id)')
            ->andWhere('s.status = :active')
            ->setParameter('active', PlatformSubscription::STATUS_ACTIVE)
            ->getQuery()
            ->getSingleScalarResult();

        return (int) $count;
    }

    public function add(PlatformSubscription $subscription): void
    {
        $this->getEntityManager()->persist($subscription);
    }
}
