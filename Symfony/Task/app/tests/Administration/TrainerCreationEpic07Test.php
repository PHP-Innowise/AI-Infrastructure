<?php

declare(strict_types=1);

namespace App\Tests\Administration;

use App\Billing\Repository\PlatformSubscriptionRepository;
use App\Billing\Repository\TrainerBillingSettingsRepository;
use App\Identity\Form\CreateTrainerType;
use App\Platform\Repository\TrainerRepository;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-07.04 — Super Admin Creates Trainer Account. AC-01-1..8's own
 * TrainerProvisioningTest.php covers the Identity-service half directly;
 * this file covers Epic-07's own addition end to end, through the real
 * HTTP form: the subscription-tier field and the resulting
 * PlatformSubscription + Stripe subscription.
 */
final class TrainerCreationEpic07Test extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-07-16: the creation form captures a subscription tier, and saving
     * creates a Stripe subscription (Epic-05 integration) alongside the
     * trainer account.
     */
    public function testCreatingATrainerWithAChosenTierProvisionsThePlatformSubscription(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/super-admin/trainers/new');
        self::assertResponseIsSuccessful();
        self::assertCount(1, $crawler->filter('select[name="create_trainer[subscriptionTier]"]'), 'AC-07-16: a subscription-tier field exists.');

        $form = $crawler->selectButton('Create trainer account')->form([
            'create_trainer[businessName]' => 'Epic Seven Sports Academy',
            'create_trainer[trainerFirstName]' => 'Tara',
            'create_trainer[trainerLastName]' => 'Seven',
            'create_trainer[email]' => 'tara.seven@example.test',
            'create_trainer[subscriptionTier]' => (string) CreateTrainerType::TIER_GROWTH_MINOR_UNITS,
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();
        $this->client->followRedirect();
        self::assertSelectorTextContains('.flash--success', 'Epic Seven Sports Academy');

        /** @var TrainerRepository $trainers */
        $trainers = self::getContainer()->get(TrainerRepository::class);
        $trainer = $trainers->findOneBySlug('epic-seven-sports-academy');
        self::assertNotNull($trainer, 'The trainer was created.');

        $this->activateTenant($trainer);

        /** @var TrainerBillingSettingsRepository $billingSettings */
        $billingSettings = self::getContainer()->get(TrainerBillingSettingsRepository::class);
        $settings = $billingSettings->getOrCreateForTrainer($trainer);
        self::assertSame(
            CreateTrainerType::TIER_GROWTH_MINOR_UNITS,
            $settings->getMonthlySubscriptionPriceMinorUnits(),
            'AC-07-16: the chosen tier becomes the starting subscription price.',
        );

        /** @var PlatformSubscriptionRepository $platformSubscriptions */
        $platformSubscriptions = self::getContainer()->get(PlatformSubscriptionRepository::class);
        $subscription = $platformSubscriptions->findForTrainer($trainer);
        self::assertNotNull($subscription);
        self::assertNotNull($subscription->getStripeSubscriptionId(), 'AC-07-16: a Stripe subscription id is attached.');
        self::assertTrue($subscription->isActive(), 'AC-07-16: the platform subscription moves Active.');
    }

    /**
     * AC-07-16 default: leaving the tier untouched still provisions the
     * Standard ($15/month, BR-05-13's own platform default) tier.
     */
    public function testDefaultTierMatchesThePlatformStandardRate(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/super-admin/trainers/new');
        $form = $crawler->selectButton('Create trainer account')->form([
            'create_trainer[businessName]' => 'Default Tier Sports',
            'create_trainer[trainerFirstName]' => 'Dana',
            'create_trainer[trainerLastName]' => 'Default',
            'create_trainer[email]' => 'dana.default@example.test',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();

        /** @var TrainerRepository $trainers */
        $trainers = self::getContainer()->get(TrainerRepository::class);
        $trainer = $trainers->findOneBySlug('default-tier-sports');
        self::assertNotNull($trainer);
        $this->activateTenant($trainer);

        /** @var TrainerBillingSettingsRepository $billingSettings */
        $billingSettings = self::getContainer()->get(TrainerBillingSettingsRepository::class);
        self::assertSame(
            CreateTrainerType::TIER_STANDARD_MINOR_UNITS,
            $billingSettings->getOrCreateForTrainer($trainer)->getMonthlySubscriptionPriceMinorUnits(),
        );
    }

    /**
     * AC-07-17: the Users tool also shows a "Create Trainer" quick-action
     * button (the Dashboard's own copy is proven by DashboardTest).
     */
    public function testUsersToolShowsCreateTrainerQuickAction(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/super-admin/users');

        self::assertResponseIsSuccessful();
        self::assertGreaterThan(0, $crawler->selectLink('+ Create trainer')->count());
    }

    /**
     * A Trainer (not Super Admin) cannot reach trainer creation at all.
     */
    public function testATrainerCannotCreateAnotherTrainer(): void
    {
        $this->client->loginUser($this->trainer('peak-performance')->getOwnerAccount());
        $this->client->request('GET', '/super-admin/trainers/new');

        self::assertResponseStatusCodeSame(403);
    }
}
