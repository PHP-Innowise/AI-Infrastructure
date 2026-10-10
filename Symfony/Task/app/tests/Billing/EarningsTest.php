<?php

declare(strict_types=1);

namespace App\Tests\Billing;

use App\Billing\Repository\TrainerBillingSettingsRepository;
use App\Billing\Stripe\StripeEarningsSummary;
use App\Tests\Support\BillingFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-05.09 — Trainer Views Payout History.
 */
final class EarningsTest extends WebTestCase
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
     * AC-05-25: a connected trainer sees a simplified in-platform summary
     * — current-period earnings, next payout date, last payout amount, and
     * lifetime earnings — read live from Stripe (StripeReportingReader,
     * never a locally-summed figure), plus a link to the full Stripe
     * Express Dashboard.
     *
     * Uses 'baseline-athletics', not 'peak-performance': StripeConnectTest's
     * own AC-05-1 lifecycle test needs 'peak-performance' to still be
     * UNCONNECTED when it runs (it is the one file that owns that trainer's
     * connection-state transition), and file execution runs this test
     * first alphabetically ('Billing/EarningsTest.php' before
     * 'Billing/StripeConnectTest.php') — connecting 'peak-performance' here
     * would leave it permanently connected before that other test ever
     * gets to observe it unconnected. 'baseline-athletics' has no such
     * lifecycle test depending on it, only a static-defaults check
     * (StripeConnectTest::testStripeConnectConfigurationDefaults(), which
     * never touches connection status) — connecting it here is safe.
     */
    public function testConnectedTrainerSeesTheEarningsSummaryAndADashboardLink(): void
    {
        $trainer = $this->trainer('baseline-athletics');
        $this->activateTenant($trainer);
        $this->connectStripe($trainer);

        /** @var TrainerBillingSettingsRepository $settingsRepo */
        $settingsRepo = self::getContainer()->get(TrainerBillingSettingsRepository::class);
        $settings = $settingsRepo->getOrCreateForTrainer($trainer);
        \assert(null !== $settings->getStripeConnectAccountId());

        $this->fakeStripeClient()->setConnectBalanceSummary($settings->getStripeConnectAccountId(), new StripeEarningsSummary(
            currentPeriodEarningsMinorUnits: 123_45,
            nextPayoutDate: new \DateTimeImmutable('2026-09-01'),
            lastPayoutAmountMinorUnits: 456_78,
            lifetimeEarningsMinorUnits: 9_999_00,
        ));

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', '/trainer/billing/earnings');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', '$123.45', 'AC-05-25: current-period earnings.');
        self::assertSelectorTextContains('body', '2026-09-01', 'AC-05-25: next payout date.');
        self::assertSelectorTextContains('body', '$456.78', 'AC-05-25: last payout amount.');
        self::assertSelectorTextContains('body', '$9999.00', 'AC-05-25: lifetime earnings.');

        $link = $crawler->filter('a[href*="dashboard.stripe.com"]');
        self::assertGreaterThan(0, $link->count(), 'AC-05-25/26: links to the full Stripe Express Dashboard.');
    }

    /**
     * AC-05-25: an unconnected trainer is prompted to connect Stripe first
     * — never a locally-summed fallback (architect-architecture.md "The
     * earnings boundary"). Uses 'peak-performance', which this file's own
     * other two tests never connect — see this class's own comment on the
     * previous test for the full reasoning; genuinely safe here since this
     * test only READS connection status, never mutates it.
     */
    public function testUnconnectedTrainerIsPromptedToConnectStripeInstead(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', '/trainer/billing/earnings');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Connect Stripe to see your earnings.');
        self::assertSelectorTextNotContains('body', '$', 'No money figure is shown — never a locally-summed fallback.');
    }

    /**
     * AC-05-26: the Stripe Express Dashboard is stated as the source of
     * truth for detailed transaction history, payout/bank details, tax
     * documents, and fee breakdowns — the platform links out rather than
     * duplicating any of it.
     */
    public function testEarningsPageStatesStripeDashboardIsTheSourceOfTruth(): void
    {
        $trainer = $this->trainer('baseline-athletics');
        $this->activateTenant($trainer);
        $this->connectStripe($trainer);

        $this->client->loginUser($trainer->getOwnerAccount());
        $this->client->request('GET', '/trainer/billing/earnings');

        self::assertSelectorTextContains('body', 'source of truth', 'AC-05-26: Stripe Dashboard is stated as the source of truth.');
        self::assertSelectorTextContains('body', 'tax documents');
        self::assertSelectorTextContains('body', 'payout history');
    }
}
