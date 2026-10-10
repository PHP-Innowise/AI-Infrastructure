<?php

declare(strict_types=1);

namespace App\Billing\Repository;

use App\Billing\Entity\TokenBalance;
use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\DBAL\LockMode;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<TokenBalance>
 */
class TokenBalanceRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, TokenBalance::class);
    }

    public function findForTrainerAndParent(Trainer $trainer, Account $parentAccount): ?TokenBalance
    {
        return $this->findOneBy(['trainer' => $trainer, 'parentAccount' => $parentAccount]);
    }

    /**
     * Architecture "Lock ordering": **the fixed global order is the token
     * balance row first, then the event row.** Every call site that also
     * needs to lock an `Event`/capacity row (a paid RSVP, an RSVP-cancellation
     * refund, a capacity-checked content purchase) must call this FIRST,
     * inside the same transaction, before ever calling
     * `EventRepository::lockForUpdate()` — this is the one method every such
     * path funnels through so the order cannot be gotten wrong.
     *
     * Creates the row (balance 0) if this (trainer, parent) pair has never
     * held one before, inside the SAME lock acquisition — a fresh row is
     * still exclusively "ours" for the rest of the transaction the moment it
     * is inserted, so there is no separate unlocked existence-check to race.
     */
    public function lockForUpdate(Trainer $trainer, Account $parentAccount): TokenBalance
    {
        $connection = $this->getEntityManager()->getConnection();

        // Ensure a row exists first (balance 0) — idempotent and race-safe:
        // a concurrent transaction attempting the very same insert either
        // wins this harmlessly or backs off via ON CONFLICT DO NOTHING,
        // since neither outcome changes what ends up on disk. Issued as a
        // direct statement (not through the ORM's deferred UnitOfWork) so
        // it takes effect immediately, on the same connection/transaction,
        // before the locking SELECT below.
        $connection->executeStatement(
            'INSERT INTO token_balance (trainer_id, parent_account_id, balance, updated_at)
             VALUES (?, ?, 0, ?) ON CONFLICT (trainer_id, parent_account_id) DO NOTHING',
            [$trainer->getId(), $parentAccount->getId(), (new \DateTimeImmutable())->format('Y-m-d H:i:sP')],
        );

        // The actual lock acquisition. Architecture "Lock ordering": every
        // call site that also needs an Event/capacity row calls THIS method
        // first, inside the same transaction, before ever calling
        // EventRepository::lockForUpdate() — the token balance row is always
        // the first lock taken.
        /** @var TokenBalance|null $balance */
        $balance = $this->createQueryBuilder('b')
            ->andWhere('b.trainer = :trainer')
            ->andWhere('b.parentAccount = :parent')
            ->setParameter('trainer', $trainer)
            ->setParameter('parent', $parentAccount)
            ->getQuery()
            ->setLockMode(LockMode::PESSIMISTIC_WRITE)
            ->getOneOrNullResult();

        if (null === $balance) {
            // Unreachable in practice (the insert above guarantees a row),
            // kept as a defensive guard against LockMode's own null-safety.
            throw new \LogicException('Token balance row could not be created or located.');
        }

        return $balance;
    }

    public function add(TokenBalance $balance): void
    {
        $this->getEntityManager()->persist($balance);
    }
}
