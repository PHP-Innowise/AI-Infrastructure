<?php

declare(strict_types=1);

namespace App\Tests\Billing;

use App\Billing\Service\TokenLedgerService;
use App\Tests\Support\BillingFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Test\KernelTestCase;

/**
 * I7 / architect-architecture.md "Entry immutability": "Database privilege
 * — the application role holds INSERT and SELECT only on entries and the
 * audit log." This is the task's own explicitly-required proof: "Write a
 * test proving an UPDATE attempt fails — that is the proof the invariant
 * is structural rather than a convention."
 *
 * Connects as the SAME role the running application uses (`pp_app`, via
 * the ordinary `EntityManagerInterface`/DBAL connection this test container
 * is wired with — no special credentials) and issues a raw UPDATE directly
 * against `token_entry`. A convention (no setters, no repository
 * `update()` method) would still let a stray raw query through; only a
 * REVOKE actually stops one.
 *
 * @see specs/database-designer-schema.md "`token_entry`" — "I7 (append-only) is a database privilege, not a code convention"
 */
final class TokenEntryAppendOnlyPrivilegeTest extends KernelTestCase
{
    use FixtureHelpers;
    use BillingFixtureHelpers;

    public function testUpdatingAPersistedTokenEntryIsRejectedByDatabasePrivilege(): void
    {
        self::bootKernel();

        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $parent = $this->account('player@practiceperfect.test');

        /** @var TokenLedgerService $ledger */
        $ledger = self::getContainer()->get(TokenLedgerService::class);
        $entry = $ledger->gift($trainer, $parent, 5, $trainer->getOwnerAccount(), 'Append-only privilege test fixture.');

        \assert(null !== $entry->getId());

        /** @var EntityManagerInterface $entityManager */
        $entityManager = self::getContainer()->get(EntityManagerInterface::class);
        $connection = $entityManager->getConnection();

        $this->expectException(\Throwable::class);
        $this->expectExceptionMessageMatches('/permission denied/i');

        // The application's own connection, the same pp_app role every
        // request runs as — REVOKE UPDATE, DELETE ON token_entry FROM
        // pp_app (Version20260811100000) is what must make this fail.
        $connection->executeStatement(
            'UPDATE token_entry SET amount = amount + 1 WHERE id = ?',
            [$entry->getId()],
        );
    }

    public function testDeletingAPersistedTokenEntryIsRejectedByDatabasePrivilege(): void
    {
        self::bootKernel();

        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $parent = $this->account('player@practiceperfect.test');

        /** @var TokenLedgerService $ledger */
        $ledger = self::getContainer()->get(TokenLedgerService::class);
        $entry = $ledger->gift($trainer, $parent, 5, $trainer->getOwnerAccount(), 'Append-only privilege test fixture (delete).');

        \assert(null !== $entry->getId());

        /** @var EntityManagerInterface $entityManager */
        $entityManager = self::getContainer()->get(EntityManagerInterface::class);
        $connection = $entityManager->getConnection();

        $this->expectException(\Throwable::class);
        $this->expectExceptionMessageMatches('/permission denied/i');

        $connection->executeStatement('DELETE FROM token_entry WHERE id = ?', [$entry->getId()]);
    }
}
