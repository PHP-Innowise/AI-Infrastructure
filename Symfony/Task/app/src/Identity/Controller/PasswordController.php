<?php

declare(strict_types=1);

namespace App\Identity\Controller;

use App\Identity\Exception\TokenNotUsableException;
use App\Identity\Form\RequestPasswordResetType;
use App\Identity\Form\SetNewPasswordType;
use App\Identity\Service\PasswordResetService;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;

/**
 * BR-01-4, AC-01-66: the forgot/reset password flow.
 */
final class PasswordController extends AbstractController
{
    public function __construct(
        private readonly PasswordResetService $passwordResetService,
    ) {
    }

    #[Route('/password/forgot', name: 'identity_password_forgot', methods: ['GET', 'POST'])]
    public function forgot(Request $request): Response
    {
        $form = $this->createForm(RequestPasswordResetType::class);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{email: string} $data */
            $data = $form->getData();
            // Enumeration-safe: identical outcome whether or not the email
            // exists (api-designer-spec's own default).
            $this->passwordResetService->requestReset($data['email']);

            $this->addFlash('success', 'If that email exists, a reset link is on its way.');

            return $this->redirectToRoute('identity_auth_login');
        }

        return $this->render('identity/password_forgot.html.twig', ['form' => $form]);
    }

    #[Route('/password/reset/{token}', name: 'identity_password_reset', methods: ['GET', 'POST'])]
    public function reset(Request $request, string $token): Response
    {
        $form = $this->createForm(SetNewPasswordType::class);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{plainPassword: string} $data */
            $data = $form->getData();

            try {
                $this->passwordResetService->consume($token, $data['plainPassword']);
            } catch (TokenNotUsableException) {
                // Deliberately generic — never confirms whether a token was
                // ever valid.
                return $this->render('identity/token_not_usable.html.twig', [], new Response(status: 404));
            }

            $this->addFlash('success', 'Your password has been reset. You can now sign in.');

            return $this->redirectToRoute('identity_auth_login');
        }

        return $this->render('identity/password_reset.html.twig', ['form' => $form, 'token' => $token]);
    }
}
