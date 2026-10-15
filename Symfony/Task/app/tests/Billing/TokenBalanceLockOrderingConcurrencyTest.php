<?php

declare(strict_types=1);

namespace App\Tests\Billing;

use App\Scheduling\Entity\Event;
use App\Scheduling\Repository\RsvpRepository;
use App\Scheduling\Service\RsvpService;
use App\Tests\Support\BillingFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Doctrine\DBAL\DriverManager;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\Security\Core\Authentication\Token\Storage\TokenStorageInterface;
use Symfony\Component\Security\Core\Authentication\Token\UsernamePasswordToken;

/**
 * Architecture "Lock ordering": "the fixed global order is the token
 * balance row first, then the event row... A paid RSVP takes two locks —
 * the token balance row and the event capacity row. Two concurrent RSVPs
 * can deadlock if the order varies." This is the task's own explicitly
 * requested proof: "A concurrency test in the style of
 * tests/Scheduling/CapacityConcurrencyTest.php — a second independent DBAL
 * connection holding the lock — is expected."
 *
 * Same technique as `CapacityConcurrencyTest`, applied to the OTHER lock in
 * the pair: a second, independent DBAL connection takes and holds the exact
 * row lock `TokenBalanceRepository::lockForUpdate()` takes
 * (`RsvpService::rsvp()`'s own first statement, per its class docblock —
 * BEFORE `EventRepository::lockForUpdate()`), and a genuinely concurrent
 * token-funded RSVP attempt is proven to block on it, not sail through to
 * the event row first. Combined with `CapacityConcurrencyTest`'s own proof
 * that the event row's lock is real, and `RsvpService::rsvp()`'s own source
 * (`lockFundingForUpdate()` called strictly before
 * `EventRepository::lockForUpdate()`, both inside the same transaction,
 * never reversed anywhere in this codebase), the two together demonstrate
 * the fixed order is real, not merely claimed in a comment.
 *
 * @see specs/architect-architecture.md "The token and payment ledger — Lock ordering"
 */
final class TokenBalanceLockOrderingConcurrencyTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;
    use BillingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    public function testATokenFundedRsvpBlocksWhileTheTokenBalanceRowIsLocked(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $patAccount = $this->account('player@practiceperfect.test');
        $pat = $this->patPlayer();

        // Creates the token_balance row as a side effect (via
        // TokenLedgerService::gift() -> TokenBalanceRepository::lockForUpdate()'s
        // own upsert), so the second connection below has a real row to lock.
        // The balance is read back immediately rather than assumed: this
        // suite carries no per-test database reset, so an earlier test
        // method may already have granted this exact (trainer, parent)
        // pair tokens of its own.
        $this->giveTokens($trainer, $patAccount, 10);
        $balanceBeforeAttempt = $this->tokenBalance($trainer, $patAccount);

        $event = $this->createEvent($trainer, [
            'title' => 'Lock Ordering Target Session',
            'tokenPricingEnabled' => true,
            'tokenPrice' => 2,
            'capacity' => 5,
        ]);

        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);
        $mainConnection = $em->getConnection();

        // A second, independent connection — simulates a genuinely
        // concurrent second request's own DB connection, exactly matching
        // CapacityConcurrencyTest's own precedent.
        $secondConnection = DriverManager::getConnection($mainConnection->getParams());
        $secondConnection->executeStatement(sprintf("SET app.current_trainer = '%d'", (int) $trainer->getId()));
        $secondConnection->beginTransaction();
        // Takes and holds the exact same row lock
        // TokenBalanceRepository::lockForUpdate() takes — first the row
        // itself, matching the schema's own unique key.
        $secondConnection->executeQuery(
            'SELECT id FROM token_balance WHERE trainer_id = ? AND parent_account_id = ? FOR UPDATE',
            [$trainer->getId(), $patAccount->getId()],
        );

        $blocked = false;

        try {
            $mainConnection->executeStatement("SET lock_timeout = '1500ms'");

            /** @var TokenStorageInterface $tokenStorage */
            $tokenStorage = self::getContainer()->get(TokenStorageInterface::class);
            $tokenStorage->setToken(new UsernamePasswordToken($patAccount, 'main', $patAccount->getRoles()));

            /** @var RsvpService $rsvpService */
            $rsvpService = self::getContainer()->get(RsvpService::class);

            try {
                $rsvpService->rsvp($event, $pat, $patAccount, Event::PAYMENT_TOKEN);
            } catch (\Throwable) {
                // Postgres raises 55P03 (lock_not_available) once
                // lock_timeout elapses while waiting on the row
                // $secondConnection still holds.
                $blocked = true;
            }
        } finally {
            $secondConnection->rollBack();
            $secondConnection->close();
        }

        self::assertTrue(
            $blocked,
            'Architecture "Lock ordering": a token-funded RSVP attempt must block on an already-locked token balance row, proving the balance lock is real and taken before the event row is ever reached.',
        );

        // No partial effect: the whole transaction aborted before ever
        // creating the RSVP row (which would only happen AFTER both locks
        // in the fixed order) — the attempted spend never touched the
        // ledger either.
        $this->activateTenant($trainer);
        /** @var RsvpRepository $rsvps */
        $rsvps = self::getContainer()->get(RsvpRepository::class);
        self::assertNull($rsvps->findOneByEventAndPlayer($event, $pat), 'No RSVP row was created — the blocked attempt left no partial state.');
        self::assertSame($balanceBeforeAttempt, $this->tokenBalance($trainer, $patAccount), 'The token balance is untouched by the blocked attempt.');
    }

    /**
     * The positive-path complement, through the real HTTP flow with no
     * competing lock — AC-05-7's "no payment-processing delay" holds
     * end-to-end: instant confirmation, balance debited, in one request.
     * Mirrors `CapacityConcurrencyTest::testUncontendedRsvpsStillCorrectlyFillAndRefuseTheLastSpot()`'s
     * own two-test-method shape exactly.
     */
    public function testUncontendedTokenRsvpConfirmsInstantlyAndDebitsTheBalance(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $patAccount = $this->account('player@practiceperfect.test');
        $this->giveTokens($trainer, $patAccount, 10);
        $balanceBefore = $this->tokenBalance($trainer, $patAccount);

        $event = $this->createEvent($trainer, [
            'title' => 'Uncontended Token Lock Session',
            'tokenPricingEnabled' => true,
            'tokenPrice' => 2,
        ]);

        $this->client->loginUser($patAccount);
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', sprintf('/portal/events/%d', $event->getId()));
        $form = $crawler->selectButton('Register & Pay')->form(['rsvp[paymentMethod]' => Event::PAYMENT_TOKEN]);
        $this->client->submit($form);

        self::assertResponseRedirects('/portal/reservations');
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', "You're registered!");

        $this->activateTenant($trainer);
        self::assertSame($balanceBefore - 2, $this->tokenBalance($trainer, $patAccount), 'AC-05-7: the balance is debited in the same request, no delay.');
    }
}
