<?php

declare(strict_types=1);

namespace App\Tests\Administration;

use App\Billing\Service\StripeGateway;
use App\Identity\Service\TrainerProvisioningService;
use App\Platform\Entity\Trainer;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-07.09 — Super Admin Links to Stripe Dashboard. AC-07-6/AC-07-36
 * (no financial data on the platform, only a link-out) are also proven
 * negatively by DashboardTest::testDashboardContainsNoFinancialFigures().
 */
final class StripeDashboardLinkTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-07-6/AC-07-35: opens the Stripe Express Dashboard. "In a new tab"
     * is the dashboard button's own `target="_blank"` (DashboardTest);
     * this proves the route itself actually redirects to Stripe.
     */
    public function testPlatformStripeDashboardLinkRedirectsToStripe(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $this->client->request('GET', '/super-admin/stripe');

        self::assertResponseRedirects();
        $location = $this->client->getResponse()->headers->get('Location');
        self::assertIsString($location);
        self::assertStringStartsWith('https://dashboard.stripe.com', $location, 'AC-07-35/36: lands on Stripe, not this platform.');
    }

    /**
     * AC-07-37: a per-trainer "View [Trainer]'s Stripe Account" link opens
     * that trainer's own Connect account.
     *
     * Uses a freshly created trainer, never `peak-performance`/
     * `baseline-athletics`: `StripeConnectTest`'s own class docblock
     * documents that it depends on `peak-performance` staying
     * disconnected until ITS OWN later test method connects it (PHPUnit's
     * declaration-order guarantee, no per-test rollback in this suite) —
     * connecting it here first would break that test regardless of this
     * file's own execution order relative to it.
     */
    public function testTrainerStripeDashboardLinkRedirectsToThatTrainersConnectAccount(): void
    {
        $trainer = $this->createFreshTrainer('stripe-link-connect@example.test', 'Stripe Link Connect FC');
        $this->activateTenant($trainer);

        /** @var StripeGateway $stripeGateway */
        $stripeGateway = self::getContainer()->get(StripeGateway::class);
        $settings = $stripeGateway->ensureConnectAccount($trainer);
        $connectAccountId = $settings->getStripeConnectAccountId();
        self::assertNotNull($connectAccountId);

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $this->client->request('GET', sprintf('/super-admin/trainers/%d/stripe', $trainer->getId()));

        self::assertResponseRedirects();
        $location = $this->client->getResponse()->headers->get('Location');
        self::assertSame('https://dashboard.stripe.com/'.$connectAccountId, $location, 'AC-07-37: addressed to this specific trainer\'s own Connect account.');
    }

    /**
     * A trainer that has never connected Stripe cannot be linked to a
     * Connect account that does not exist — a clear flash, not a broken
     * link to a nonexistent account.
     */
    public function testTrainerStripeDashboardLinkWithNoConnectAccountShowsAFlashInstead(): void
    {
        $trainer = $this->trainer('baseline-athletics');
        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $this->client->request('GET', sprintf('/super-admin/trainers/%d/stripe', $trainer->getId()));

        self::assertResponseRedirects(sprintf('/super-admin/trainers/%d', $trainer->getId()));
        $this->client->followRedirect();
        self::assertSelectorTextContains('.flash--error', 'has not connected Stripe yet');
    }

    /**
     * A Trainer (not Super Admin) cannot reach either link.
     */
    public function testATrainerCannotReachEitherStripeDashboardLink(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->client->loginUser($trainer->getOwnerAccount());

        $this->client->request('GET', '/super-admin/stripe');
        self::assertResponseStatusCodeSame(403);

        $this->client->request('GET', sprintf('/super-admin/trainers/%d/stripe', $trainer->getId()));
        self::assertResponseStatusCodeSame(403);
    }

    private function createFreshTrainer(string $email, string $businessName): Trainer
    {
        /** @var TrainerProvisioningService $provisioning */
        $provisioning = self::getContainer()->get(TrainerProvisioningService::class);

        return $provisioning->createTrainer($this->account('admin@practiceperfect.test'), $businessName, 'Stripe', 'Owner', $email, null);
    }
}
