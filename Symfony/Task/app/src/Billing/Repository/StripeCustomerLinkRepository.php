<?php

declare(strict_types=1);

namespace App\Billing\Repository;

use App\Billing\Entity\StripeCustomerLink;
use App\Identity\Entity\Account;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * Global — see StripeCustomerLink's own docblock.
 *
 * @extends ServiceEntityRepository<StripeCustomerLink>
 */
class StripeCustomerLinkRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, StripeCustomerLink::class);
    }

    public function findForAccount(Account $account): ?StripeCustomerLink
    {
        return $this->findOneBy(['account' => $account]);
    }

    public function add(StripeCustomerLink $link): void
    {
        $this->getEntityManager()->persist($link);
    }
}
