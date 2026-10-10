<?php

declare(strict_types=1);

namespace App\Platform\Branding;

use App\Platform\Tenancy\TenantContext;
use Doctrine\DBAL\Connection;

/**
 * Resolves the brand for the request in flight and derives the shade ramp
 * server-side, exactly as `architect-architecture.md` § "White-label branding
 * as a runtime value" settled: `lightenColor`, `darkenColor` and `hexToRgb`
 * from `Task/designs/DESIGN_TOKENS.md`, plus the contrast-safe
 * `--brand-on-primary` the frontend spec adds on top of that file's four.
 *
 * **Whose brand.** The active tenant's, whoever is looking — a player, a
 * coach, a parent, or nobody at all. `TenantFromPublicCode` resolves the
 * tenant from the code in the URL before the controller runs on
 * `/join/{code}` and `/forms/{code}`, so an unauthenticated visitor on a
 * trainer's landing page gets that trainer's brand without this class
 * needing a second, public-only resolution path. With no tenant resolved —
 * the login screen, a Super Admin, the platform's own pages — the platform
 * default is returned. There is no request that renders without a brand.
 *
 * **Nothing is cached** (AC-01-62: "changes visible immediately to all users
 * in the trainer's organization"): the settings row is read per request, and
 * the arithmetic below is a few integer operations, not a reason to
 * introduce an invalidation problem.
 *
 * **Read through DBAL, not the ORM.** The layout renders on every response,
 * including ones where a domain transaction has already rolled back — and
 * `EntityManager::wrapInTransaction()` CLOSES the EntityManager on any
 * exception, expected ones included (see `TokenLedgerService::spend()`'s own
 * docblock for what that cost). Page chrome must not be able to turn a
 * rendered 422 into a 500, so it depends on the connection rather than on a
 * manager some earlier service may have closed. Two columns and a name are
 * also cheaper to read than a hydrated `Trainer` on every request.
 *
 * `trainer` and `trainer_branding_settings` deliberately carry no RLS — the
 * branding row is global by nature, since a public landing page must render
 * a trainer's brand with no session to scope by (see the entity's own
 * docblock and the tenancy manifests) — so this query needs no tenant
 * session variable to return its row.
 *
 * @see specs/frontend-design-spec.md "Brand color derivation: PHP versus CSS color-mix()"
 * @see specs/requirements-analyst-epic-01-user-management-spec.md AC-01-60..63
 */
