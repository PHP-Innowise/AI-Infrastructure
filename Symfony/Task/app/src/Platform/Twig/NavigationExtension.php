<?php

declare(strict_types=1);

namespace App\Platform\Twig;

use App\Platform\Navigation\Navigation;
use App\Platform\Navigation\NavigationProvider;
use Twig\Extension\AbstractExtension;
use Twig\TwigFunction;

/**
 * `navigation()` in Twig — how `base.html.twig` reaches the current user's
 * navigation without every controller having to pass it.
 *
 * A function rather than a Twig global, for the same reason `branding()` is
 * one: a global is resolved once when the environment is built, and this
 * depends on who is signed in.
 */
final class NavigationExtension extends AbstractExtension
{
    public function __construct(
        private readonly NavigationProvider $navigationProvider,
    ) {
    }

    public function getFunctions(): array
    {
        return [
            new TwigFunction('navigation', $this->navigation(...)),
        ];
    }

    public function navigation(): Navigation
    {
        return $this->navigationProvider->current();
    }
}
