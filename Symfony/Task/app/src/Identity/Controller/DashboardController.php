<?php

declare(strict_types=1);

namespace App\Identity\Controller;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Platform\Entity\FeatureToggle;
use App\Platform\Repository\AccountTrainerLinkRepository;
use App\Platform\Repository\TrainerRepository;
use App\Platform\Service\FeatureGate;
use App\Platform\Tenancy\TenantContext;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * The post-login landing surface, one per role.
 *
 * AC-01-5 (a trainer reaches the trainer dashboard), AC-01-32 and AC-01-15
 * (a player sees their trainers, separated — never combined, per owner
 * decision A6).
 *
 * This is also the only navigation in the product, which makes it the single
 * place a whole module can become unreachable. Every epic added routes; only
 * two added links, so five modules shipped invisible — reachable by typing a
 * URL and no other way. The template now enumerates each role's surfaces, and
 * DashboardNavigationTest asserts that enumeration against the router so the
 * next module cannot quietly go missing.
 */
final class DashboardController extends AbstractController
{
    #[Route('/dashboard', name: 'app_dashboard', methods: ['GET'])]
    #[IsGranted('IS_AUTHENTICATED_FULLY')]
    public function __invoke(
        TenantContext $tenantContext,
        AccountTrainerLinkRepository $links,
        TrainerRepository $trainers,
        FeatureGate $featureGate,
    ): Response {
        /** @var Account $account */
        $account = $this->getUser();

        return $this->render('identity/dashboard.html.twig', [
            'account' => $account,
            'role' => $account->getRole(),
            'roleLabel' => $account->getRole()->label(),
            // Shown so a reviewer can see which tenant the request resolved to.
            // A Super Admin holds no tenant at all, which is the point of not
            // configuring a role hierarchy.
            'activeTrainerId' => $tenantContext->getTrainerIdOrNull(),
            'trainerLinks' => AccountRole::SuperAdmin === $account->getRole()
                ? []
                : $links->findActiveFor($account),
            // BR-07-1: a disabled feature "disappears from the trainer's UI".
            // A link that leads to a 403 is not a disabled feature, it is a
            // broken one, so the gate is consulted here and not only in the
            // controllers behind these links.
            //
            // Empty when no tenant is resolved, and the template then shows the
            // link. That is deliberate: "disabled for this trainer" and "we do
            // not yet know which trainer" are different states, and only the
            // first should hide anything. FeatureGate itself defaults to
            // enabled for the same reason. A player who has not picked a
            // context yet must still be able to find their content.
            'features' => $this->enabledFeatures($tenantContext, $trainers, $featureGate),
        ]);
    }

    /**
     * @return array<string, bool> feature name => enabled, empty when no tenant
     */
    private function enabledFeatures(
        TenantContext $tenantContext,
        TrainerRepository $trainers,
        FeatureGate $featureGate,
    ): array {
        $trainerId = $tenantContext->getTrainerIdOrNull();

        if (null === $trainerId) {
            // A Super Admin holds no tenant; their own surfaces are never
            // feature-gated, because toggles are platform configuration *about*
            // a trainer rather than about the administrator.
            return [];
        }

        $trainer = $trainers->find($trainerId);

        if (null === $trainer) {
            return [];
        }

        $enabled = [];

        foreach (FeatureToggle::ALL_FEATURES as $feature) {
            $enabled[$feature] = $featureGate->isEnabled($trainer, $feature);
        }

        return $enabled;
    }
}
