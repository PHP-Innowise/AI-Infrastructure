<?php

declare(strict_types=1);

namespace App\Platform\Repository;

use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<Trainer>
 */
class TrainerRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, Trainer::class);
    }

    public function findOneBySlug(string $slug): ?Trainer
    {
        return $this->findOneBy(['slug' => $slug]);
    }
}
