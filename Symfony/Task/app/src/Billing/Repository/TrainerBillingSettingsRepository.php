<?php

declare(strict_types=1);

namespace App\Billing\Repository;

use App\Billing\Entity\TrainerBillingSettings;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<TrainerBillingSettings>
 */
class TrainerBillingSettingsRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, TrainerBillingSettings::class);
    }

    public function findForTrainer(Trainer $trainer): ?TrainerBillingSettings
    {
        return $this->find($trainer->getId());
    }

    /**
     * Lazily provisions a trainer's billing settings on first access, at
     * whatever platform default rate is in force (AC-05-2). Nothing in
     * Identity (Epic-01, where a `Trainer` is actually created) may call
     * into Billing directly — module map: "Identity... May call: Platform"
     * only — so this cannot be provisioned eagerly at trainer-creation time
     * from that side; every Billing-owned entry point that needs a
     * trainer's settings (the settings screen, a paid-event/content
     * gateway call, a fee edit) calls this instead of `find()` directly.
     * Idempotent: returns the existing row if one is already there.
     */
    public function getOrCreateForTrainer(Trainer $trainer): TrainerBillingSettings
    {
        $existing = $this->findForTrainer($trainer);

        if (null !== $existing) {
            return $existing;
        }

        $settings = new TrainerBillingSettings($trainer);
        $this->add($settings);

        return $settings;
    }

    public function add(TrainerBillingSettings $settings): void
    {
        $this->getEntityManager()->persist($settings);
    }
}
