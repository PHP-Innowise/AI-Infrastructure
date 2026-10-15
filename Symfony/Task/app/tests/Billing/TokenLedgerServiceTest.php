<?php

declare(strict_types=1);

namespace App\Tests\Billing;

use App\Billing\Entity\PaymentRecord;
use App\Billing\Entity\TokenEntry;
use App\Billing\Exception\InsufficientTokenBalanceException;
use App\Billing\Exception\RefundExceedsSpendException;
use App\Billing\Service\TokenLedgerService;
use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use App\Platform\Repository\AuditLogEntryRepository;
use App\Tests\Support\BillingFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Doctrine\DBAL\Exception\DriverException;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Test\KernelTestCase;

/**
 * The token ledger's own invariants (I1-I6 — I7 is
 * TokenEntryAppendOnlyPrivilegeTest's own, separate proof), exercised
 * through `TokenLedgerService`, the sole writer.
 *
 * @see specs/database-designer-schema.md "The token and payment ledger"
 * @see specs/architect-architecture.md "The token and payment ledger — Invariants, as enforceable rules"
 */
final class TokenLedgerServiceTest extends KernelTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;
    use BillingFixtureHelpers;

    /**
     * I1: the balance projection equals the sum of that pair's entries, at
     * every step.
     */
    public function testBalanceProjectionAlwaysEqualsSumOfEntries(): void
    {
        self::bootKernel();
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $parent = $this->account('player@practiceperfect.test');
        $pat = $this->patPlayer();

        /** @var TokenLedgerService $ledger */
        $ledger = self::getContainer()->get(TokenLedgerService::class);

        $ledger->gift($trainer, $parent, 10, $trainer->getOwnerAccount(), 'I1 fixture: gift.');
        self::assertTrue($ledger->isReconciled($trainer, $parent), 'I1 holds after a gift.');

        $spend = $ledger->spend($trainer, $parent, 3, $pat, 'I1 fixture: spend.');
        self::assertTrue($ledger->isReconciled($trainer, $parent), 'I1 holds after a spend.');

        $ledger->refund($trainer, $spend, 3, 'I1 fixture: refund.');
        self::assertTrue($ledger->isReconciled($trainer, $parent), 'I1 holds after a refund.');
    }

    /**
     * I2: a balance can never go negative — the database CHECK is the
     * backstop; `spend()`'s own pre-check is what turns it into a clean
     * exception (AC-05-8's own "shows the shortfall") instead of a caught
     * constraint violation.
     */
    public function testSpendBeyondBalanceIsRejectedBeforeAnyWrite(): void
    {
        self::bootKernel();
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $parent = $this->account('player@practiceperfect.test');
        $pat = $this->patPlayer();

        /** @var TokenLedgerService $ledger */
        $ledger = self::getContainer()->get(TokenLedgerService::class);
        $before = $ledger->balanceFor($trainer, $parent);

        try {
            $ledger->spend($trainer, $parent, $before + 100, $pat, 'I2 fixture: over-spend attempt.');
            self::fail('Expected InsufficientTokenBalanceException.');
        } catch (InsufficientTokenBalanceException $exception) {
            // AC-05-8: "You have 1 token, need 2 tokens" — the shortfall is
            // carried on the exception for the UI to render that message.
            self::assertSame($before, $exception->currentBalance);
            self::assertSame($before + 100, $exception->required);
        }

        self::assertSame($before, $ledger->balanceFor($trainer, $parent), 'I2: no partial debit on a rejected spend.');
    }

    /**
     * The rejected spend must leave the caller's EntityManager USABLE.
     *
     * The test above proved the exception and the shortfall it carries, and
     * passed for months while the product returned HTTP 500 on this exact
     * path: the exception was thrown from inside `wrapInTransaction`, so
     * Doctrine rolled back and closed the EntityManager, and both gateways —
     * which catch this exception precisely so they can mark their payment
     * record failed and flush — died on `EntityManagerClosed` instead.
     *
     * Asserting the exception is not enough. What the callers actually need
     * is to keep working afterward, so that is what this asserts.
     */
    public function testARejectedSpendLeavesTheEntityManagerOpenForTheCaller(): void
    {
        self::bootKernel();
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $parent = $this->account('player@practiceperfect.test');
        $pat = $this->patPlayer();

        /** @var TokenLedgerService $ledger */
        $ledger = self::getContainer()->get(TokenLedgerService::class);
        /** @var EntityManagerInterface $entityManager */
        $entityManager = self::getContainer()->get(EntityManagerInterface::class);

        $before = $ledger->balanceFor($trainer, $parent);

        try {
            $ledger->spend($trainer, $parent, $before + 100, $pat, 'Closed-EM regression: over-spend attempt.');
            self::fail('Expected InsufficientTokenBalanceException.');
        } catch (InsufficientTokenBalanceException) {
            // Exactly what SchedulingPaymentIntentGateway and
            // ContentPaymentIntentGateway do in their own catch blocks.
            self::assertTrue($entityManager->isOpen(), 'A rejected spend must not close the EntityManager.');
            $entityManager->flush();
        }

        // And the same manager still writes: a gift after the rejection
        // lands, proving the connection was not left rollback-only either.
        $ledger->gift($trainer, $parent, 1, $trainer->getOwnerAccount(), 'Closed-EM regression: the ledger still works.');
        self::assertSame($before + 1, $ledger->balanceFor($trainer, $parent));
    }

    /**
     * A7/I3: every spend records the beneficiary player, even though the
     * balance sits at the parent-trainer pair — a parent spending on
     * behalf of a child still names the CHILD as beneficiary, never the
     * parent.
     */
    public function testSpendRecordsTheBeneficiaryPlayerEvenWhenTheParentPays(): void
    {
        self::bootKernel();
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $parent = $this->account('player@practiceperfect.test');
        $alex = $this->alexPlayer();
        $this->ensureActivePlayerMembership($trainer, $alex);

        /** @var TokenLedgerService $ledger */
        $ledger = self::getContainer()->get(TokenLedgerService::class);
        $ledger->gift($trainer, $parent, 10, $trainer->getOwnerAccount(), 'A7 fixture.');

        $entry = $ledger->spend($trainer, $parent, 2, $alex, 'A7 fixture: spend for child.');

        self::assertSame($parent->getId(), $entry->getParentAccount()->getId(), 'The balance-holding party is the parent.');
        self::assertSame($alex->getId(), $entry->getBeneficiaryPlayer()?->getId(), 'A7: the beneficiary is the CHILD who benefited, not the parent.');
    }

    /**
     * I3's second half: a refund inherits the beneficiary of the spend it
     * references — enforced in code (TokenEntry::refund()), not a CHECK
     * constraint (Postgres cannot read another row).
     */
    public function testRefundInheritsBeneficiaryFromTheSpendItReferences(): void
    {
        self::bootKernel();
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $parent = $this->account('player@practiceperfect.test');
        $alex = $this->alexPlayer();
        $this->ensureActivePlayerMembership($trainer, $alex);

        /** @var TokenLedgerService $ledger */
        $ledger = self::getContainer()->get(TokenLedgerService::class);
        $ledger->gift($trainer, $parent, 10, $trainer->getOwnerAccount(), 'I3 fixture.');
        $spend = $ledger->spend($trainer, $parent, 2, $alex, 'I3 fixture: spend.');

        $refund = $ledger->refund($trainer, $spend, 2, 'I3 fixture: refund.');

        self::assertSame($alex->getId(), $refund->getBeneficiaryPlayer()?->getId(), 'I3: the refund inherits the spend\'s own beneficiary.');
    }

    /**
     * I4: the sum of refunds against one spend never exceeds it — BR-05-5
     * permits a PARTIAL refund (so two smaller refunds may legally sum to
     * the total), but never more than the original.
     */
    public function testPartialRefundsAreAllowedButCannotExceedTheOriginalSpend(): void
    {
        self::bootKernel();
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $parent = $this->account('player@practiceperfect.test');
        $pat = $this->patPlayer();

        /** @var TokenLedgerService $ledger */
        $ledger = self::getContainer()->get(TokenLedgerService::class);
        $ledger->gift($trainer, $parent, 10, $trainer->getOwnerAccount(), 'I4 fixture.');
        $spend = $ledger->spend($trainer, $parent, 5, $pat, 'I4 fixture: a 5-token spend.');

        // BR-05-5: a partial refund is legal.
        $ledger->refund($trainer, $spend, 3, 'I4 fixture: partial refund 1.');
        // A second partial refund exactly exhausting the remainder is still legal.
        $ledger->refund($trainer, $spend, 2, 'I4 fixture: partial refund 2 (exhausts it).');

        $this->expectException(RefundExceedsSpendException::class);
        $ledger->refund($trainer, $spend, 1, 'I4 fixture: this one must be rejected — nothing left to refund.');
    }

    /**
     * I5: sign matches kind — proven here for the two signed kinds this
     * service exposes at the PHP level (the CHECK constraint is the
     * database-level backstop, covering every kind including `adjustment`).
     */
    public function testEntryAmountSignMatchesItsKind(): void
    {
        self::bootKernel();
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $parent = $this->account('player@practiceperfect.test');
        $pat = $this->patPlayer();

        /** @var TokenLedgerService $ledger */
        $ledger = self::getContainer()->get(TokenLedgerService::class);
        $gift = $ledger->gift($trainer, $parent, 4, $trainer->getOwnerAccount(), 'I5 fixture: gift.');
        self::assertSame(4, $gift->getAmount(), 'I5: purchase/gift/referral_reward/refund are positive.');

        $spend = $ledger->spend($trainer, $parent, 3, $pat, 'I5 fixture: spend.');
        self::assertSame(-3, $spend->getAmount(), 'I5: spend is always stored negative.');
    }

    /**
     * I6: at most one entry exists per (payment record, purpose) — outbound
     * idempotency at the ledger level, so a replayed webhook cannot
     * double-credit the same payment. Proven here via TokenLedgerService::purchase()'s
     * own idempotent re-check; the unique partial index is the database-level
     * backstop.
     */
    public function testPurchaseIsIdempotentPerPaymentRecord(): void
    {
        self::bootKernel();
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $parent = $this->account('player@practiceperfect.test');

        $paymentRecord = $this->completedTokenPurchaseRecord($trainer, $parent, 1000);

        /** @var TokenLedgerService $ledger */
        $ledger = self::getContainer()->get(TokenLedgerService::class);
        $before = $ledger->balanceFor($trainer, $parent);

        $first = $ledger->purchase($trainer, $parent, 10, $paymentRecord, 'I6 fixture: first credit.');
        $second = $ledger->purchase($trainer, $parent, 10, $paymentRecord, 'I6 fixture: replayed credit.');

        self::assertSame($first->getId(), $second->getId(), 'I6: a replay returns the SAME entry, never a second one.');
        self::assertSame($before + 10, $ledger->balanceFor($trainer, $parent), 'I6: the balance was credited exactly once, not twice.');
    }

    /**
     * AC-05-33: gifting is audit-logged.
     */
    public function testGiftIsAuditLogged(): void
    {
        self::bootKernel();
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $parent = $this->account('player@practiceperfect.test');

        /** @var TokenLedgerService $ledger */
        $ledger = self::getContainer()->get(TokenLedgerService::class);
        $ledger->gift($trainer, $parent, 7, $trainer->getOwnerAccount(), 'AC-05-33 fixture: audited gift.');

        /** @var AuditLogEntryRepository $auditLog */
        $auditLog = self::getContainer()->get(AuditLogEntryRepository::class);
        $entries = $auditLog->search('token.gift');

        self::assertNotEmpty($entries, 'AC-05-33: gifting tokens writes an audit log entry.');
    }

    /**
     * BR-05-12/A7: the `adjustment` kind is Super-Admin-only in practice
     * (enforced by the calling controller/voter — TokenLedgerService::adjustment()
     * itself takes whichever Account it is given, trusting the caller to have
     * already checked ROLE_SUPER_ADMIN), reason required, and always
     * audit-logged.
     */
    public function testAdjustmentRequiresAReasonAndIsAuditLogged(): void
    {
        self::bootKernel();
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $parent = $this->account('player@practiceperfect.test');
        $superAdmin = $this->account('admin@practiceperfect.test');

        /** @var TokenLedgerService $ledger */
        $ledger = self::getContainer()->get(TokenLedgerService::class);

        $this->expectException(\InvalidArgumentException::class);

        try {
            $ledger->adjustment($trainer, $parent, 5, $superAdmin, '');
        } finally {
            /** @var AuditLogEntryRepository $auditLog */
            $auditLog = self::getContainer()->get(AuditLogEntryRepository::class);
            // Not yet recorded — the guard fires before any write.
            self::assertEmpty(array_filter(
                $auditLog->search('token.adjustment'),
                static fn ($e): bool => '' === $e->getDetails()['reason'],
            ));
        }
    }

    public function testAdjustmentWithAReasonSucceedsAndIsAuditLogged(): void
    {
        self::bootKernel();
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $parent = $this->account('player@practiceperfect.test');
        $superAdmin = $this->account('admin@practiceperfect.test');

        /** @var TokenLedgerService $ledger */
        $ledger = self::getContainer()->get(TokenLedgerService::class);
        $before = $ledger->balanceFor($trainer, $parent);

        $entry = $ledger->adjustment($trainer, $parent, -1 * min(1, $before), $superAdmin, 'Reconciling an out-of-band Stripe Dashboard refund.');

        self::assertSame(TokenEntry::KIND_ADJUSTMENT, $entry->getKind());
        self::assertSame($superAdmin->getId(), $entry->getPerformedByAccount()?->getId());

        /** @var AuditLogEntryRepository $auditLog */
        $auditLog = self::getContainer()->get(AuditLogEntryRepository::class);
        self::assertNotEmpty($auditLog->search('token.adjustment'), 'BR-05-12: every adjustment is audit-logged.');
    }

    /**
     * I3: the database CHECK constraint (chk_token_entry_beneficiary_required)
     * is the structural backstop behind TokenEntry::spend()'s own
     * application-level guard — proven directly by attempting to bypass the
     * entity's own constructor guard is not possible from PHP (private
     * constructor, only named constructors), so this proves the DB side
     * instead: a hand-crafted INSERT with kind='spend' and no beneficiary
     * must fail.
     */
    public function testDatabaseRejectsASpendEntryWithNoBeneficiary(): void
    {
        self::bootKernel();
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $parent = $this->account('player@practiceperfect.test');

        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);

        // chk_token_entry_beneficiary_required is a CHECK constraint, not a
        // NOT NULL column (the column stays nullable at the type level so
        // gift/purchase/referral_reward/adjustment entries can omit it) —
        // the driver reports this as a generic constraint-violation
        // DriverException, not the more specific NotNullConstraintViolationException.
        $this->expectException(DriverException::class);
        $this->expectExceptionMessageMatches('/chk_token_entry_beneficiary_required/');

        $em->getConnection()->executeStatement(
            <<<'SQL'
                INSERT INTO token_entry (trainer_id, parent_account_id, kind, amount, description, created_at)
                VALUES (?, ?, 'spend', -1, 'Bypassing the beneficiary requirement.', now())
                SQL,
            [$trainer->getId(), $parent->getId()],
        );
    }

    private function completedTokenPurchaseRecord(Trainer $trainer, Account $payer, int $amountMinorUnits): PaymentRecord
    {
        $record = new PaymentRecord(
            $trainer,
            PaymentRecord::TYPE_TOKEN_PURCHASE,
            PaymentRecord::METHOD_CARD,
            $amountMinorUnits,
            $payer->getEmail(),
            $payer->getEmail(),
            $payer,
        );
        $package = $this->createTokenPackage($trainer, 'I6 fixture package', ['tokenCount' => 10, 'priceMinorUnits' => $amountMinorUnits]);
        $record->attachRelatedTokenPackage($package);
        $record->applyFee(500, 0);
        $record->markCompleted();

        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);
        $em->persist($record);
        $em->flush();

        return $record;
    }
}
