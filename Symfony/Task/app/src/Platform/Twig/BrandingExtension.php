<?php

declare(strict_types=1);

namespace App\Platform\Twig;

use App\Platform\Branding\Branding;
use App\Platform\Branding\BrandingProvider;
use Twig\Extension\AbstractExtension;
use Twig\TwigFunction;

/**
 * `branding()` in Twig — how `base.html.twig` reaches the resolved brand for
 * the request in flight.
 *
 * A function rather than a Twig global: a global is resolved once when the
 * environment is built, which would hand every request the brand of whoever
 * happened to be first. The brand is per-request by definition (AC-01-62),
 * so it has to be asked for per render.
 */
final class BrandingExtension extends AbstractExtension
{
    public function __construct(
        private readonly BrandingProvider $brandingProvider,
    ) {
    }

    public function getFunctions(): array
    {
        return [
            new TwigFunction('branding', $this->branding(...)),
        ];
    }

    public function branding(): Branding
    {
        return $this->brandingProvider->current();
    }
}
