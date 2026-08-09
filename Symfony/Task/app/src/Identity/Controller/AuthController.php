<?php

declare(strict_types=1);

namespace App\Identity\Controller;

use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
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

        return $this->render('identity/login.html.twig', [
            'last_username' => $authenticationUtils->getLastUsername(),
            // Deliberately the framework's generic message: a login form that
            // distinguishes "no such account" from "wrong password" is an
            // account-enumeration oracle.
            'error' => $authenticationUtils->getLastAuthenticationError(),
        ]);
    }

    #[Route('/logout', name: 'identity_auth_logout', methods: ['GET', 'POST'])]
    public function logout(): never
    {
        throw new \LogicException('Intercepted by the firewall\'s logout listener.');
    }
}
