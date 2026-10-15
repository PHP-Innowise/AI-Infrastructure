<?php

declare(strict_types=1);

namespace App\Platform\Repository;

use App\Identity\Entity\Account;
use App\Platform\Entity\FeatureToggle;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * Global — see FeatureToggle's own docblock for why this lives in
 * `Platform` rather than `Administration`.
 *
 * @extends ServiceEntityRepository<FeatureToggle>
 */
class FeatureToggleRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, FeatureToggle::class);
    }

    public function findOneByTrainerAndFeature(Trainer $trainer, string $featureName): ?FeatureToggle
    {
        return $this->findOneBy(['trainer' => $trainer, 'featureName' => $featureName]);
    }

    /**
     * AC-07-18: the trainer's own feature-toggle list, in the epic's own
     * display order (LPPP, Marketing, Camps) — never alphabetical, which
     * would silently reorder if a feature name changes.
     *
     * @return list<FeatureToggle>
     */
    public function findAllForTrainer(Trainer $trainer): array
    {
        /** @var list<FeatureToggle> $rows */
        $rows = $this->createQueryBuilder('f')
            ->andWhere('f.trainer = :trainer')
            ->setParameter('trainer', $trainer)
            ->getQuery()
            ->getResult();

        $byFeature = [];
        foreach ($rows as $row) {
            $byFeature[$row->getFeatureName()] = $row;
        }

        return array_values(array_filter(array_map(
            static fn (string $feature): ?FeatureToggle => $byFeature[$feature] ?? null,
            FeatureToggle::ALL_FEATURES,
        )));
    }

    /**
     * BR-07-2: "All three features default to enabled for new trainers."
     * Idempotent — a re-run (e.g. a fixture reload, or a trainer created
     * before this table existed and backfilled by the migration) never
     * overwrites an existing row's state, since Super Admin may already
     * have disabled one before onboarding completes ("...Super Admin can
     * disable a feature before a trainer's onboarding completes").
     */
    public function seedDefaultsForTrainer(Trainer $trainer, Account $by): void
    {
        $existing = [];
        foreach ($this->findAllForTrainer($trainer) as $toggle) {
            $existing[$toggle->getFeatureName()] = true;
        }

        foreach (FeatureToggle::ALL_FEATURES as $feature) {
            if (!isset($existing[$feature])) {
                $this->add(new FeatureToggle($trainer, $feature, $by));
            }
        }
    }

    public function add(FeatureToggle $toggle): void
    {
        $this->getEntityManager()->persist($toggle);
    }
}
