<?php

declare(strict_types=1);

namespace App\Administration\Controller;

use App\Platform\Voter\ImpersonationVoter;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * AC-01-35: returns the Super Admin to their own view. Reachable from any
 * prefix — gated on `IS_IMPERSONATOR`, Symfony's `switch_user` marker role,
 * not on any specific business role, since the acting session is currently
 * running under the target's own role by design.
 *
 * The actual identity restoration is Symfony's native `switch_user` exit
 * mechanism (`?_switch_user=_exit`); `SwitchUserAuditSubscriber` records the
 * ImpersonationSession's end in response to that same event, so this
 * controller only needs to trigger it.
 */
final class ImpersonationExitController extends AbstractController
{
    #[Route('/impersonation/exit', name: 'impersonation_exit', methods: ['POST'])]
    #[IsGranted(ImpersonationVoter::IMPERSONATION_END)]
    public function __invoke(Request $request): Response
    {
        if (!$this->isCsrfTokenValid('impersonation-exit', $request->request->getString('_token'))) {
            throw $this->createAccessDeniedException('Invalid CSRF token.');
        }

        // specs/api-designer-spec.md:691: "Redirect to administration_dashboard"
        // — Epic-07 now provides that route (it did not exist when this
        // controller was first built in Epic-01, hence app_dashboard as a
        // stand-in then).
        return $this->redirectToRoute('administration_dashboard', ['_switch_user' => '_exit']);
    }
}
