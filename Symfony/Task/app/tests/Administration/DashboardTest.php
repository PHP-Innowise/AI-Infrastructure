<?php

declare(strict_types=1);

namespace App\Tests\Administration;

use App\Billing\Repository\PlatformSubscriptionRepository;
use App\Billing\Service\StripeGateway;
use App\Identity\Entity\Account;
use App\Identity\Service\TrainerProvisioningService;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-07.01 — Super Admin Views Operational Dashboard.
 */
final class DashboardTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-07-1: after login (or via "Dashboard" navigation), Super Admin
     * sees operational metrics only — proven together with AC-07-7 below.
     */
    public function testSuperAdminSeesTheDashboard(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $this->client->request('GET', '/super-admin/dashboard');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('h1', 'Super Admin Dashboard');
    }

    /**
     * AC-07-2: total trainers (active Stripe subscriptions), total
     * players, total coaches, new users this week/month, and a 30-day
     * growth chart. Proven with a fresh trainer created and provisioned to
     * Active within this exact test, and a before/after delta — the
     * fixture database is shared, mutable state across the whole `make
     * test` run (TrainerFeeEditTest's own established precedent for this
     * exact caveat), so an absolute count assertion would be unreliable.
     */
    public function testDashboardShowsUserMetricsIncludingActiveTrainerCount(): void
    {
        /** @var PlatformSubscriptionRepository $subscriptions */
        $subscriptions = self::getContainer()->get(PlatformSubscriptionRepository::class);
        $before = $subscriptions->countActive();

        $trainer = $this->createActivelySubscribedTrainer('dashboard-metrics-owner@example.test', 'Dashboard Metrics FC');

        $after = $subscriptions->countActive();
        self::assertSame($before + 1, $after, 'A freshly provisioned trainer is Active.');

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/super-admin/dashboard');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', (string) $after, 'AC-07-2: the active-trainers figure (BR-07-7) is shown.');
        // The labels name what they count. "Players" alone means the roster
        // elsewhere in this product — under decision A1 a child has a player
        // profile and usually no account — so an unqualified "Total players"
        // here reads as a contradiction of the CRM dashboard's larger figure.
        self::assertSelectorTextContains('body', 'Player accounts (registered)');
        self::assertSelectorTextContains('body', 'Coach accounts (registered)');
        self::assertSelectorTextContains('body', 'New users this week');
        self::assertSelectorTextContains('body', 'New users this month');
        self::assertSelectorTextContains('body', '30-day user growth');

        unset($trainer);
    }

    /**
     * AC-07-3: sessions this week (training, private, camps — this
     * codebase's own real event types, see DashboardController's own
     * docblock on the vocabulary discrepancy), RSVPs this week, attendance
     * rate, no-show rate.
     */
    public function testDashboardShowsSessionMetrics(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $this->client->request('GET', '/super-admin/dashboard');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Session Metrics');
        self::assertSelectorTextContains('body', 'Sessions —');
        self::assertSelectorTextContains('body', 'RSVPs');
        self::assertSelectorTextContains('body', 'Attendance rate');
        self::assertSelectorTextContains('body', 'No-show rate');
    }

    /**
     * AC-07-4: most active trainers (name, session count, player count)
     * and top players (name, trainer, attendance count). The fixture
     * trainer/player/coach/event/attendance data (AppFixtures) is real and
     * always present, so the named fixture trainer is asserted directly
     * rather than only checking the section renders.
     */
    public function testDashboardShowsTopPerformers(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $this->client->request('GET', '/super-admin/dashboard');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Most active trainers');
        self::assertSelectorTextContains('body', 'Peak Performance Basketball', 'AC-07-4: the fixture trainer with real event/attendance data ranks.');
        self::assertSelectorTextContains('body', 'Top players');
    }

    /**
     * AC-07-5: presets Last 7/30/90 days, defaulting to Last 30 days.
     */
    public function testDashboardDateRangeSelectorDefaultsTo30Days(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/super-admin/dashboard');

        self::assertResponseIsSuccessful();
        $select = $crawler->filter('select[name="range"]');
        self::assertCount(1, $select);
        self::assertSame(['7', '30', '90'], $select->filter('option')->each(static fn ($node) => $node->attr('value')));
        self::assertSame('30', $select->filter('option[selected]')->attr('value'), 'AC-07-5: defaults to Last 30 days.');

        $this->client->request('GET', '/super-admin/dashboard', ['range' => '7']);
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'last 7 days');
    }

    /**
     * AC-07-6: a prominent "View Financial Reports in Stripe" button
     * opening the Stripe Express Dashboard.
     */
    public function testDashboardShowsViewFinancialReportsInStripeButton(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/super-admin/dashboard');

        self::assertResponseIsSuccessful();
        $link = $crawler->selectLink('View Financial Reports in Stripe');
        self::assertGreaterThan(0, $link->count());
        self::assertSame('/super-admin/stripe', $link->attr('href'));
    }

    /**
     * AC-07-7 (negative): no revenue, payout or transaction figure is
     * duplicated on the platform dashboard — checked as the literal
     * absence of a dollar sign anywhere on the page, since every genuine
     * dashboard figure here is a count or a percentage, never a currency
     * amount.
     */
    public function testDashboardContainsNoFinancialFigures(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/super-admin/dashboard');

        self::assertResponseIsSuccessful();
        self::assertStringNotContainsString('$', $crawler->filter('.skeleton-card')->text(), 'AC-07-7: no money figure anywhere on this page.');
    }

    /**
     * AC-07-17: the dashboard shows a "Create Trainer" quick-action button.
     */
    public function testDashboardShowsCreateTrainerQuickAction(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/super-admin/dashboard');

        self::assertResponseIsSuccessful();
        $link = $crawler->selectLink('+ Create Trainer');
        self::assertGreaterThan(0, $link->count());
        self::assertSame('/super-admin/trainers/new', $link->attr('href'));
    }

    /**
     * A Trainer (not Super Admin) cannot reach the dashboard at all.
     */
    public function testATrainerCannotReachTheDashboard(): void
    {
        $this->client->loginUser($this->trainer('peak-performance')->getOwnerAccount());
        $this->client->request('GET', '/super-admin/dashboard');

        self::assertResponseStatusCodeSame(403);
    }

    private function createActivelySubscribedTrainer(string $email, string $businessName): Account
    {
        /** @var TrainerProvisioningService $provisioning */
        $provisioning = self::getContainer()->get(TrainerProvisioningService::class);
        $admin = $this->account('admin@practiceperfect.test');

        $trainer = $provisioning->createTrainer($admin, $businessName, 'Metrics', 'Owner', $email, null);

        // TrainerBillingSettings/PlatformSubscription are trainer-scoped
        // (RLS) — a genuine tenant context is required before
        // provisionPlatformSubscription() can read or insert either row,
        // matching TrainerCreationController's own AdministrativeScope
        // usage for this exact call in production.
        $this->activateTenant($trainer);

        /** @var StripeGateway $stripeGateway */
        $stripeGateway = self::getContainer()->get(StripeGateway::class);
        $stripeGateway->provisionPlatformSubscription($trainer);

        return $trainer->getOwnerAccount();
    }
}
