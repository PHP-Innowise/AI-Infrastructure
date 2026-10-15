<?php

declare(strict_types=1);

namespace App\Identity\Repository;

use App\Identity\Entity\ShareLinkOpen;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<ShareLinkOpen>
 */
class ShareLinkOpenRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, ShareLinkOpen::class);
    }

    public function add(ShareLinkOpen $open): void
    {
        $this->getEntityManager()->persist($open);
    }

    public function countFor(int $shareLinkId): int
    {
        return (int) $this->createQueryBuilder('o')
            ->select('COUNT(o.id)')
            ->andWhere('IDENTITY(o.shareLink) = :shareLinkId')
            ->setParameter('shareLinkId', $shareLinkId)
            ->getQuery()
            ->getSingleScalarResult();
    }

    /**
     * AC-03-39/BR-03-22: "15 link opens this week" — every ShareLink this
     * trainer owns, within the active tenant (the Doctrine filter/RLS scope
     * this automatically since ShareLinkOpen is trainer-scoped).
     */
    public function countForActiveTenantBetween(\DateTimeImmutable $from, \DateTimeImmutable $to): int
    {
        $count = $this->createQueryBuilder('o')
            ->select('COUNT(o.id)')
            ->andWhere('o.openedAt >= :from')
            ->andWhere('o.openedAt < :to')
            ->setParameter('from', $from)
            ->setParameter('to', $to)
            ->getQuery()
            ->getSingleScalarResult();

        return (int) $count;
    }
}
