<?php

declare(strict_types=1);

namespace App\Platform\Repository;

use App\Platform\Entity\Trainer;
use App\Platform\Entity\TrainerBrandingSettings;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<TrainerBrandingSettings>
 */
class TrainerBrandingSettingsRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, TrainerBrandingSettings::class);
    }

    public function findForTrainer(Trainer $trainer): ?TrainerBrandingSettings
    {
        return $this->find($trainer->getId());
    }

    public function add(TrainerBrandingSettings $settings): void
    {
        $this->getEntityManager()->persist($settings);
    }
}
