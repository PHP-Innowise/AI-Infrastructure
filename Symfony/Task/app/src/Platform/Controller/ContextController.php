<?php

declare(strict_types=1);

namespace App\Platform\Controller;

use App\Identity\Entity\Account;
use App\Platform\Repository\AccountTrainerLinkRepository;
use App\Platform\Tenancy\TenantResolver;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * AC-01-15: the trainer context switcher — multi-trainer players switch
 * between fully isolated contexts, like switching accounts. Re-validated on
 * every subsequent request by TenantResolver source 3, not trusted blindly
 * here — this route only records the selection.
 */
#[IsGranted('ROLE_PLAYER')]
final class ContextController extends AbstractController
{
    public function __construct(
        private readonly AccountTrainerLinkRepository $accountTrainerLinks,
    ) {
    }

    #[Route('/context/trainer/{trainer<\d+>}', name: 'platform_context_trainer_switch', methods: ['POST'])]
    public function __invoke(Request $request, int $trainer): Response
    {
        /** @var Account $account */
        $account = $this->getUser();

        if (!$this->isCsrfTokenValid('trainer-context'.$trainer, $request->request->getString('_token'))) {
            throw $this->createAccessDeniedException('Invalid CSRF token.');
        }

        if (!$this->accountTrainerLinks->isActiveLink($account, $trainer)) {
            throw $this->createAccessDeniedException('Not an active trainer relationship for this account.');
        }

        $request->getSession()->set(TenantResolver::SESSION_KEY, $trainer);

        return $this->redirect($request->headers->get('referer') ?? $this->generateUrl('app_dashboard'));
    }
}
