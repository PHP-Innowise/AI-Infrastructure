<?php

declare(strict_types=1);

namespace App\Billing\Service;

use App\Billing\Entity\PaymentRecord;
use App\Billing\Entity\TokenBalance;
use App\Billing\Entity\TokenEntry;
use App\Billing\Exception\InsufficientTokenBalanceException;
use App\Billing\Exception\RefundExceedsSpendException;
use App\Billing\Repository\TokenBalanceRepository;
use App\Billing\Repository\TokenEntryRepository;
use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use App\Platform\Service\AuditLogger;
use App\Scheduling\Entity\Event;
use Doctrine\ORM\EntityManagerInterface;

/**
 * The **only** writer of token entries and balances
 * (architect-architecture.md "Cross-cutting services": "Enforces lock
 * order, sign-by-kind and every ledger invariant"). `Scheduling`, `Content`
 * and `Growth` reach this exclusively through their own narrow seams
 * (`PaymentIntentGateway`'s adapters, `TokenVoter`-gated controllers) —
 * nothing outside `Billing` constructs a `TokenEntry` directly.
 *
 * Every public method here is self-contained: it locks what it needs and
 * commits (via `wrapInTransaction`, which safely NESTS inside an
 * already-open outer transaction rather than opening a second physical one
 * — Doctrine's own documented behavior). `lockBalanceForUpdate()` is the
 * one exception, existing purely so a caller that ALSO needs a second lock
 * (the event/capacity row) can establish architecture's fixed order —
 * "the token balance row first, then the event row" — before taking that
 * second lock itself, all inside the caller's own outer transaction.
 *
 * @see specs/database-designer-schema.md "The token and payment ledger"
 * @see specs/architect-architecture.md "The token and payment ledger"
 */
