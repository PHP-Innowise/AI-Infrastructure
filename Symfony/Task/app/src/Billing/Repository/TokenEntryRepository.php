<?php

declare(strict_types=1);

namespace App\Billing\Repository;

use App\Billing\Entity\PaymentRecord;
use App\Billing\Entity\TokenEntry;
use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * Read/insert only — I7 (append-only) is enforced at the database privilege
 * level (`REVOKE UPDATE, DELETE`), and this repository adds no method that
 * could even attempt one; there is no `update()`/`remove()` here, on
 * purpose.
 *
 * @extends ServiceEntityRepository<TokenEntry>
 */
class TokenEntryRepository extends ServiceEntityRepository
{
    /**
     * The advisory-lock "class" namespace for `lockSpendEntryForUpdate()` —
     * see that method's own docblock. An arbitrary, fixed constant, chosen
     * once and never reused for any other purpose in this codebase.
     */
    private const ADVISORY_LOCK_CLASS = 8_500_105;

    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, TokenEntry::class);
    }

    /**
     * I1: the reconciliation sum — the balance projection must equal this,
     * for the same (trainer, parent) pair, at all times. Used by
     * `TokenLedgerService` itself (computing the new balance to write) and
     * independently by the reconciliation console command/test.
     */
    public function sumForTrainerAndParent(Trainer $trainer, Account $parentAccount): int
    {
        $sum = $this->createQueryBuilder('e')
            ->select('COALESCE(SUM(e.amount), 0)')
            ->andWhere('e.trainer = :trainer')
            ->andWhere('e.parentAccount = :parent')
            ->setParameter('trainer', $trainer)
            ->setParameter('parent', $parentAccount)
            ->getQuery()
            ->getSingleScalarResult();

        return (int) $sum;
    }

    /**
     * I4: the sum of refunds already issued against one spend entry — read
     * only AFTER the spend row has been locked
     * (`lockSpendEntryForUpdate()`), inside the same transaction, per
     * specs/database-designer-schema.md "`token_entry`": "Locking the spend
     * row first serializes every concurrent refund attempt against it, so
     * the sum read afterward is safe even though it is a plain, unlocked
     * read."
     */
    public function sumRefundsAgainstSpend(TokenEntry $spend): int
    {
        $sum = $this->createQueryBuilder('e')
            ->select('COALESCE(SUM(e.amount), 0)')
            ->andWhere('e.refundsEntry = :spend')
            ->setParameter('spend', $spend)
            ->getQuery()
            ->getSingleScalarResult();

        return (int) $sum;
    }

    /**
     * I4's own locking step — implemented as a PostgreSQL transaction-scoped
     * ADVISORY lock (`pg_advisory_xact_lock`), not the `SELECT ... FOR
     * UPDATE` specs/database-designer-schema.md literally names ("Locking
     * the spend row first serializes every concurrent refund attempt
     * against it... a no-op SELECT ... FOR UPDATE is a legal, common
     * serialization primitive").
     *
     * **This is a deliberate, verified deviation, not a style choice —
     * recorded here and in the coder's final report.** PostgreSQL's row
     * locking clauses (`FOR UPDATE`, `FOR NO KEY UPDATE`, `FOR SHARE`, `FOR
     * KEY SHARE` — all four, regardless of lock strength) require the
     * table's `UPDATE` privilege, confirmed directly against this exact
     * database: `SELECT id FROM token_entry FOR UPDATE` as `pp_app` fails
     * with "permission denied for table token_entry" once I7's `REVOKE
     * UPDATE, DELETE ON token_entry FROM pp_app` (Version20260811100000) is
     * in force — the schema doc's own two stated mechanisms for I4 and I7
     * are mutually exclusive on real PostgreSQL, not merely in theory. An
     * advisory lock needs no table privilege at all (it is keyed by an
     * arbitrary integer pair, not tied to any row or table ACL), so it
     * serializes concurrent refund attempts against the SAME spend entry id
     * exactly as intended, without reopening I7.
     *
     * The lock is namespaced (`pg_advisory_xact_lock(bigint, bigint)`, a
     * fixed arbitrary "class" id + the spend entry's own id) rather than
     * the single-bigint overload, so this can never collide with a
     * numeric key any OTHER advisory-lock consumer might choose —
     * architect-architecture.md "Shared runtime state" already runs
     * Symfony's own PostgreSQL lock store (`symfony/lock`) against this
     * same database.
     *
     * Transaction-scoped (`_xact_`, not session-scoped): released
     * automatically at COMMIT or ROLLBACK, matching a row lock's own
     * lifetime, with no risk of a leaked session-scoped lock surviving past
     * this unit of work.
     */
    public function lockSpendEntryForUpdate(int $spendEntryId): ?TokenEntry
    {
        $this->getEntityManager()->getConnection()->executeStatement(
            'SELECT pg_advisory_xact_lock(?, ?)',
            [self::ADVISORY_LOCK_CLASS, $spendEntryId],
        );

        return $this->find($spendEntryId);
    }

    public function findOneByPaymentRecordAndKind(PaymentRecord $paymentRecord, string $kind): ?TokenEntry
    {
        return $this->findOneBy(['paymentRecord' => $paymentRecord, 'kind' => $kind]);
    }

    /**
     * I6: has an entry of $kind already been recorded for $paymentRecord —
     * the ledger-level outbound idempotency check ("at most one entry
     * exists per (payment record, purpose)"), consulted before inserting so
     * a replayed webhook cannot double-credit the same payment. The unique
     * partial index is the actual enforcement; this is the pre-check that
     * lets the caller respond gracefully instead of catching a constraint
     * violation.
     */
    public function existsForPaymentRecordAndKind(PaymentRecord $paymentRecord, string $kind): bool
    {
        $count = $this->createQueryBuilder('e')
            ->select('COUNT(e.id)')
            ->andWhere('e.paymentRecord = :paymentRecord')
            ->andWhere('e.kind = :kind')
            ->setParameter('paymentRecord', $paymentRecord)
            ->setParameter('kind', $kind)
            ->getQuery()
            ->getSingleScalarResult();

        return ((int) $count) > 0;
    }

    /**
     * AC-05-4/BR-05-2: "You have N tokens with [Trainer]" plus recent
     * activity.
     *
     * @return list<TokenEntry>
     */
    public function findRecentForTrainerAndParent(Trainer $trainer, Account $parentAccount, int $limit = 20): array
    {
        /** @var list<TokenEntry> $rows */
        $rows = $this->createQueryBuilder('e')
            ->andWhere('e.trainer = :trainer')
            ->andWhere('e.parentAccount = :parent')
            ->setParameter('trainer', $trainer)
            ->setParameter('parent', $parentAccount)
            ->orderBy('e.createdAt', 'DESC')
            ->setMaxResults($limit)
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function add(TokenEntry $entry): void
    {
        $this->getEntityManager()->persist($entry);
    }
}
