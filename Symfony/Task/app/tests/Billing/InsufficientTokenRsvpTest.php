<?php

declare(strict_types=1);

namespace App\Tests\Billing;

use App\Billing\Repository\PaymentRecordRepository;
use App\Scheduling\Entity\Event;
use App\Scheduling\Repository\RsvpRepository;
use App\Tests\Support\BillingFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * AC-05-8, over HTTP: "If the player has insufficient tokens, the system
 * shows the shortfall ('You have 1 token, need 2 tokens') and offers 'Buy
 * More Tokens'... or 'Pay with Card Instead'."
 *
 * This is the test the criterion needed and did not have. AC-05-8 was
 * claimed by `TokenLedgerServiceTest::testSpendBeyondBalanceIsRejectedBeforeAnyWrite()`,
 * which asserts that the *service* throws with the shortfall on it — true,
 * and passing, while the product itself answered this request with HTTP 500
 * (`EntityManagerClosed`, raised by the gateway flushing after Doctrine had
 * closed the manager on rollback) and never rendered the message at all.
 *
 * Every assertion here is deliberately about what the player experiences:
 * the status, the sentence, the two offers, and the absence of any
 * half-created registration.
 *
 * @see specs/requirements-analyst-epic-05-payments-tokens-spec.md AC-05-8
 */
final class InsufficientTokenRsvpTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;
    use BillingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    public function testAnRsvpWithTooFewTokensShowsTheShortfallInsteadOfFailing(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $parent = $this->account('player@practiceperfect.test');

        // One token in hand, two required — AC-05-8's own example numbers.
        $this->setTokenBalanceTo($trainer, $parent, 1);

        $event = $this->createEvent($trainer, [
            'title' => 'Two Token Clinic',
            'tokenPricingEnabled' => true,
            'tokenPrice' => 2,
        ]);

        $this->client->loginUser($parent);
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', sprintf('/portal/events/%d', $event->getId()));
        $form = $crawler->selectButton('Register & Pay')->form(['rsvp[paymentMethod]' => Event::PAYMENT_TOKEN]);
        $this->client->submit($form);

        self::assertResponseRedirects(sprintf('/portal/events/%d', $event->getId()));
        $crawler = $this->client->followRedirect();

        self::assertResponseIsSuccessful('A shortfall is an answer, not a server error.');
        self::assertSelectorTextContains('body', 'You have 1 token, need 2 tokens.', 'AC-05-8: the shortfall itself.');

        // AC-05-8's first offer. The second, "Pay with Card Instead", is
        // the usd option in the payment-method choice, which this event
        // does not carry — asserted on its own below.
        self::assertGreaterThan(
            0,
            $crawler->selectLink('Buy More Tokens')->count(),
            'AC-05-8: the way out of the shortfall is on the page.',
        );
    }

    /**
     * AC-05-8's second offer, on an event priced both ways.
     */
    public function testAnEventPricedBothWaysStillOffersTheCardAfterAShortfall(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $parent = $this->account('player@practiceperfect.test');
        $this->setTokenBalanceTo($trainer, $parent, 0);

        $event = $this->createEvent($trainer, [
            'title' => 'Dual Priced Clinic',
            'tokenPricingEnabled' => true,
            'tokenPrice' => 1,
            'usdPricingEnabled' => true,
            'usdPriceMinorUnits' => 2500,
        ]);

        $this->client->loginUser($parent);
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', sprintf('/portal/events/%d', $event->getId()));
        $this->client->submit($crawler->selectButton('Register & Pay')->form(['rsvp[paymentMethod]' => Event::PAYMENT_TOKEN]));

        $crawler = $this->client->followRedirect();
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'You have 0 tokens, need 1 token.');

        $methods = $crawler->filter('input[name="rsvp[paymentMethod]"]')->each(
            static fn ($node): string => (string) $node->attr('value'),
        );
        self::assertContains(Event::PAYMENT_USD, $methods, 'AC-05-8: "Pay with Card Instead" is still available.');
    }

    /**
     * The half-registration this must never leave behind: an RSVP the
     * player did not pay for, or a payment record for a spend that never
     * happened. Both would show up on their reservations and transaction
     * history as things they have to reason about.
     */
    public function testARejectedRsvpCreatesNoRegistrationAndNoPaymentRecord(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $parent = $this->account('player@practiceperfect.test');
        $this->setTokenBalanceTo($trainer, $parent, 0);
        $pat = $this->patPlayer();

        $paymentRecordsBefore = $this->paymentRecordCount();

        $event = $this->createEvent($trainer, [
            'title' => 'Unaffordable Clinic',
            'tokenPricingEnabled' => true,
            'tokenPrice' => 3,
        ]);

        $this->client->loginUser($parent);
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', sprintf('/portal/events/%d', $event->getId()));
        $this->client->submit($crawler->selectButton('Register & Pay')->form(['rsvp[paymentMethod]' => Event::PAYMENT_TOKEN]));
        $this->client->followRedirect();

        $this->activateTenant($trainer);

        /** @var RsvpRepository $rsvps */
        $rsvps = self::getContainer()->get(RsvpRepository::class);
        self::assertNull($rsvps->findOneByEventAndPlayer($event, $pat), 'No RSVP row for a registration that was refused.');
        self::assertSame($paymentRecordsBefore, $this->paymentRecordCount(), 'No payment record for a spend that never happened.');
        self::assertSame(0, $this->tokenBalance($trainer, $parent), 'The balance is untouched.');
    }

    /**
     * Read fresh each time: `WebTestCase` reboots the kernel around a
     * request, so a repository fetched before one is bound to a container
     * that no longer resolves the active tenant, and every scoped read
     * through it comes back empty.
     */
    private function paymentRecordCount(): int
    {
        /** @var PaymentRecordRepository $paymentRecords */
        $paymentRecords = self::getContainer()->get(PaymentRecordRepository::class);

        return \count($paymentRecords->findAll());
    }
}
