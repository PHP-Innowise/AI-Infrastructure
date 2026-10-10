<?php

declare(strict_types=1);

namespace App\Identity\Controller;

use App\Identity\Exception\TokenNotUsableException;
use App\Identity\Form\SetNewPasswordType;
use App\Identity\Service\PasswordResetService;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;

/**
 * AC-01-3/4/5: first-login credential setup after a Super Admin creates a
 * trainer. Same token/consumption protocol as password reset — see
 * PasswordResetToken's own docblock for why there is no separate token type.
 */
final class AccountSetupController extends AbstractController
{
    public function __construct(
        private readonly PasswordResetService $passwordResetService,
    ) {
    }

    #[Route('/account/setup/{token}', name: 'identity_account_setup', methods: ['GET', 'POST'])]
    public function __invoke(Request $request, string $token): Response
    {
        $form = $this->createForm(SetNewPasswordType::class);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{plainPassword: string} $data */
            $data = $form->getData();

            try {
                $this->passwordResetService->consume($token, $data['plainPassword']);
            } catch (TokenNotUsableException) {
                return $this->render('identity/token_not_usable.html.twig', [], new Response(status: 404));
            }

            $this->addFlash('success', 'Your account is set up. You can now sign in.');

            // AC-01-5: the new trainer can log in and access the dashboard —
            // this redirect starts that path.
            return $this->redirectToRoute('identity_auth_login');
        }

        return $this->render('identity/account_setup.html.twig', ['form' => $form, 'token' => $token]);
    }
}
