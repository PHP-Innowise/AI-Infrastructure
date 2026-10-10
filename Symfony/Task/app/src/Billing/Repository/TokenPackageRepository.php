<?php

declare(strict_types=1);

namespace App\Billing\Repository;

use App\Billing\Entity\TokenPackage;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<TokenPackage>
 */
class TokenPackageRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, TokenPackage::class);
    }

    /**
     * AC-05-4: the packages a player/parent may pick from.
     *
     * @return list<TokenPackage>
     */
    public function findActiveForTrainer(Trainer $trainer): array
    {
        /** @var list<TokenPackage> $rows */
        $rows = $this->createQueryBuilder('p')
            ->andWhere('p.trainer = :trainer')
            ->andWhere('p.isActive = true')
            ->setParameter('trainer', $trainer)
            ->orderBy('p.tokenCount', 'ASC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * `billing_trainer_pricing_edit`: every package the trainer has ever
     * created, active or retired, excluding one-off custom-amount packages
     * (`TokenPackage`'s own docblock) — those are never trainer-visible.
     *
     * @return list<TokenPackage>
     */
    public function findAllForTrainer(Trainer $trainer): array
    {
        /** @var list<TokenPackage> $rows */
        $rows = $this->createQueryBuilder('p')
            ->andWhere('p.trainer = :trainer')
            ->setParameter('trainer', $trainer)
            ->orderBy('p.tokenCount', 'ASC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function add(TokenPackage $package): void
    {
        $this->getEntityManager()->persist($package);
    }
}
