<?php

declare(strict_types=1);

namespace App\Growth\Repository;

use App\Growth\Entity\Coupon;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<Coupon>
 */
class CouponRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, Coupon::class);
    }

    /**
     * Coupon codes are case-sensitive in this schema (unlike Label names) —
     * a plain VARCHAR equality comparison, deliberately NOT `LOWER(...)`
     * either side (`specs/database-designer-schema.md` "`coupon`": "See
     * Open questions — case-sensitivity unstated"; the schema chose the
     * conservative case-sensitive default). Do not "fix" this into
     * case-insensitivity.
     */
    public function findOneByTrainerAndCode(Trainer $trainer, string $code): ?Coupon
    {
        return $this->findOneBy(['trainer' => $trainer, 'code' => $code]);
    }

    /**
     * AC-06-19/26: the trainer's coupon list, newest first.
     *
     * @return list<Coupon>
     */
    public function findAllForActiveTenant(): array
    {
        /** @var list<Coupon> $rows */
        $rows = $this->createQueryBuilder('c')
            ->orderBy('c.createdAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * AC-06-28: "total coupons created" (all-time).
     */
    public function countCreated(Trainer $trainer): int
    {
        return (int) $this->createQueryBuilder('c')
            ->select('COUNT(c.id)')
            ->andWhere('c.trainer = :trainer')
            ->setParameter('trainer', $trainer)
            ->getQuery()
            ->getSingleScalarResult();
    }

    /**
     * AC-06-28: "active coupons" (current state, not point-in-time).
     */
    public function countActive(Trainer $trainer): int
    {
        return (int) $this->createQueryBuilder('c')
            ->select('COUNT(c.id)')
            ->andWhere('c.trainer = :trainer')
            ->andWhere('c.isActive = true')
            ->setParameter('trainer', $trainer)
            ->getQuery()
            ->getSingleScalarResult();
    }

    public function add(Coupon $coupon): void
    {
        $this->getEntityManager()->persist($coupon);
    }

    public function remove(Coupon $coupon): void
    {
        $this->getEntityManager()->remove($coupon);
    }
}
