<?php

declare(strict_types=1);

namespace App\Controller;

use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;

/**
 * Placeholder landing page for the walking skeleton.
 *
 * Epic-01 replaces this: an unauthenticated visitor will be routed to login or
 * to the trainer's public surface depending on how they arrived. It exists now
 * only so the skeleton renders the layout and the design-token pipeline end to
 * end rather than answering 404 at the root.
 */
final class HomeController extends AbstractController
{
    #[Route('/', name: 'app_home', methods: ['GET'])]
    public function __invoke(): Response
    {
        return $this->render('home/index.html.twig');
    }
}
