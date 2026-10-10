<?php

declare(strict_types=1);

namespace App\Billing\Repository;

use App\Billing\Entity\PaymentRecord;
use App\Billing\Entity\SubscriptionEntitlement;
use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<SubscriptionEntitlement>
 */
class SubscriptionEntitlementRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, SubscriptionEntitlement::class);
    }

    /**
     * BR-05-14: the entitlement covering $today, if any — the database's own
     * EXCLUDE constraint guarantees at most one row can ever match.
     */
    public function findActiveOn(Trainer $trainer, Account $parentAccount, \DateTimeImmutable $today): ?SubscriptionEntitlement
    {
        return $this->createQueryBuilder('s')
            ->andWhere('s.trainer = :trainer')
            ->andWhere('s.parentAccount = :parent')
            ->andWhere('s.activationDate <= :today')
            ->andWhere('s.windowEndsOn >= :today')
            ->setParameter('trainer', $trainer)
            ->setParameter('parent', $parentAccount)
            ->setParameter('today', $today->format('Y-m-d'))
            ->getQuery()
            ->getOneOrNullResult();
    }

    /**
     * `billing_portal_subscription_purchase`'s own "you already hold one"
     * guard — pending-activation or active, i.e. anything not yet expired.
     *
     * @return list<SubscriptionEntitlement>
     */
    public function findNotYetExpired(Trainer $trainer, Account $parentAccount, \DateTimeImmutable $today): array
    {
        /** @var list<SubscriptionEntitlement> $rows */
        $rows = $this->createQueryBuilder('s')
            ->andWhere('s.trainer = :trainer')
            ->andWhere('s.parentAccount = :parent')
            ->andWhere('s.windowEndsOn >= :today')
            ->setParameter('trainer', $trainer)
            ->setParameter('parent', $parentAccount)
            ->setParameter('today', $today->format('Y-m-d'))
            ->orderBy('s.activationDate', 'ASC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * The webhook handler's own idempotency check for
     * `customer.subscription.created`-driven entitlement granting
     * (AC-05-29, AC-05-35 replay-safety) — `uniq_subscription_entitlement_
     * payment_record` is the database-level guarantee this mirrors.
     * `PaymentRecord` carries no `related*` column back to the
     * entitlement it funds (see `PaymentRecord::TYPE_PLAYER_SUBSCRIPTION`'s
     * own docblock for why); this is the one place the reverse lookup is
     * actually needed.
     */
    public function findOneByPaymentRecord(PaymentRecord $paymentRecord): ?SubscriptionEntitlement
    {
        return $this->createQueryBuilder('s')
            ->andWhere('s.paymentRecord = :paymentRecord')
            ->setParameter('paymentRecord', $paymentRecord)
            ->getQuery()
            ->getOneOrNullResult();
    }

    public function add(SubscriptionEntitlement $entitlement): void
    {
        $this->getEntityManager()->persist($entitlement);
    }
}
