<?php

declare(strict_types=1);

namespace App\Platform\Repository;

use App\Identity\Entity\Account;
use App\Platform\Entity\ImpersonationSession;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<ImpersonationSession>
 */
class ImpersonationSessionRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, ImpersonationSession::class);
    }

    /**
     * "Is this admin already impersonating someone" — the partial index
     * `(admin_account_id) WHERE ended_at IS NULL` backs exactly this lookup.
     *
     * Ordered by id, not just startedAt: the column is `TIMESTAMP(0)` (whole
     * seconds), so two sessions opened within the same second would
     * otherwise tie and sort arbitrarily.
     */
    public function findOpenSessionForAdmin(Account $admin): ?ImpersonationSession
    {
        return $this->createQueryBuilder('s')
            ->andWhere('s.adminAccount = :admin')
            ->andWhere('s.endedAt IS NULL')
            ->setParameter('admin', $admin)
            ->orderBy('s.id', 'DESC')
            ->setMaxResults(1)
            ->getQuery()
            ->getOneOrNullResult();
    }

    /**
     * AC-01-36: the Impersonation History report.
     *
     * @return list<ImpersonationSession>
     */
    public function findAll(): array
    {
        /** @var list<ImpersonationSession> $rows */
        $rows = $this->createQueryBuilder('s')
            ->orderBy('s.id', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function add(ImpersonationSession $session): void
    {
        $this->getEntityManager()->persist($session);
    }
}
