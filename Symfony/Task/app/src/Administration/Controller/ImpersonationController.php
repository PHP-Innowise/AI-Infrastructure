<?php

declare(strict_types=1);

namespace App\Administration\Controller;

use App\Identity\Entity\Account;
use App\Platform\Service\ImpersonationService;
use App\Platform\Voter\ImpersonationVoter;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-01.07. AC-01-33/34: confirmation happens in the Users tool UI (a modal
 * naming the target and role) before this POST is ever reached — this
 * controller performs the authorization check BR-01-21/AC-01-37 needs, then
 * hands off to Symfony's native `switch_user` mechanism via the redirect's
 * `_switch_user` query parameter.
 */
#[IsGranted('ROLE_SUPER_ADMIN')]
final class ImpersonationController extends AbstractController
{
    public function __construct(
        private readonly ImpersonationService $impersonationService,
    ) {
    }

    #[Route('/super-admin/users/{account}/impersonate', name: 'administration_impersonation_start', methods: ['POST'])]
    public function start(Request $request, Account $account): Response
    {
        // BR-01-21/AC-01-37: denies targeting another Super Admin.
        $this->denyAccessUnlessGranted(ImpersonationVoter::IMPERSONATION_START, $account);

        if (!$this->isCsrfTokenValid('impersonate'.$account->getId(), $request->request->getString('_token'))) {
            throw $this->createAccessDeniedException('Invalid CSRF token.');
        }

        /** @var Account $admin */
        $admin = $this->getUser();
        $this->impersonationService->start($admin, $account);

        // Symfony's switch_user firewall listener intercepts this query
        // parameter on the very next request and swaps the token — see
        // security.yaml's switch_user config and SwitchUserAuditSubscriber.
        return $this->redirectToRoute('app_dashboard', ['_switch_user' => $account->getEmail()]);
    }
}
