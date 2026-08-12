<?php

declare(strict_types=1);

namespace App\Administration\Controller;

use App\Identity\Entity\Account;
use App\Platform\Voter\ImpersonationVoter;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-01.07. AC-01-33/34: confirmation happens in the Users tool UI (a modal
 * naming the target and role) before this POST is ever reached — this
 * controller checks CSRF and hands off to Symfony's native `switch_user`
 * mechanism via the redirect's `_switch_user` query parameter.
 *
 * It no longer opens the ImpersonationSession itself. Recording the start
 * here recorded only impersonations that came through here, and the
 * `_switch_user` parameter works on every URL in the application — so the
 * audit trail had a way around it that the swap itself did not.
 * `SwitchUserAuditSubscriber` now writes both ends in response to the swap,
 * which is the event no path can skip.
 *
 * The voter call below is kept as a pre-flight: the firewall will ask the
 * same voter the same question one redirect later, but failing here turns a
 * forbidden target into a 403 on the button the admin actually pressed,
 * rather than after a redirect to somewhere else.
 */
#[IsGranted('ROLE_SUPER_ADMIN')]
final class ImpersonationController extends AbstractController
{
    #[Route('/super-admin/users/{account}/impersonate', name: 'administration_impersonation_start', methods: ['POST'])]
    public function start(Request $request, Account $account): Response
    {
        // BR-01-21/AC-01-37: denies targeting another Super Admin.
        $this->denyAccessUnlessGranted(ImpersonationVoter::IMPERSONATION_START, $account);

        if (!$this->isCsrfTokenValid('impersonate'.$account->getId(), $request->request->getString('_token'))) {
            throw $this->createAccessDeniedException('Invalid CSRF token.');
        }

        // Symfony's switch_user firewall listener intercepts this query
        // parameter on the very next request, re-checks the same voter, swaps
        // the token, and dispatches the event SwitchUserAuditSubscriber
        // records — see security.yaml's switch_user config.
        return $this->redirectToRoute('app_dashboard', ['_switch_user' => $account->getEmail()]);
    }
}
