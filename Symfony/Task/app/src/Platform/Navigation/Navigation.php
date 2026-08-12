<?php

declare(strict_types=1);

namespace App\Platform\Navigation;

use App\Identity\Entity\AccountRole;

/**
 * What the shared chrome needs to draw one request's navigation: who is
 * looking, and which of their trainer's features are switched on.
 *
 * Anonymous requests — the login screen, a public landing page — resolve to
 * a null role and no links at all.
 */
final readonly class Navigation
{
    /**
     * @param array<string, bool> $features feature name => enabled; empty when
     *                                      no tenant is resolved
     */
    public function __construct(
        public ?AccountRole $role,
        private array $features,
    ) {
    }

    public function isFor(string $role): bool
    {
        return $this->role?->value === $role;
    }

    /**
     * BR-07-1: a disabled feature "disappears from the trainer's UI", so a
     * gated group is omitted rather than left to 403 when clicked.
     *
     * Unknown defaults to shown, matching `FeatureGate`'s own default and
     * the reason for it: "disabled for this trainer" and "we do not yet know
     * which trainer" are different states, and only the first should hide
     * anything. A player who has not picked a context yet must still be able
     * to find their content.
     */
    public function isEnabled(string $feature): bool
    {
        return $this->features[$feature] ?? true;
    }
}
