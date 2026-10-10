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

    public function slugExists(string $slug): bool
    {
        return null !== $this->findOneBySlug($slug);
    }

    /**
     * AC-07-38 and administration_trainers_index's own listing.
     *
     * @return list<Trainer>
     */
    public function search(?string $query, int $limit = 50): array
    {
        $qb = $this->createQueryBuilder('t')
            ->orderBy('t.createdAt', 'DESC')
            ->setMaxResults($limit);

        if (null !== $query && '' !== trim($query)) {
            $qb->andWhere('t.businessName LIKE :q')->setParameter('q', '%'.$query.'%');
        }

        /** @var list<Trainer> $rows */
        $rows = $qb->getQuery()->getResult();

        return $rows;
    }

    public function add(Trainer $trainer): void
    {
        $this->getEntityManager()->persist($trainer);
    }
}
