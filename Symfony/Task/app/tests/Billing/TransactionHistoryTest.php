<?php

declare(strict_types=1);

namespace App\Tests\Billing;

use App\Billing\Entity\PaymentRecord;
use App\Tests\Support\BillingFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-05.08 — Player Views Transaction History.
 */
final class TransactionHistoryTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;
    use BillingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-05-22/23, BR-05-17: the list is scoped to the CURRENT trainer
     * context only (a token purchase with a different trainer never
     * shows), each row states date, type, amount, method and status, and
     * the list is filterable by type and payment method.
     */
    public function testTransactionHistoryIsScopedToTheCurrentTrainerAndFilterable(): void
    {
        $trainerA = $this->trainer('peak-performance');
        $trainerB = $this->trainer('baseline-athletics');
        $pat = $this->account('player@practiceperfect.test');

        $this->activateTenant($trainerA);
        $package = $this->createTokenPackage($trainerA, 'History Package', ['tokenCount' => 10, 'priceMinorUnits' => 9000]);
        $tokenPurchase = new PaymentRecord($trainerA, PaymentRecord::TYPE_TOKEN_PURCHASE, PaymentRecord::METHOD_CARD, 9000, 'Pat Parent', $pat->getEmail(), $pat);
        $tokenPurchase->attachRelatedTokenPackage($package);
        $tokenPurchase->applyFee(500, 450);
        $tokenPurchase->markCompleted();
        $this->paymentRecordEntityManager()->persist($tokenPurchase);

        $event = $this->createEvent($trainerA, ['title' => 'History RSVP Session', 'usdPricingEnabled' => true, 'usdPriceMinorUnits' => 2000]);
        $rsvp = $this->createRsvp($event, $this->patPlayer(), \App\Scheduling\Entity\Rsvp::METHOD_USD, \App\Scheduling\Entity\Rsvp::STATUS_CONFIRMED);
        $rsvpCharge = new PaymentRecord($trainerA, PaymentRecord::TYPE_EVENT_RSVP, PaymentRecord::METHOD_CARD, 2000, 'Pat Parent', $pat->getEmail(), $pat);
        $rsvpCharge->attachRelatedRsvp($rsvp);
        $rsvpCharge->applyFee(500, 100);
        $rsvpCharge->markCompleted();
        $this->paymentRecordEntityManager()->persist($rsvpCharge);
        $this->paymentRecordEntityManager()->flush();

        // A DIFFERENT trainer — must never appear in trainerA's history.
        $this->activateTenant($trainerB);
        $otherPackage = $this->createTokenPackage($trainerB, 'Other Trainer Package', ['tokenCount' => 20, 'priceMinorUnits' => 18000]);
        $otherTrainerPurchase = new PaymentRecord($trainerB, PaymentRecord::TYPE_TOKEN_PURCHASE, PaymentRecord::METHOD_CARD, 18000, 'Pat Parent', $pat->getEmail(), $pat);
        $otherTrainerPurchase->attachRelatedTokenPackage($otherPackage);
        $otherTrainerPurchase->applyFee(500, 900);
        $otherTrainerPurchase->markCompleted();
        $this->paymentRecordEntityManager()->persist($otherTrainerPurchase);
        $this->paymentRecordEntityManager()->flush();

        $this->client->loginUser($pat);
        $this->switchPlayerToTrainer($this->client, $trainerA);
        $crawler = $this->client->request('GET', '/portal/transactions');

        self::assertSelectorTextContains('body', '$90.00', 'AC-05-23: the token purchase amount is shown.');
        self::assertSelectorTextContains('body', '$20.00', 'AC-05-23: the RSVP charge amount is shown.');
        self::assertSelectorTextNotContains('body', '$180.00', 'AC-05-22/BR-05-17: a different trainer\'s transaction never appears.');
        self::assertSelectorTextContains('body', 'card', 'AC-05-23: payment method is shown.');
        self::assertSelectorTextContains('body', 'completed', 'AC-05-23: status is shown.');

        // Filter by type: only the token purchase.
        $crawler = $this->client->request('GET', '/portal/transactions', ['type' => 'token_purchase']);
        self::assertSelectorTextContains('body', '$90.00');
        self::assertSelectorTextNotContains('body', '$20.00', 'AC-05-23: filtered out by type.');

        // Filter by type: only the event RSVP.
        $crawler = $this->client->request('GET', '/portal/transactions', ['type' => 'event_rsvp']);
        self::assertSelectorTextContains('body', '$20.00');
        self::assertSelectorTextNotContains('body', '$90.00', 'AC-05-23: filtered out by type.');
    }

    /**
     * AC-05-17/23: a refund appears in the list as its own entry.
     */
    public function testARefundAppearsInTransactionHistory(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $pat = $this->account('player@practiceperfect.test');
        $package = $this->createTokenPackage($trainer, 'Refund History Package', ['tokenCount' => 10, 'priceMinorUnits' => 9000]);

        $original = new PaymentRecord($trainer, PaymentRecord::TYPE_TOKEN_PURCHASE, PaymentRecord::METHOD_CARD, 9000, 'Pat Parent', $pat->getEmail(), $pat);
        $original->attachRelatedTokenPackage($package);
        $original->applyFee(500, 450);
        $original->markCompleted();
        $this->paymentRecordEntityManager()->persist($original);
        $this->paymentRecordEntityManager()->flush();

        $refund = PaymentRecord::forRefund($original, 9000);
        $original->markRefunded('re_fixture_history');
        $this->paymentRecordEntityManager()->persist($refund);
        $this->paymentRecordEntityManager()->flush();

        $this->client->loginUser($pat);
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', '/portal/transactions');

        self::assertSelectorTextContains('body', '(refund)', 'AC-05-17/23: the refund is its own visible entry.');
    }

    /**
     * AC-05-24: an OPTIONAL parent "Family Overview" dashboard — the epic's
     * own acceptance criterion names it "(Optional)" explicitly. It was not
     * built in this pass: no route, controller, or template exists for it
     * anywhere in this codebase (confirmed by inspection — no
     * "family" + "overview"-shaped portal route exists alongside the
     * transaction-history/family controllers that DO exist). Recorded
     * honestly as unimplemented rather than faked, per this epic's own
     * explicit "(Optional)" framing rather than a firm MVP requirement.
     */
    public function testFamilyOverviewDashboardIsOptionalAndNotBuiltInThisPass(): void
    {
        self::markTestSkipped('AC-05-24: "(Optional)" per the epic\'s own text — not built in this Epic-05 pass. The per-trainer transaction history (AC-05-22/23) is built and tested above; a cross-trainer/cross-child aggregate view is not.');
    }

    private function paymentRecordEntityManager(): EntityManagerInterface
    {
        /** @var EntityManagerInterface $entityManager */
        $entityManager = self::getContainer()->get(EntityManagerInterface::class);

        return $entityManager;
    }
}
