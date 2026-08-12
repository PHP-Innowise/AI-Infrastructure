<?php

declare(strict_types=1);

namespace App\Identity\Controller;

use App\Identity\Entity\Account;
use App\Identity\Exception\TokenNotUsableException;
use App\Identity\Service\EmailVerificationService;
use App\Identity\Voter\AccountVoter;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * BR-01-5, AC-01-67.
 */
final class EmailVerificationController extends AbstractController
{
    public function __construct(
        private readonly EmailVerificationService $emailVerificationService,
    ) {
    }

    #[Route('/email/verify/{token}', name: 'identity_email_verify', methods: ['GET'])]
    public function verify(string $token): Response
    {
        try {
            $this->emailVerificationService->verify($token);
        } catch (TokenNotUsableException) {
            return $this->render('identity/token_not_usable.html.twig', [], new Response(status: 404));
        }

        $this->addFlash('success', 'Your email address has been verified.');

        return $this->redirectToRoute('identity_auth_login');
    }

    #[Route('/email/verify/resend', name: 'identity_email_verify_resend', methods: ['POST'])]
    #[IsGranted('IS_AUTHENTICATED_FULLY')]
    public function resend(Request $request): Response
    {
        /** @var Account $account */
        $account = $this->getUser();
        $this->denyAccessUnlessGranted(AccountVoter::EMAIL_VERIFY_RESEND, $account);

        if (!$this->isCsrfTokenValid('email-verify-resend', $request->request->getString('_token'))) {
            throw $this->createAccessDeniedException('Invalid CSRF token.');
        }

        $this->emailVerificationService->sendVerification($account);
        $this->addFlash('success', 'Verification email sent.');

        return $this->redirectToRoute('app_dashboard');
    }
}
