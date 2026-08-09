<?php

declare(strict_types=1);

namespace App\Identity\Controller;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Platform\Repository\AccountTrainerLinkRepository;
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
 */
final class DashboardController extends AbstractController
{
    #[Route('/dashboard', name: 'app_dashboard', methods: ['GET'])]
    #[IsGranted('IS_AUTHENTICATED_FULLY')]
    public function __invoke(
        TenantContext $tenantContext,
        AccountTrainerLinkRepository $links,
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
        ]);
    }
}