final readonly class TokenLedgerService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private TokenBalanceRepository $balances,
        private TokenEntryRepository $entries,
        private AuditLogger $auditLogger,
    ) {
    }

    /**
     * Architecture "Lock ordering": acquires the token balance row lock —
     * always the FIRST lock in the fixed global order. A caller that will
     * also lock an event/capacity row in the SAME transaction
     * (`RsvpService::rsvp()`) calls this before its own
     * `EventRepository::lockForUpdate()`. Re-locking the same row later
     * (e.g. this class's own `spend()`, called afterward) is a harmless
     * no-op re-acquisition within the same transaction, never a second,
     * conflicting lock.
     */
    public function lockBalanceForUpdate(Trainer $trainer, Account $parentAccount): TokenBalance
    {
        return $this->balances->lockForUpdate($trainer, $parentAccount);
    }

    public function balanceFor(Trainer $trainer, Account $parentAccount): int
    {
        return $this->balances->findForTrainerAndParent($trainer, $parentAccount)?->getBalance() ?? 0;
    }

    /**
     * AC-05-4: a token purchase funded by $paymentRecord. I6 idempotency:
     * a replayed webhook for the same payment record is a silent no-op,
     * returning the entry already recorded rather than double-crediting.
     */
    public function purchase(Trainer $trainer, Account $parentAccount, int $tokenCount, PaymentRecord $paymentRecord, string $description): TokenEntry
    {
        return $this->entityManager->wrapInTransaction(function () use ($trainer, $parentAccount, $tokenCount, $paymentRecord, $description): TokenEntry {
            $existing = $this->entries->findOneByPaymentRecordAndKind($paymentRecord, TokenEntry::KIND_PURCHASE);

            if (null !== $existing) {
                return $existing;
            }

            $balance = $this->balances->lockForUpdate($trainer, $parentAccount);
            $entry = TokenEntry::purchase($trainer, $parentAccount, $tokenCount, $paymentRecord, $description);
            $this->entries->add($entry);
            $balance->applyNewTotal($balance->getBalance() + $tokenCount);
            $this->entityManager->flush();

            return $entry;
        });
    }

    /**
     * AC-05-33: trainer-only, manual, no payment, audit-logged.
     */
    public function gift(Trainer $trainer, Account $parentAccount, int $tokenCount, Account $grantedBy, string $note): TokenEntry
    {
        return $this->entityManager->wrapInTransaction(function () use ($trainer, $parentAccount, $tokenCount, $grantedBy, $note): TokenEntry {
            $balance = $this->balances->lockForUpdate($trainer, $parentAccount);
            $entry = TokenEntry::gift($trainer, $parentAccount, $tokenCount, $grantedBy, $note);
            $this->entries->add($entry);
            $balance->applyNewTotal($balance->getBalance() + $tokenCount);
            $this->entityManager->flush();

            $this->auditLogger->record($grantedBy, 'token.gift', 'TokenBalance', $balance->getId(), $trainer, [
                'parentAccountId' => $parentAccount->getId(),
                'amount' => $tokenCount,
                'note' => $note,
            ]);
            $this->entityManager->flush();

            return $entry;
        });
    }

    /**
     * Epic-06's own reward (architect-architecture.md Decisions, "Referral
     * rewards") — kept as the prepared extension point for Growth, not
     * called by anything in this codebase yet.
     */
    public function referralReward(Trainer $trainer, Account $parentAccount, int $tokenCount, int $referralId, string $description): TokenEntry
    {
        return $this->entityManager->wrapInTransaction(function () use ($trainer, $parentAccount, $tokenCount, $referralId, $description): TokenEntry {
            $balance = $this->balances->lockForUpdate($trainer, $parentAccount);
            $entry = TokenEntry::referralReward($trainer, $parentAccount, $tokenCount, $referralId, $description);
            $this->entries->add($entry);
            $balance->applyNewTotal($balance->getBalance() + $tokenCount);
            $this->entityManager->flush();

            return $entry;
        });
    }

    /**
     * AC-05-7/9/18/19: spends tokens for an RSVP or a content purchase. A7:
     * $beneficiaryPlayer is always recorded, even though the balance sits
     * at the parent-trainer pair. Throws InsufficientTokenBalanceException
     * (AC-05-8) rather than ever writing a balance-breaching entry — I2's
     * database CHECK is the backstop, this is the fail-fast application
     * check.
     *
     * @throws InsufficientTokenBalanceException
     */
    public function spend(
        Trainer $trainer,
        Account $parentAccount,
        int $tokenCount,
        PlayerProfile $beneficiaryPlayer,
        string $description,
        ?PaymentRecord $paymentRecord = null,
        ?Event $relatedEvent = null,
        ?int $relatedContentItemId = null,
    ): TokenEntry {
        return $this->entityManager->wrapInTransaction(function () use ($trainer, $parentAccount, $tokenCount, $beneficiaryPlayer, $description, $paymentRecord, $relatedEvent, $relatedContentItemId): TokenEntry {
            $balance = $this->balances->lockForUpdate($trainer, $parentAccount);

            if ($balance->getBalance() < $tokenCount) {
                throw InsufficientTokenBalanceException::forShortfall($balance->getBalance(), $tokenCount);
            }

            $entry = TokenEntry::spend($trainer, $parentAccount, $tokenCount, $beneficiaryPlayer, $description, $paymentRecord, $relatedEvent, $relatedContentItemId);
            $this->entries->add($entry);
            $balance->applyNewTotal($balance->getBalance() - $tokenCount);
            $this->entityManager->flush();

            return $entry;
        });
    }

    /**
     * AC-05-14/16/17, BR-05-5: refunds tokens for a canceled RSVP/purchase.
     * I4 (sum of refunds against one spend never exceeds it) is enforced
     * here: the spend entry is locked first — a PostgreSQL advisory lock,
     * NOT `SELECT ... FOR UPDATE` on the row itself (see
     * `TokenEntryRepository::lockSpendEntryForUpdate()`'s own docblock for
     * why: Postgres requires `UPDATE` privilege for row-locking clauses,
     * which I7's `REVOKE UPDATE` structurally forbids) — which serializes
     * every concurrent refund attempt against it, so the sum read
     * immediately afterward is safe even though unlocked.
     *
     * @throws RefundExceedsSpendException
     */
    public function refund(Trainer $trainer, TokenEntry $spend, int $tokenCount, string $description): TokenEntry
    {
        return $this->entityManager->wrapInTransaction(function () use ($trainer, $spend, $tokenCount, $description): TokenEntry {
            $lockedSpend = $this->entries->lockSpendEntryForUpdate((int) $spend->getId())
                ?? throw new \LogicException('The spend entry being refunded no longer exists.');

            $alreadyRefunded = $this->entries->sumRefundsAgainstSpend($lockedSpend);
            $originalAmount = abs($lockedSpend->getAmount());

            if ($alreadyRefunded + $tokenCount > $originalAmount) {
                throw RefundExceedsSpendException::forAttempt($originalAmount, $alreadyRefunded, $tokenCount);
            }

            $balance = $this->balances->lockForUpdate($trainer, $lockedSpend->getParentAccount());
            $entry = TokenEntry::refund($trainer, $lockedSpend, $tokenCount, $description);
            $this->entries->add($entry);
            $balance->applyNewTotal($balance->getBalance() + $tokenCount);
            $this->entityManager->flush();

            return $entry;
        });
    }

    /**
     * BR-05-12: Super-Admin-only out-of-band correction, reason required,
     * always audit-logged — the platform's own reconciliation tool for
     * Stripe Dashboard activity with no platform-side counterpart. I2's
     * CHECK constraint is the final backstop against a negative adjustment
     * that would breach a zero balance; this method's own pre-check makes
     * that a clean exception rather than a caught constraint violation.
     */
    public function adjustment(Trainer $trainer, Account $parentAccount, int $signedAmount, Account $superAdmin, string $reason): TokenEntry
    {
        return $this->entityManager->wrapInTransaction(function () use ($trainer, $parentAccount, $signedAmount, $superAdmin, $reason): TokenEntry {
            $balance = $this->balances->lockForUpdate($trainer, $parentAccount);
            $newTotal = $balance->getBalance() + $signedAmount;

            if ($newTotal < 0) {
                throw InsufficientTokenBalanceException::forShortfall($balance->getBalance(), -$signedAmount);
            }

            $entry = TokenEntry::adjustment($trainer, $parentAccount, $signedAmount, $superAdmin, $reason);
            $this->entries->add($entry);
            $balance->applyNewTotal($newTotal);
            $this->entityManager->flush();

            $this->auditLogger->record($superAdmin, 'token.adjustment', 'TokenBalance', $balance->getId(), $trainer, [
                'parentAccountId' => $parentAccount->getId(),
                'amount' => $signedAmount,
                'reason' => $reason,
            ]);
            $this->entityManager->flush();

            return $entry;
        });
    }

    /**
     * I1: independent reconciliation — the balance projection must equal
     * the sum of that pair's entries. Used by the reconciliation console
     * command and its own test; never called from a request path.
     */
    public function isReconciled(Trainer $trainer, Account $parentAccount): bool
    {
        $balance = $this->balances->findForTrainerAndParent($trainer, $parentAccount);
        $projected = $balance?->getBalance() ?? 0;
        $summed = $this->entries->sumForTrainerAndParent($trainer, $parentAccount);

        return $projected === $summed;
    }
}
