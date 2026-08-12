<?php

declare(strict_types=1);

namespace App\Platform\Navigation;

use App\Identity\Entity\Account;
use App\Platform\Tenancy\TenantContext;
use Doctrine\DBAL\Connection;
use Symfony\Bundle\SecurityBundle\Security;

/**
 * Resolves the navigation for the request in flight, so the shared layout can
 * draw it on every page instead of only on the dashboard.
 *
 * It used to be the dashboard controller's job, which is why the product had
 * navigation on exactly one screen: open Players, Tokens or Branding and the
 * only way back was the browser's own back button. Manual testing put it
 * plainly — 114 templates, not one link to the dashboard, and an empty
 * `{% block header %}` in the layout that nothing overrode.
 *
 * **Read through DBAL, not the ORM**, for the same reason `BrandingProvider`
 * is: chrome renders on every response, including ones whose domain
 * transaction has already rolled back and closed the EntityManager. Page
 * furniture must not be able to turn a rendered 422 into a 500.
 *
 * `feature_toggle` carries no RLS — toggles are platform configuration
 * *about* a trainer rather than tenant data — so this reads by trainer id
 * without needing the tenant session variable to be set first.
 *
 * @see specs/requirements-analyst-epic-07-super-admin-spec.md BR-07-1
 */
final readonly class NavigationProvider
{
    public function __construct(
        private Security $security,
        private TenantContext $tenantContext,
        private Connection $connection,
    ) {
    }

    public function current(): Navigation
    {
        $account = $this->security->getUser();

        if (!$account instanceof Account) {
            return new Navigation(null, []);
        }

        return new Navigation($account->getRole(), $this->enabledFeatures());
    }

    /**
     * @return array<string, bool>
     */
    private function enabledFeatures(): array
    {
        $trainerId = $this->tenantContext->getTrainerIdOrNull();

        if (null === $trainerId) {
            // A Super Admin holds no tenant, and their own surfaces are never
            // feature-gated: toggles describe a trainer, not an administrator.
            return [];
        }

        /** @var list<array{feature_name: mixed, is_enabled: mixed}> $rows */
        $rows = $this->connection->fetchAllAssociative(
            'SELECT feature_name, is_enabled FROM feature_toggle WHERE trainer_id = ?',
            [$trainerId],
        );

        $enabled = [];

        foreach ($rows as $row) {
            $enabled[(string) $row['feature_name']] = (bool) $row['is_enabled'];
        }

        return $enabled;
    }
}