final readonly class BrandingProvider
{
    /**
     * The platform's own brand — DESIGN_TOKENS.md's accent, and the value
     * `assets/styles/app.css` was written against.
     */
    public const string PLATFORM_PRIMARY_HEX = '#00B300';

    public const string PLATFORM_NAME = 'PracticePerfect';

    /**
     * DESIGN_TOKENS.md § "CSS Custom Properties": soft is `lighten(accent,
     * 20%)`, deep is `darken(accent, 20%)`.
     */
    private const float SHADE_STEP = 0.2;

    /**
     * The two candidate foregrounds for anything painted on the brand
     * colour: `--color-surface` and `--gray-900` from the static token
     * layer. Whichever contrasts better with the trainer's actual colour
     * wins — no fixed choice can be right for every hue a free colour
     * picker allows.
     */
    private const string LIGHT_FOREGROUND = '#FFFFFF';

    private const string DARK_FOREGROUND = '#0D0D0D';

    public function __construct(
        private TenantContext $tenantContext,
        private Connection $connection,
    ) {
    }

    public function current(): Branding
    {
        $row = $this->rowForActiveTenant();

        if (null === $row) {
            return $this->brandFor(self::PLATFORM_PRIMARY_HEX, null, self::PLATFORM_NAME);
        }

        $primaryHex = $row['primary_color_hex'];
        $logoPath = $row['logo_path'];

        return $this->brandFor(
            \is_string($primaryHex) && '' !== $primaryHex ? $primaryHex : self::PLATFORM_PRIMARY_HEX,
            \is_string($logoPath) && '' !== $logoPath ? $logoPath : null,
            \is_string($row['business_name']) ? $row['business_name'] : self::PLATFORM_NAME,
        );
    }

    /**
     * @return array{business_name: mixed, primary_color_hex: mixed, logo_path: mixed}|null
     */
    private function rowForActiveTenant(): ?array
    {
        $trainerId = $this->tenantContext->getTrainerIdOrNull();

        if (null === $trainerId) {
            return null;
        }

        /** @var array{business_name: mixed, primary_color_hex: mixed, logo_path: mixed}|false $row */
        $row = $this->connection->fetchAssociative(
            'SELECT t.business_name, b.primary_color_hex, b.logo_path
               FROM trainer t
               LEFT JOIN trainer_branding_settings b ON b.trainer_id = t.id
              WHERE t.id = ?',
            [$trainerId],
        );

        return false === $row ? null : $row;
    }

    private function brandFor(string $primaryHex, ?string $logoPath, string $businessName): Branding
    {
        $rgb = $this->hexToRgb($primaryHex);

        return new Branding(
            primaryHex: strtoupper($primaryHex),
            primarySoftHex: $this->lighten($rgb, self::SHADE_STEP),
            primaryDeepHex: $this->darken($rgb, self::SHADE_STEP),
            primaryRgb: implode(', ', $rgb),
            onPrimaryHex: $this->contrastingForeground($rgb),
            logoPath: $logoPath,
            businessName: $businessName,
        );
    }

    /**
     * @return array{int, int, int}
     */
    private function hexToRgb(string $hex): array
    {
        // Guaranteed #RRGGBB by TrainerBrandingSettings' own validation and
        // by this class's platform constant; hexdec cannot fail here.
        return [
            (int) hexdec(substr($hex, 1, 2)),
            (int) hexdec(substr($hex, 3, 2)),
            (int) hexdec(substr($hex, 5, 2)),
        ];
    }

    /**
     * Toward white by $percent of the remaining distance — the reading that
     * reproduces DESIGN_TOKENS.md's own published ramp: #00B300 lightened
     * 20% is #33C233, the platform default this codebase already shipped
     * hardcoded.
     *
     * @param array{int, int, int} $rgb
     */
    private function lighten(array $rgb, float $percent): string
    {
        return $this->toHex(array_map(
            static fn (int $channel): int => (int) round($channel + (255 - $channel) * $percent),
            $rgb,
        ));
    }

    /**
     * Toward black by the same measure: #00B300 darkened 20% is #008F00,
     * again matching the shipped default.
     *
     * @param array{int, int, int} $rgb
     */
    private function darken(array $rgb, float $percent): string
    {
        return $this->toHex(array_map(
            static fn (int $channel): int => (int) round($channel * (1 - $percent)),
            $rgb,
        ));
    }

    /**
     * WCAG 2.2 relative luminance, then the better of the two candidate
     * foregrounds by contrast ratio. A trainer who picks pale yellow gets
     * near-black labels on their buttons instead of unreadable white ones,
     * without the colour itself being rejected — AC-01-61 states a free hex
     * picker and no failure mode, so the robustness belongs here rather
     * than in a validation rule the epic never asked for.
     *
     * @param array{int, int, int} $rgb
     */
    private function contrastingForeground(array $rgb): string
    {
        $brand = $this->relativeLuminance($rgb);
        $onLight = $this->contrastRatio($brand, $this->relativeLuminance($this->hexToRgb(self::LIGHT_FOREGROUND)));
        $onDark = $this->contrastRatio($brand, $this->relativeLuminance($this->hexToRgb(self::DARK_FOREGROUND)));

        return $onDark >= $onLight ? self::DARK_FOREGROUND : self::LIGHT_FOREGROUND;
    }

    /**
     * @param array{int, int, int} $rgb
     */
    private function relativeLuminance(array $rgb): float
    {
        [$r, $g, $b] = array_map($this->linearize(...), $rgb);

        return 0.2126 * $r + 0.7152 * $g + 0.0722 * $b;
    }

    private function linearize(int $channel): float
    {
        $normalized = $channel / 255;

        return $normalized <= 0.04045
            ? $normalized / 12.92
            : (($normalized + 0.055) / 1.055) ** 2.4;
    }

    private function contrastRatio(float $one, float $other): float
    {
        $lighter = max($one, $other);
        $darker = min($one, $other);

        return ($lighter + 0.05) / ($darker + 0.05);
    }

    /**
     * @param list<int> $rgb
     */
    private function toHex(array $rgb): string
    {
        return sprintf('#%02X%02X%02X', ...$rgb);
    }
}
