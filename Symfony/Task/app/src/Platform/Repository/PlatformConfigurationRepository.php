<?php

declare(strict_types=1);

namespace App\Platform\Repository;

use App\Identity\Entity\Account;
use App\Platform\Entity\PlatformConfiguration;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\ORM\EntityManagerInterface;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<PlatformConfiguration>
 */
class PlatformConfigurationRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, PlatformConfiguration::class);
    }

    public function findOneByKey(string $key): ?PlatformConfiguration
    {
        return $this->findOneBy(['key' => $key]);
    }

    /**
     * Lazily provisions a config row at $default on first read — same
     * "provision on first access, idempotent" idiom as
     * `TrainerBillingSettingsRepository::getOrCreateForTrainer()`. Global
     * (no tenant), so unlike that method there is no per-trainer key.
     */
    public function getOrCreate(string $key, mixed $default): PlatformConfiguration
    {
        $existing = $this->findOneByKey($key);

        if (null !== $existing) {
            return $existing;
        }

        $configuration = new PlatformConfiguration($key, $default);
        $this->add($configuration);

        /** @var EntityManagerInterface $entityManager */
        $entityManager = $this->getEntityManager();
        $entityManager->flush();

        return $configuration;
    }

    public function add(PlatformConfiguration $configuration): void
    {
        $this->getEntityManager()->persist($configuration);
    }

    public function upsert(string $key, mixed $value, ?Account $updatedBy): PlatformConfiguration
    {
        $existing = $this->findOneByKey($key);

        if (null !== $existing) {
            $existing->replaceValue($value, $updatedBy);

            return $existing;
        }

        $configuration = new PlatformConfiguration($key, $value, $updatedBy);
        $this->add($configuration);

        return $configuration;
    }
}
