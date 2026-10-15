<?php

declare(strict_types=1);

namespace App\Tests\Platform;

use App\Platform\Repository\TrainerBrandingSettingsRepository;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-01.14 — Trainer Customizes Portal Branding.
 */
final class TrainerBrandingTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-01-60: uploads a logo (PNG/JPG/SVG), which then displays in the
     * trainer's portal header (base.html.twig's branding block reads the
     * same TrainerBrandingSettings row every request).
     */
    public function testTrainerUploadsALogo(): void
    {
        $trainer = $this->account('trainer@practiceperfect.test');
        $this->client->loginUser($trainer);

        $crawler = $this->client->request('GET', '/trainer/branding');
        self::assertResponseIsSuccessful();

        $pngBytes = base64_decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=', true);
        self::assertNotFalse($pngBytes);
        $tmpPath = tempnam(sys_get_temp_dir(), 'logo').'.png';
        file_put_contents($tmpPath, $pngBytes);

        $form = $crawler->selectButton('Save branding')->form();
        /** @var \Symfony\Component\DomCrawler\Field\FileFormField $logoField */
        $logoField = $form['portal_branding[logo]'];
        $logoField->upload($tmpPath);
        $this->client->submit($form);

        self::assertResponseRedirects('/trainer/branding');

        /** @var TrainerBrandingSettingsRepository $settings */
        $settings = self::getContainer()->get(TrainerBrandingSettingsRepository::class);
        $branding = $settings->findForTrainer($this->trainer('peak-performance'));
        self::assertNotNull($branding);
        self::assertNotNull($branding->getLogoPath(), 'AC-01-60: the logo is stored.');

        @unlink($tmpPath);
    }

    /**
     * AC-01-61: a primary brand colour, real-time preview via the colour
     * input, with a reset-to-default option.
     */
    public function testTrainerSetsAPrimaryBrandColor(): void
    {
        $trainer = $this->account('trainer@practiceperfect.test');
        $this->client->loginUser($trainer);

        $crawler = $this->client->request('GET', '/trainer/branding');
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Save branding')->form([
            'portal_branding[primaryColorHex]' => '#3366ff',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/trainer/branding');

        /** @var TrainerBrandingSettingsRepository $settings */
        $settings = self::getContainer()->get(TrainerBrandingSettingsRepository::class);
        $branding = $settings->findForTrainer($this->trainer('peak-performance'));

        self::assertNotNull($branding);
        self::assertSame('#3366ff', $branding->getPrimaryColorHex(), 'AC-01-61: primary colour saved as a hex code.');
    }

    /**
     * AC-01-61: reset-to-default clears the trainer's custom colour.
     */
    public function testTrainerResetsColorToDefault(): void
    {
        $trainer = $this->account('trainer@practiceperfect.test');
        $this->client->loginUser($trainer);

        $crawler = $this->client->request('GET', '/trainer/branding');
        $form = $crawler->selectButton('Save branding')->form([
            'portal_branding[primaryColorHex]' => '#123456',
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects();

        $crawler2 = $this->client->request('GET', '/trainer/branding');
        $form2 = $crawler2->selectButton('Save branding')->form([
            'portal_branding[primaryColorHex]' => '',
        ]);
        $this->client->submit($form2);
        self::assertResponseRedirects();

        /** @var TrainerBrandingSettingsRepository $settings */
        $settings = self::getContainer()->get(TrainerBrandingSettingsRepository::class);
        $branding = $settings->findForTrainer($this->trainer('peak-performance'));
        self::assertNotNull($branding);
        self::assertNull($branding->getPrimaryColorHex(), 'AC-01-61: reset-to-default clears the custom colour.');
    }

    /**
     * AC-01-62: saving applies immediately for the whole organization — the
     * branding block on every page (base.html.twig) is read per-request from
     * the same settings row, nothing is cached.
     */
    public function testBrandingAppliesImmediatelyAcrossThePortal(): void
    {
        $trainer = $this->account('trainer@practiceperfect.test');
        $this->client->loginUser($trainer);

        $crawler = $this->client->request('GET', '/trainer/branding');
        $form = $crawler->selectButton('Save branding')->form([
            'portal_branding[primaryColorHex]' => '#abcdef',
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects();
        $this->client->followRedirect();

        self::assertSelectorTextContains('.flash--success', 'Branding saved');

        /** @var TrainerBrandingSettingsRepository $settings */
        $settings = self::getContainer()->get(TrainerBrandingSettingsRepository::class);
        $branding = $settings->findForTrainer($this->trainer('peak-performance'));
        self::assertNotNull($branding);
        self::assertSame('#abcdef', $branding->getPrimaryColorHex());
    }

    /**
     * AC-01-63: MVP scope is one logo and one primary colour only — no font
     * customization or layout options are exposed on the form.
     */
    public function testBrandingFormExposesOnlyLogoAndPrimaryColor(): void
    {
        $trainer = $this->account('trainer@practiceperfect.test');
        $this->client->loginUser($trainer);

        $this->client->request('GET', '/trainer/branding');

        self::assertSelectorExists('input[name="portal_branding[logo]"]');
        self::assertSelectorExists('input[name="portal_branding[primaryColorHex]"]');
        self::assertSelectorNotExists('input[name="portal_branding[font]"]');
        self::assertSelectorNotExists('select[name="portal_branding[layout]"]');
    }

    /**
     * Only the trainer who owns the branding settings can edit them —
     * TrainerBrandingSettings is trainer-scoped by tenant, not by a bespoke
     * ownership voter (BrandingController resolves it from TenantContext).
     */
    public function testOnlyATrainerCanReachTheBrandingPage(): void
    {
        $this->client->loginUser($this->account('coach@practiceperfect.test'));

        $this->client->request('GET', '/trainer/branding');

        self::assertResponseStatusCodeSame(403);
    }
}
