<?php

declare(strict_types=1);

namespace App\Identity\Repository;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\AccountStatus;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<Account>
 */
class AccountRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, Account::class);
    }

    /**
     * Email is CITEXT, so this is case-insensitive at the database level
     * rather than by convention at each call site (BR-01-2).
     */
    public function findOneByEmail(string $email): ?Account
    {
        return $this->findOneBy(['email' => $email]);
    }

    /**
     * AC-01-72: the Users tool listing, tool-specific search and filters —
     * not a global search (architect-architecture.md's own distinction).
     *
     * @return list<Account>
     */
    public function search(?string $query, ?AccountRole $role, ?AccountStatus $status, int $limit = 50): array
    {
        $qb = $this->createQueryBuilder('a')
            ->leftJoin('a.profile', 'p')
            ->addSelect('p')
            ->orderBy('a.createdAt', 'DESC')
            ->setMaxResults($limit);

        if (null !== $query && '' !== trim($query)) {
            $qb->andWhere('a.email LIKE :q OR p.firstName LIKE :q OR p.lastName LIKE :q')
                ->setParameter('q', '%'.$query.'%');
        }

        if (null !== $role) {
            $qb->andWhere('a.role = :role')->setParameter('role', $role);
        }

        if (null !== $status) {
            $qb->andWhere('a.status = :status')->setParameter('status', $status);
        }

        /** @var list<Account> $rows */
        $rows = $qb->getQuery()->getResult();

        return $rows;
    }

    public function add(Account $account): void
    {
        $this->getEntityManager()->persist($account);
    }
}
