<?php

declare(strict_types=1);

namespace App\Platform\Repository;

use App\Platform\Entity\PublicTenantCode;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<PublicTenantCode>
 */
class PublicTenantCodeRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, PublicTenantCode::class);
    }

    /**
     * The lookup that breaks the resolution circularity. Runs with no tenant
     * established, which is precisely why this table carries no RLS policy.
     *
     * An unknown code resolves nothing, and the route 404s before a single
     * trainer-scoped statement is issued.
     */
    public function findTrainerIdByCode(string $code): ?int
    {
        /** @var array{trainerId: int|string}|null $row */
        $row = $this->createQueryBuilder('c')
            ->select('IDENTITY(c.trainer) AS trainerId')
            ->andWhere('c.code = :code')
            ->setParameter('code', $code)
            ->getQuery()
            ->getOneOrNullResult();

        return null === $row ? null : (int) $row['trainerId'];
    }

    public function findOneByReference(string $kind, int $referenceId): ?PublicTenantCode
    {
        return $this->findOneBy(['kind' => $kind, 'referenceId' => $referenceId]);
    }

    public function add(PublicTenantCode $code): void
    {
        $this->getEntityManager()->persist($code);
    }

    public function remove(PublicTenantCode $code): void
    {
        $this->getEntityManager()->remove($code);
    }
}
