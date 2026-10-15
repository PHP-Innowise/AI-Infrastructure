<?php

declare(strict_types=1);

namespace App\Identity\Repository;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\AccountStatus;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\ORM\QueryBuilder;
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
     * AC-01-72/AC-07-8..10: the Users tool listing, tool-specific search and
     * filters — not a global search (architect-architecture.md's own
     * distinction). AC-07-12: 50/page, via $page (1-based).
     *
     * @return list<Account>
     */
    public function search(?string $query, ?AccountRole $role, ?AccountStatus $status, int $limit = 50, int $page = 1): array
    {
        $qb = $this->filteredQuery($query, $role, $status)
            ->addSelect('p')
            ->orderBy('a.createdAt', 'DESC')
            ->setMaxResults($limit)
            ->setFirstResult($limit * max(0, $page - 1));

        /** @var list<Account> $rows */
        $rows = $qb->getQuery()->getResult();

        return $rows;
    }

    /**
     * AC-07-12: page-navigation total, same filters as search().
     */
    public function countMatching(?string $query, ?AccountRole $role, ?AccountStatus $status): int
    {
        $count = $this->filteredQuery($query, $role, $status)
            ->select('COUNT(a.id)')
            ->getQuery()
            ->getSingleScalarResult();

        return (int) $count;
    }

    /**
     * BR-07-7: "total players = player accounts with status active; total
     * coaches = coach accounts with status active."
     */
    public function countByRoleAndStatus(AccountRole $role, AccountStatus $status): int
    {
        $count = $this->createQueryBuilder('a')
            ->select('COUNT(a.id)')
            ->andWhere('a.role = :role')
            ->andWhere('a.status = :status')
            ->setParameter('role', $role)
            ->setParameter('status', $status)
            ->getQuery()
            ->getSingleScalarResult();

        return (int) $count;
    }

    /**
     * BR-07-8: "New users this week = users registered in the last 7 days" —
     * every role, matching the epic's own unqualified "users" wording. Also
     * used for "new users this month" with a wider window (AC-07-2).
     */
    public function countCreatedBetween(\DateTimeImmutable $from, \DateTimeImmutable $to): int
    {
        $count = $this->createQueryBuilder('a')
            ->select('COUNT(a.id)')
            ->andWhere('a.createdAt >= :from')
            ->andWhere('a.createdAt < :to')
            ->setParameter('from', $from)
            ->setParameter('to', $to)
            ->getQuery()
            ->getSingleScalarResult();

        return (int) $count;
    }

    /**
     * AC-07-2: "30-day user growth chart" — one row per calendar day in
     * `[from, to)` that had at least one registration; days with zero are
     * filled in by the caller (a sparse native query, matching
     * `AttendanceRecordRepository::attendanceTallyBetween()`'s own use of a
     * native aggregate for reporting, since `Account` is global — no
     * `CrossTenantReadService` crossing needed).
     *
     * @return array<string, int> ISO date ('Y-m-d') => count
     */
    public function dailyRegistrationCounts(\DateTimeImmutable $from, \DateTimeImmutable $to): array
    {
        $rows = $this->getEntityManager()->getConnection()->fetchAllAssociative(
            <<<'SQL'
                SELECT DATE(created_at) AS day, COUNT(*) AS cnt
                FROM account
                WHERE created_at >= :from AND created_at < :to
                GROUP BY DATE(created_at)
                SQL,
            ['from' => $from->format('Y-m-d H:i:sP'), 'to' => $to->format('Y-m-d H:i:sP')],
        );

        $counts = [];
        foreach ($rows as $row) {
            $counts[(string) $row['day']] = (int) $row['cnt'];
        }

        return $counts;
    }

    public function add(Account $account): void
    {
        $this->getEntityManager()->persist($account);
    }

    private function filteredQuery(?string $query, ?AccountRole $role, ?AccountStatus $status): QueryBuilder
    {
        $qb = $this->createQueryBuilder('a')
            ->leftJoin('a.profile', 'p');

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

        return $qb;
    }
}
