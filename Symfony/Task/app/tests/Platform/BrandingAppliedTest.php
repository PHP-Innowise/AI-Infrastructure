<?php

declare(strict_types=1);

namespace App\Tests\Platform;

use App\Platform\Service\TrainerBrandingService;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * AC-01-62: "Save changes → Branding applied", "Changes visible immediately
 * to all users in trainer's organization". AC-01-60: "Logo visible to:
 * Trainer's players, coaches, parents."
 *
 * These are the assertions AC-01-60/61/62 needed and did not have. The
 * existing branding tests assert that the colour and the logo path are
 * *stored*, which was true and passing while the value reached no page at
 * all: `base.html.twig` emitted the hardcoded platform default on every
 * request, no template read `primaryColorHex`, and the logo appeared only on
 * the trainer's own settings screen. A trainer could set a brand, be told
 * "Branding saved.", and see nothing change anywhere.
 *
 * Everything below therefore asserts against rendered pages, and against
 * pages belonging to the people the criterion names.
 */
final class BrandingAppliedTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    public function testAPlayerSeesTheirTrainersColourOnTheirOwnPages(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->setBrandColor('#C21E56');

        try {
            $this->client->loginUser($this->account('player@practiceperfect.test'));
            $this->switchPlayerToTrainer($this->client, $trainer);
            $this->client->request('GET', '/portal/calendar');

            self::assertResponseIsSuccessful();
            $css = $this->inlineBrandingCss();

            self::assertStringContainsString('--brand-primary: #C21E56;', $css);
            self::assertStringContainsString('--brand-primary-soft: #CE4B78;', $css, 'The derived ramp travels with it.');
            self::assertStringContainsString('--brand-primary-deep: #9B1845;', $css);
            self::assertStringContainsString('--brand-primary-rgb: 194, 30, 86;', $css);
            self::assertStringNotContainsString('#00B300', $css, 'The platform default must not survive alongside the trainer colour.');
        } finally {
            $this->resetBrandColor();
        }
    }

    /**
     * The other half of "for everyone in that trainer's organization": not
     * for anyone else. A brand that leaked across tenants would be the same
     * class of defect as a data leak, just visible instead of readable.
     */
    public function testAnotherTrainersOrganizationIsUnaffected(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->setBrandColor('#C21E56');

        try {
            $this->client->loginUser($this->account('trainer-b@practiceperfect.test'));
            $this->client->request('GET', '/dashboard');

            self::assertResponseIsSuccessful();
            $css = $this->inlineBrandingCss();

            self::assertStringNotContainsString('#C21E56', $css, "Baseline Athletics must not inherit Peak Performance's brand.");
            self::assertStringContainsString('--brand-primary: #00B300;', $css, 'A trainer who set no colour keeps the platform default.');
        } finally {
            $this->resetBrandColor();
        }
    }

    /**
     * BR-08-6 / the branding entity's own reason for carrying no RLS: a
     * prospective customer meets the trainer's brand before they have an
     * account, on a page resolved from a code rather than a session.
     */
    public function testTheColourReachesAnUnauthenticatedLandingPage(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->setBrandColor('#C21E56');

        try {
            $this->client->request('GET', '/join/join-peak-performance');

            self::assertResponseIsSuccessful();
            self::assertStringContainsString('--brand-primary: #C21E56;', $this->inlineBrandingCss());
        } finally {
            $this->resetBrandColor();
        }
    }

    /**
     * AC-01-60. The logo is rendered through <img>, never inlined, so an
     * uploaded SVG cannot execute in this origin.
     */
    public function testAPlayerSeesTheirTrainersLogo(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        /** @var TrainerBrandingService $branding */
        $branding = self::getContainer()->get(TrainerBrandingService::class);
        $branding->updateLogo($trainer, '/uploads/branding/fixture-logo.svg');

        try {
            $this->client->loginUser($this->account('player@practiceperfect.test'));
            $this->switchPlayerToTrainer($this->client, $trainer);
            $crawler = $this->client->request('GET', '/portal/calendar');

            self::assertResponseIsSuccessful();
            $logo = $crawler->filter('header img.brand-bar__logo');
            self::assertCount(1, $logo, 'AC-01-60: the logo is on the pages a player actually visits.');
            self::assertSame('/uploads/branding/fixture-logo.svg', $logo->attr('src'));
            self::assertSame('Peak Performance Basketball', $logo->attr('alt'));
        } finally {
            $branding->updateLogo($trainer, null);
        }
    }

    /**
     * With no logo uploaded the platform's own mark stands in, so the bar is
     * never an empty frame — the same "always resolves something" rule the
     * colour follows.
     */
    public function testThePlatformLogoStandsInUntilATrainerUploadsOne(): void
    {
        $this->client->request('GET', '/login');

        self::assertResponseIsSuccessful();
        self::assertSelectorExists('header img.brand-bar__logo');
    }

    private function setBrandColor(string $hex): void
    {
        /** @var TrainerBrandingService $branding */
        $branding = self::getContainer()->get(TrainerBrandingService::class);
        $branding->updatePrimaryColor($this->trainer('peak-performance'), $hex);
    }

    /**
     * Cleared rather than set back to a literal: with no row value the
     * provider falls through to the platform default, which is the state
     * every other test in the suite assumes.
     */
    private function resetBrandColor(): void
    {
        $this->activateTenant($this->trainer('peak-performance'));

        /** @var TrainerBrandingService $branding */
        $branding = self::getContainer()->get(TrainerBrandingService::class);
        $branding->updatePrimaryColor($this->trainer('peak-performance'), null);
    }

    private function inlineBrandingCss(): string
    {
        $style = $this->client->getCrawler()->filter('head style');
        self::assertGreaterThan(0, $style->count(), 'The runtime branding layer must always be emitted.');

        return $style->text();
    }
}
