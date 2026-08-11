<?php

declare(strict_types=1);

namespace App\Tests\Administration;

use App\Identity\Service\TrainerProvisioningService;
use App\Platform\Entity\FeatureToggle;
use App\Platform\Entity\Trainer;
use App\Platform\Repository\AuditLogEntryRepository;
use App\Platform\Repository\FeatureToggleRepository;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-07.05 — Super Admin Configures Feature Toggle per Trainer.
 *
 * Every test that actually DISABLES a toggle creates its own fresh trainer
 * (never `peak-performance`/`baseline-athletics`) — those two fixture
 * trainers are shared, mutable state read by Growth/Content tests
 * throughout the whole `make test` run, built long before feature toggles
 * existed and therefore assuming LPPP/Marketing stay enabled; permanently
 * disabling either on a shared fixture trainer would be a real regression
 * risk to Epic-04/06's own coverage, not merely a style preference.
 */
final class FeatureToggleTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-07-18: navigates to the Trainers list, selects a trainer, opens
     * "Feature Settings" to view the toggle list. AC-07-19: exactly three
     * features, each defaulting to ON. BR-07-2 restates the same default
     * for a genuinely freshly-created trainer (seeded at creation, not
     * merely by this screen's own defensive re-seed).
     */
    public function testFeatureSettingsShowsAllThreeFeaturesEnabledByDefault(): void
    {
        $trainer = $this->createFreshTrainer('toggle-view@example.test', 'Toggle View FC');

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', sprintf('/super-admin/trainers/%d/features', $trainer->getId()));

        self::assertResponseIsSuccessful();
        $rows = $crawler->filter('table tbody tr');
        self::assertCount(3, $rows, 'AC-07-19: exactly three toggleable features.');
        self::assertSelectorTextContains('body', 'LPPP Content System');
        self::assertSelectorTextContains('body', 'Marketing Tools');
        self::assertSelectorTextContains('body', 'Camps', 'The edge case resolution: Camps is always shown for MVP.');

        foreach ($rows as $row) {
            self::assertStringContainsString('Enabled', $row->textContent, 'BR-07-2: every feature defaults to enabled.');
        }
    }

    /**
     * AC-07-20: toggling requires a confirmation naming the trainer and
     * feature and warning of the effect — and nothing changes until that
     * confirmation is actually submitted.
     */
    public function testDisablingShowsAConfirmationNamingTrainerAndFeatureAndDoesNotApplyUntilConfirmed(): void
    {
        $trainer = $this->createFreshTrainer('toggle-confirm@example.test', 'Toggle Confirm FC');

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request(
            'GET',
            sprintf('/super-admin/trainers/%d/features', $trainer->getId()),
            ['feature' => FeatureToggle::FEATURE_LPPP],
        );

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains(
            '.form-error',
            'Disable LPPP Content System for Toggle Confirm FC? Existing content will be hidden from players.',
        );
        self::assertGreaterThan(0, $crawler->selectButton('Confirm disable')->count());

        /** @var FeatureToggleRepository $toggles */
        $toggles = self::getContainer()->get(FeatureToggleRepository::class);
        $toggle = $toggles->findOneByTrainerAndFeature($trainer, FeatureToggle::FEATURE_LPPP);
        self::assertNotNull($toggle);
        self::assertTrue($toggle->isEnabled(), 'AC-07-20: merely viewing the confirmation applies nothing yet.');
    }

    /**
     * AC-07-20 (apply immediately) + AC-07-21 (audit logged: who, which
     * feature, for which trainer, when) + BR-07-3 (immediate effect).
     */
    public function testConfirmingDisableAppliesImmediatelyAndIsAudited(): void
    {
        $trainer = $this->createFreshTrainer('toggle-disable@example.test', 'Toggle Disable FC');
        $admin = $this->account('admin@practiceperfect.test');
        $this->client->loginUser($admin);

        $crawler = $this->client->request(
            'GET',
            sprintf('/super-admin/trainers/%d/features', $trainer->getId()),
            ['feature' => FeatureToggle::FEATURE_MARKETING],
        );
        $form = $crawler->selectButton('Confirm disable')->form();
        $this->client->submit($form);

        self::assertResponseRedirects(sprintf('/super-admin/trainers/%d/features', $trainer->getId()));
        $this->client->followRedirect();
        self::assertSelectorTextContains('.flash--success', 'Marketing Tools disabled for Toggle Disable FC.');

        /** @var FeatureToggleRepository $toggles */
        $toggles = self::getContainer()->get(FeatureToggleRepository::class);
        $toggle = $toggles->findOneByTrainerAndFeature($trainer, FeatureToggle::FEATURE_MARKETING);
        self::assertNotNull($toggle);
        self::assertFalse($toggle->isEnabled(), 'AC-07-20/BR-07-3: applies immediately.');

        /** @var AuditLogEntryRepository $auditLog */
        $auditLog = self::getContainer()->get(AuditLogEntryRepository::class);
        $entries = $auditLog->search('feature_toggled', (int) $trainer->getId());
        self::assertNotEmpty($entries, 'AC-07-21: the toggle change is logged.');
        self::assertSame($admin->getId(), $entries[0]->getActorAccount()?->getId(), 'AC-07-21: who.');
        self::assertSame('marketing', $entries[0]->getDetails()['feature'] ?? null, 'AC-07-21: which feature.');
        self::assertSame($trainer->getId(), $entries[0]->getRelatedTrainer()?->getId(), 'AC-07-21: for which trainer.');
        self::assertFalse($entries[0]->getDetails()['newEnabled'] ?? true);
    }

    /**
     * Edge case: a previously-disabled feature, re-enabled, restores
     * access — "all previously hidden data reappears" (BR-07-3). Proven
     * here as the toggle's own state round-tripping true -> false -> true;
     * the "nothing is deleted" half is proven structurally by disable()/
     * enable() only ever mutating this one row (see FeatureToggle's own
     * docblock) — there is no code path anywhere that touches LPPP content
     * as a side effect of a toggle flip.
     */
    public function testReEnablingAPreviouslyDisabledFeatureRestoresIt(): void
    {
        $trainer = $this->createFreshTrainer('toggle-reenable@example.test', 'Toggle Reenable FC');
        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $disableCrawler = $this->client->request(
            'GET',
            sprintf('/super-admin/trainers/%d/features', $trainer->getId()),
            ['feature' => FeatureToggle::FEATURE_CAMPS],
        );
        $this->client->submit($disableCrawler->selectButton('Confirm disable')->form());
        self::assertResponseRedirects();

        $reenableCrawler = $this->client->request(
            'GET',
            sprintf('/super-admin/trainers/%d/features', $trainer->getId()),
            ['feature' => FeatureToggle::FEATURE_CAMPS],
        );
        self::assertSelectorTextContains('.form-error', 'Enable Camps for Toggle Reenable FC?');
        $this->client->submit($reenableCrawler->selectButton('Confirm enable')->form());

        self::assertResponseRedirects();
        $this->client->followRedirect();
        self::assertSelectorTextContains('.flash--success', 'Camps enabled for Toggle Reenable FC.');

        /** @var FeatureToggleRepository $toggles */
        $toggles = self::getContainer()->get(FeatureToggleRepository::class);
        self::assertTrue($toggles->findOneByTrainerAndFeature($trainer, FeatureToggle::FEATURE_CAMPS)?->isEnabled());
    }

    /**
     * BR-07-1: "LPPP disabled removes trainer access to the LPPP section."
     */
    public function testDisablingLpppDeniesTheTrainerAccessToTheLpppSection(): void
    {
        $trainer = $this->createFreshTrainer('toggle-lppp@example.test', 'Toggle LPPP FC');
        $this->disableFeature($trainer, FeatureToggle::FEATURE_LPPP);

        $this->client->loginUser($trainer->getOwnerAccount());
        $this->client->request('GET', '/trainer/content/learn/new');

        self::assertResponseStatusCodeSame(403, 'BR-07-1: the LPPP section is denied once disabled.');
    }

    /**
     * BR-07-1: "Marketing disabled removes the trainer's ability to create
     * coupons or view the referral dashboard" — both routes named
     * explicitly in the business rule are checked.
     */
    public function testDisablingMarketingDeniesCouponsAndTheReferralDashboard(): void
    {
        $trainer = $this->createFreshTrainer('toggle-marketing@example.test', 'Toggle Marketing FC');
        $this->disableFeature($trainer, FeatureToggle::FEATURE_MARKETING);

        $this->client->loginUser($trainer->getOwnerAccount());

        $this->client->request('GET', '/trainer/marketing/coupons');
        self::assertResponseStatusCodeSame(403, 'BR-07-1: coupons denied once Marketing is disabled.');

        $this->client->request('GET', '/trainer/marketing/referrals');
        self::assertResponseStatusCodeSame(403, 'BR-07-1: the referral dashboard denied once Marketing is disabled.');
    }

    /**
     * The converse of the two tests above: with every toggle at its BR-07-2
     * default (enabled), both previously-ungated Growth routes still work —
     * proving the gate is genuinely conditional, not a blanket new denial.
     */
    public function testMarketingRoutesRemainReachableWhileTheToggleStaysEnabled(): void
    {
        $trainer = $this->createFreshTrainer('toggle-marketing-on@example.test', 'Toggle Marketing On FC');

        $this->client->loginUser($trainer->getOwnerAccount());

        $this->client->request('GET', '/trainer/marketing/coupons');
        self::assertResponseIsSuccessful();

        $this->client->request('GET', '/trainer/marketing/referrals');
        self::assertResponseIsSuccessful();
    }

    /**
     * A Trainer (not Super Admin) cannot reach the feature-toggle screen —
     * `PlatformConfigurationVoter::PLATFORM_CONFIG_EDIT` is
     * `ROLE_SUPER_ADMIN`-only outright, no self-service branch.
     */
    public function testATrainerCannotEditItsOwnFeatureToggles(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->client->loginUser($trainer->getOwnerAccount());

        $this->client->request('GET', sprintf('/super-admin/trainers/%d/features', $trainer->getId()));

        self::assertResponseStatusCodeSame(403);
    }

    private function createFreshTrainer(string $email, string $businessName): Trainer
    {
        /** @var TrainerProvisioningService $provisioning */
        $provisioning = self::getContainer()->get(TrainerProvisioningService::class);

        return $provisioning->createTrainer($this->account('admin@practiceperfect.test'), $businessName, 'Toggle', 'Owner', $email, null);
    }

    private function disableFeature(Trainer $trainer, string $feature): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request(
            'GET',
            sprintf('/super-admin/trainers/%d/features', $trainer->getId()),
            ['feature' => $feature],
        );
        $this->client->submit($crawler->selectButton('Confirm disable')->form());
        self::assertResponseRedirects();
    }
}
