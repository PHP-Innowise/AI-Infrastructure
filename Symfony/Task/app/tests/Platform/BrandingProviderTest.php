<?php

declare(strict_types=1);

namespace App\Tests\Platform;

use App\Platform\Branding\BrandingProvider;
use App\Platform\Service\TrainerBrandingService;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\Test\KernelTestCase;

/**
 * The shade ramp and the contrast-safe foreground, derived server-side per
 * `architect-architecture.md` § "White-label branding as a runtime value"
 * and DESIGN_TOKENS.md § "CSS Custom Properties".
 *
 * @see specs/frontend-design-spec.md "Brand color derivation: PHP versus CSS color-mix()"
 */
final class BrandingProviderTest extends KernelTestCase
{
    use FixtureHelpers;

    /**
     * The arithmetic has a published answer to check itself against: this
     * codebase shipped #00B300's ramp hardcoded in base.html.twig for
     * months, and DESIGN_TOKENS.md defines soft/deep as lighten/darken by
     * 20%. A derivation that does not reproduce those exact values is
     * reading the transformation differently than the design tokens do.
     */
    public function testThePlatformDefaultReproducesTheRampThisCodebaseShipped(): void
    {
        self::bootKernel();

        /** @var BrandingProvider $provider */
        $provider = self::getContainer()->get(BrandingProvider::class);
        $brand = $provider->current();

        self::assertSame('#00B300', $brand->primaryHex);
        self::assertSame('#33C233', $brand->primarySoftHex, 'lighten(accent, 20%)');
        self::assertSame('#008F00', $brand->primaryDeepHex, 'darken(accent, 20%)');
        self::assertSame('0, 179, 0', $brand->primaryRgb, 'hexToRgb, "r, g, b" for rgba() usage');
        self::assertSame('#0D0D0D', $brand->onPrimaryHex);
        self::assertNull($brand->logoPath, 'No tenant, no uploaded logo — the template falls back to the platform mark.');
        self::assertSame('PracticePerfect', $brand->businessName);
    }

    /**
     * A Super Admin has no tenant by design, and the login screen has no
     * user at all. Neither may render a page whose --brand-* properties are
     * undefined — app.css references them unconditionally.
     */
    public function testNoResolvedTenantStillYieldsACompleteBrand(): void
    {
        self::bootKernel();

        /** @var BrandingProvider $provider */
        $provider = self::getContainer()->get(BrandingProvider::class);
        $brand = $provider->current();

        foreach (['primaryHex', 'primarySoftHex', 'primaryDeepHex', 'primaryRgb', 'onPrimaryHex'] as $property) {
            self::assertNotSame('', $brand->{$property}, sprintf('%s must always resolve.', $property));
        }
    }

    public function testAnActiveTenantGetsItsOwnColourAndItsOwnRamp(): void
    {
        self::bootKernel();
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        /** @var TrainerBrandingService $branding */
        $branding = self::getContainer()->get(TrainerBrandingService::class);
        $branding->updatePrimaryColor($trainer, '#C21E56');

        try {
            /** @var BrandingProvider $provider */
            $provider = self::getContainer()->get(BrandingProvider::class);
            $brand = $provider->current();

            self::assertSame('#C21E56', $brand->primaryHex);
            // 194 + (255-194)*0.2 = 206.2 -> 206 (CE); 30 + 45 = 75 (4B); 86 + 33.8 = 119.8 -> 120 (78)
            self::assertSame('#CE4B78', $brand->primarySoftHex);
            // 194*0.8 = 155.2 -> 155 (9B); 30*0.8 = 24 (18); 86*0.8 = 68.8 -> 69 (45)
            self::assertSame('#9B1845', $brand->primaryDeepHex);
            self::assertSame('194, 30, 86', $brand->primaryRgb);
            self::assertSame('Peak Performance Basketball', $brand->businessName);
        } finally {
            $branding->updatePrimaryColor($trainer, null);
        }
    }

    /**
     * The reason --brand-on-primary is computed rather than stored: AC-01-61
     * permits any hex, and no single foreground is readable on all of them.
     *
     * @param non-empty-string $primaryHex
     * @param non-empty-string $expectedForeground
     */
    #[\PHPUnit\Framework\Attributes\DataProvider('foregroundProvider')]
    public function testTheForegroundFollowsTheBrandsLuminance(string $primaryHex, string $expectedForeground, string $why): void
    {
        self::bootKernel();
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        /** @var TrainerBrandingService $branding */
        $branding = self::getContainer()->get(TrainerBrandingService::class);
        $branding->updatePrimaryColor($trainer, $primaryHex);

        try {
            /** @var BrandingProvider $provider */
            $provider = self::getContainer()->get(BrandingProvider::class);
            self::assertSame($expectedForeground, $provider->current()->onPrimaryHex, $why);
        } finally {
            $branding->updatePrimaryColor($trainer, null);
        }
    }

    /**
     * @return iterable<string, array{string, string, string}>
     */
    public static function foregroundProvider(): iterable
    {
        yield 'pale yellow' => ['#FFF176', '#0D0D0D', 'White on pale yellow is the unreadable case this property exists to prevent.'];
        yield 'navy' => ['#0B1F3A', '#FFFFFF', 'A dark brand needs a light foreground.'];
        yield 'mid grey' => ['#767676', '#FFFFFF', 'The awkward middle still has to resolve to whichever side contrasts more.'];
        yield 'white' => ['#FFFFFF', '#0D0D0D', 'A white brand must not paint white text on itself.'];
    }
}
