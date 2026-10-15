<?php

declare(strict_types=1);

namespace App\Billing\Repository;

use App\Billing\Entity\StripeEventReceipt;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * Global — see StripeEventReceipt's own docblock.
 *
 * @extends ServiceEntityRepository<StripeEventReceipt>
 */
class StripeEventReceiptRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, StripeEventReceipt::class);
    }

    public function findOneByStripeEventId(string $stripeEventId): ?StripeEventReceipt
    {
        return $this->findOneBy(['stripeEventId' => $stripeEventId]);
    }

    public function add(StripeEventReceipt $receipt): void
    {
        $this->getEntityManager()->persist($receipt);
    }
}
