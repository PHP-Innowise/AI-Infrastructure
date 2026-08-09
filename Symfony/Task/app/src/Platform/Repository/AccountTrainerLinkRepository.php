<?php

declare(strict_types=1);

namespace App\Platform\Repository;

use App\Identity\Entity\Account;
use App\Platform\Entity\AccountTrainerLink;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<AccountTrainerLink>
 */
class AccountTrainerLinkRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, AccountTrainerLink::class);
    }

    /**
     * The resolver's validation step. A selected trainer is only honoured if
     * an active link actually exists — this is the security-bearing half of
     * resolution source 3.
     */
    public function isActiveLink(Account $account, int $trainerId): bool
    {
        $count = $this->createQueryBuilder('l')
            ->select('COUNT(l.id)')
            ->andWhere('l.account = :account')
            ->andWhere('IDENTITY(l.trainer) = :trainerId')
            ->andWhere('l.status = :active')
            ->setParameter('account', $account)
            ->setParameter('trainerId', $trainerId)
            ->setParameter('active', AccountTrainerLink::STATUS_ACTIVE)
            ->getQuery()
            ->getSingleScalarResult();

        return $count > 0;
    }

    /**
     * Backs the trainer switcher (AC-01-15) as well as single-tenant
     * auto-resolution.
     *
     * @return list<int>
     */
    public function findActiveTrainerIdsFor(Account $account): array
    {
        /** @var list<array{trainerId: int|string}> $rows */
        $rows = $this->createQueryBuilder('l')
            ->select('IDENTITY(l.trainer) AS trainerId')
            ->andWhere('l.account = :account')
            ->andWhere('l.status = :active')
            ->setParameter('account', $account)
            ->setParameter('active', AccountTrainerLink::STATUS_ACTIVE)
            ->orderBy('l.linkedAt', 'ASC')
            ->getQuery()
            ->getArrayResult();

        return array_map(static fn (array $row): int => (int) $row['trainerId'], $rows);
    }

    /**
     * @return list<AccountTrainerLink>
     */
    public function findActiveFor(Account $account): array
    {
        /** @var list<AccountTrainerLink> $links */
        $links = $this->createQueryBuilder('l')
            ->andWhere('l.account = :account')
            ->andWhere('l.status = :active')
            ->setParameter('account', $account)
            ->setParameter('active', AccountTrainerLink::STATUS_ACTIVE)
            ->orderBy('l.linkedAt', 'ASC')
            ->getQuery()
            ->getResult();

        return $links;
    }
}
