<?php

declare(strict_types=1);

namespace App\Identity\Repository;

use App\Identity\Entity\Account;
use App\Identity\Entity\ParentChildLink;
use App\Identity\Entity\PlayerProfile;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<ParentChildLink>
 */
class ParentChildLinkRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, ParentChildLink::class);
    }

    /**
     * AC-01-22: the Family / Player Profiles listing.
     *
     * @return list<ParentChildLink>
     */
    public function findByParent(Account $parent): array
    {
        /** @var list<ParentChildLink> $rows */
        $rows = $this->createQueryBuilder('l')
            ->andWhere('l.parentAccount = :parent')
            ->setParameter('parent', $parent)
            ->orderBy('l.createdAt', 'ASC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function findByChildPlayer(PlayerProfile $child): ?ParentChildLink
    {
        return $this->findOneBy(['childPlayer' => $child]);
    }

    public function findByChildAccount(Account $childAccount): ?ParentChildLink
    {
        return $this->findOneBy(['childAccount' => $childAccount]);
    }

    /**
     * AC-01-21: a non-blocking duplicate warning — same parent, similar name
     * and the same age (derived from date of birth at query time by the
     * caller, since age is not stored).
     *
     * @return list<ParentChildLink>
     */
    public function findSimilarForParent(Account $parent, string $firstName): array
    {
        /** @var list<ParentChildLink> $rows */
        $rows = $this->createQueryBuilder('l')
            ->join('l.childPlayer', 'p')
            ->andWhere('l.parentAccount = :parent')
            ->andWhere('LOWER(p.firstName) = LOWER(:firstName)')
            ->setParameter('parent', $parent)
            ->setParameter('firstName', $firstName)
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function add(ParentChildLink $link): void
    {
        $this->getEntityManager()->persist($link);
    }
}
