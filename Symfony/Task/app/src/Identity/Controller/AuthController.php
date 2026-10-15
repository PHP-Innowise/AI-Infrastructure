<?php

declare(strict_types=1);

namespace App\Identity\Controller;

use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Core\Exception\CustomUserMessageAccountStatusException;
use Symfony\Component\Security\Http\Authentication\AuthenticationUtils;

/**
 * Authentication entry points.
 *
 * AC-01-5, AC-01-36. The login form is handled by Symfony's form_login
 * authenticator; this controller only renders it and surfaces the error.
 */
final class AuthController extends AbstractController
{
    #[Route('/login', name: 'identity_auth_login', methods: ['GET', 'POST'])]
    public function login(AuthenticationUtils $authenticationUtils): Response
    {
        if ($this->getUser()) {
            return $this->redirectToRoute('app_dashboard');
        }

        $error = $authenticationUtils->getLastAuthenticationError();

        return $this->render('identity/login.html.twig', [
            'last_username' => $authenticationUtils->getLastUsername(),
            // Deliberately the framework's generic message for a genuine
            // wrong-password/no-such-account attempt: a login form that
            // distinguishes the two is an account-enumeration oracle.
            'error' => $error,
            // AC-01-52's edge case is a different situation, not an
            // enumeration risk: these credentials are correct, so the
            // account owner already knows they exist. AccountStatusUserChecker
            // is the only source of this specific exception type.
            'accountStatusMessage' => $error instanceof CustomUserMessageAccountStatusException ? $error->getMessage() : null,
        ]);
    }

    #[Route('/logout', name: 'identity_auth_logout', methods: ['GET', 'POST'])]
    public function logout(): never
    {
        throw new \LogicException('Intercepted by the firewall\'s logout listener.');
    }
}
