<?php

declare(strict_types=1);

namespace App\Tests\Billing;

use App\Billing\Repository\TrainerBillingSettingsRepository;
use App\Tests\Support\BillingFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-05.01 — Trainer Connects Stripe Account.
 */
final class StripeConnectTest extends WebTestCase
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
     * AC-05-3: "If a trainer has not connected Stripe, they cannot create
     * paid events... and cannot sell content, but can still create free
     * events." Exercised at the entity/service boundary that gates a
     * NON-ZERO price — EventService's own paid-pricing guard.
     *
     * Must run before testTrainerConnectsStripeAndSeesConnectedStatus()
     * below, and does — PHPUnit runs a class's test methods in declaration
     * order by default, and this suite's fixtures are loaded once per
     * `make test` run with no per-test transaction rollback (see
     * `test-db` in the Makefile), so once that test connects
     * 'peak-performance' to Stripe, it stays connected for the rest of
     * this process. Deliberately still uses 'peak-performance', not
     * 'baseline-athletics': the latter is never otherwise driven through
     * the real `/trainer/events/new` HTTP form anywhere in this suite, and
     * doing so here for the first time was observed to lazily provision a
     * self-`CoachMembership` row for it (`MembershipService::
     * ensureSelfCoachMembership()`, AC-02-8 — the coach-assignment
     * dropdown includes the trainer themselves) that then leaked into
     * `CoachListTenancyTest`, an Epic-01 test asserting 'baseline-athletics'
     * has zero coaches. 'peak-performance' already carries that same
     * self-membership from other, pre-existing Epic-02 tests, and every
     * test that asserts against its coach list already does so with a
     * containment check, not an exact/exclusive one — safe either way.
     */
    public function testUnconnectedTrainerCannotSetPaidEventPricingButCanCreateFreeEvents(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        $this->client->loginUser($trainer->getOwnerAccount());

        $crawler = $this->client->request('GET', '/trainer/events/new');
        $form = $crawler->selectButton('Create event')->form([
            'event[title]' => 'Blocked Paid Event Attempt',
            'event[eventType]' => 'training_session',
            'event[startsAt]' => (new \DateTimeImmutable('+3 days'))->format('Y-m-d\TH:i'),
            'event[endsAt]' => (new \DateTimeImmutable('+3 days +1 hour'))->format('Y-m-d\TH:i'),
            'event[location]' => 'Court 1',
            'event[capacity]' => '10',
            'event[usdPricingEnabled]' => '1',
            'event[usdPrice]' => '10.00',
        ]);
        $this->client->submit($form);

        self::assertSelectorTextContains('body', 'Connect Stripe first', 'AC-05-3: blocked with the message "Connect Stripe first".');

        // Free events remain unaffected — proven through the same real
        // controller/form path, not the entity-level createEvent() fixture
        // helper, so this actually exercises EventService's guard being
        // scoped to non-zero USD pricing rather than assumed to be.
        //
        // A fresh GET is deliberate, not incidental: KernelBrowser reboots
        // the kernel (a fresh container, a fresh EntityManager) before each
        // request unless disableReboot() was called, which this test never
        // does. That reboot is relied on here — EntityManager::
        // wrapInTransaction() (EventService::create()'s transaction
        // wrapper) closes the EntityManager on ANY exception thrown from
        // its callback, including this clean, caught
        // StripeNotConnectedException, so the container bound to the
        // blocked request above cannot safely run another Doctrine write
        // (persist()/flush() would fail with EntityManagerClosed — found
        // by hitting exactly that when a prior version of this test called
        // the createEvent() fixture helper immediately afterward, on the
        // same un-rebooted container). This second request's own reboot
        // sidesteps it entirely.
        $crawler = $this->client->request('GET', '/trainer/events/new');
        $freeForm = $crawler->selectButton('Create event')->form([
            'event[title]' => 'Still-Free Event While Unconnected',
            'event[eventType]' => 'training_session',
            'event[startsAt]' => (new \DateTimeImmutable('+4 days'))->format('Y-m-d\TH:i'),
            'event[endsAt]' => (new \DateTimeImmutable('+4 days +1 hour'))->format('Y-m-d\TH:i'),
            'event[location]' => 'Court 1',
            'event[capacity]' => '10',
            // AC-02-58: token pricing defaults ON (1 token) — must be
            // explicitly turned off here, same as usdPricingEnabled being
            // explicitly left off, to submit a genuinely free event rather
            // than a token-priced one.
            'event[tokenPricingEnabled]' => false,
        ]);
        $this->client->submit($freeForm);

        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var \App\Scheduling\Repository\EventRepository $events */
        $events = self::getContainer()->get(\App\Scheduling\Repository\EventRepository::class);
        $created = current(array_filter($events->findAllForActiveTenant(), static fn ($e) => 'Still-Free Event While Unconnected' === $e->getTitle()));
        self::assertNotFalse($created, 'AC-05-3: a free event is unaffected by not being connected to Stripe.');
        self::assertTrue($created->isFree());
        self::assertEmpty(array_filter($events->findAllForActiveTenant(), static fn ($e) => 'Blocked Paid Event Attempt' === $e->getTitle()), 'The paid event was never created.');
    }

    /**
     * AC-05-2: Stripe Connect configuration defaults — Express account
     * type (implicit in `StripeGateway::ensureConnectAccount()`'s own call
     * shape), 5% application fee, monthly payout schedule, $15/month
     * trainer subscription — all Super-Admin-configurable per trainer
     * (AC-05-27 exercises the edit itself).
     */
    public function testStripeConnectConfigurationDefaults(): void
    {
        $trainer = $this->trainer('baseline-athletics');
        $this->activateTenant($trainer);

        /** @var TrainerBillingSettingsRepository $settingsRepo */
        $settingsRepo = self::getContainer()->get(TrainerBillingSettingsRepository::class);
        $settings = $settingsRepo->getOrCreateForTrainer($trainer);

        self::assertSame(500, $settings->getPlatformFeeBasisPoints(), 'AC-05-2: application fee defaults to 5%.');
        self::assertSame('monthly', $settings->getPayoutSchedule(), 'AC-05-2: payout schedule defaults to monthly.');
        self::assertSame(1500, $settings->getMonthlySubscriptionPriceMinorUnits(), 'AC-05-2: trainer subscription defaults to $15/month.');
    }

    /**
     * AC-05-1: "clicks 'Connect Stripe'... is redirected to Stripe Connect
     * Express onboarding... is redirected back... sees status 'Stripe
     * Connected ✓'." Onboarding/KYC itself happens on Stripe's own hosted
     * pages — out of this platform's reach to test — so this proves the
     * platform's own three steps: the Connect redirect is issued, the
     * return route verifies against Stripe's own Account object (never
     * trusting the redirect alone), and the settings screen then shows the
     * connected status.
     *
     * Runs last, deliberately — see
     * testUnconnectedTrainerCannotSetPaidEventPricingButCanCreateFreeEvents()'s
     * own docblock above for why.
     */
    public function testTrainerConnectsStripeAndSeesConnectedStatus(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        // KernelBrowser reboots the kernel (a fresh container, and so a
        // fresh InMemoryStripeClient with no configured state) before each
        // request by default — disableReboot() keeps this test's own
        // fakeStripeClient() configuration alive across the whole flow,
        // matching RsvpTest::testPaidEventConfirmsOnceGatewayReportsSuccess()'s
        // own precedent.
        $this->client->disableReboot();
        $this->client->loginUser($trainer->getOwnerAccount());

        $crawler = $this->client->request('GET', '/trainer/billing');
        self::assertSelectorTextContains('body', 'Stripe not connected');

        $form = $crawler->selectButton('Connect Stripe')->form();
        $this->client->submit($form);

        self::assertTrue($this->client->getResponse()->isRedirect(), 'AC-05-1: redirected to Stripe Connect Express onboarding.');
        $onboardingUrl = (string) $this->client->getResponse()->headers->get('Location');
        self::assertStringContainsString('connect.stripe.test', $onboardingUrl);

        // Simulates completing Stripe's own hosted KYC flow — out of this
        // platform's reach — then being redirected back.
        $this->activateTenant($trainer);
        /** @var TrainerBillingSettingsRepository $settingsRepo */
        $settingsRepo = self::getContainer()->get(TrainerBillingSettingsRepository::class);
        $settings = $settingsRepo->getOrCreateForTrainer($trainer);
        self::assertNotNull($settings->getStripeConnectAccountId(), 'The Connect account was created at the "Connect Stripe" step.');
        $this->fakeStripeClient()->setAccountOnboardingComplete($settings->getStripeConnectAccountId());

        $this->client->request('GET', '/trainer/billing/stripe/return');
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', 'Stripe Connected ✓', 'AC-05-1: sees status "Stripe Connected ✓" after onboarding completes.');
    }
}
