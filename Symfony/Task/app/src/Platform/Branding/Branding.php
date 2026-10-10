<?php

declare(strict_types=1);

namespace App\Platform\Branding;

/**
 * One request's resolved brand: the five runtime-overridable custom
 * properties, plus what the page needs to show a logo.
 *
 * Every hex here is already derived and validated — a template only
 * interpolates. That matters because these values land inside an inline
 * `<style>` block, where an unconstrained string would be a CSS injection
 * into every page of a tenant; `TrainerBrandingSettings::updatePrimaryColor()`
 * rejects anything but `#RRGGBB` at the entity boundary, and
 * `BrandingProvider` derives the rest arithmetically from it, so no
 * user-supplied text ever reaches the stylesheet.
 *
 * @see specs/frontend-design-spec.md "Design tokens: CSS custom properties" — runtime layer
 */
final readonly class Branding
{
    public function __construct(
        public string $primaryHex,
        public string $primarySoftHex,
        public string $primaryDeepHex,
        public string $primaryRgb,
        public string $onPrimaryHex,
        /** Web path of an uploaded logo, or null to fall back to the platform's own. */
        public ?string $logoPath,
        /** Whose brand this is — the alt text for the logo, never rendered as page copy. */
        public string $businessName,
    ) {
    }
}
