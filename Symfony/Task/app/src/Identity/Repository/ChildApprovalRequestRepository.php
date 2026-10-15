<?php

declare(strict_types=1);

namespace App\Identity\Repository;

use App\Identity\Entity\Account;
use App\Identity\Entity\ChildApprovalRequest;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<ChildApprovalRequest>
 */
class ChildApprovalRequestRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, ChildApprovalRequest::class);
    }

    /**
     * AC-01-25/26: the parent's pending-approvals inbox, within the active
     * tenant.
     *
     * @return list<ChildApprovalRequest>
     */
    public function findForParent(Account $parent): array
    {
        /** @var list<ChildApprovalRequest> $rows */
        $rows = $this->createQueryBuilder('r')
            ->andWhere('r.parentAccount = :parent')
            ->setParameter('parent', $parent)
            ->orderBy('r.requestedAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function add(ChildApprovalRequest $request): void
    {
        $this->getEntityManager()->persist($request);
    }
}
