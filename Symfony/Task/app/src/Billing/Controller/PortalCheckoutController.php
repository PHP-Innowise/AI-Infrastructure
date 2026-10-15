<?php

declare(strict_types=1);

namespace App\Billing\Controller;

use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * BR-05-6: "the user is redirected back to the platform after payment, and
 * a webhook confirms payment success asynchronously" — these two pages are
 * pure waypoints; the ACTUAL confirmation is
 * `App\Billing\MessageHandler\ProcessStripeWebhookEventHandler`, never this
 * request.
 *
 * @see specs/api-designer-spec.md "Billing module" — player/parent portal table
 */
#[IsGranted('ROLE_PLAYER')]
final class PortalCheckoutController extends AbstractController
{
    #[Route('/portal/checkout/success', name: 'billing_portal_checkout_success', methods: ['GET'])]
    public function success(Request $request): Response
    {
        return $this->render('billing/portal_checkout_success.html.twig', [
            'context' => $request->query->getString('context'),
        ]);
    }

    #[Route('/portal/checkout/cancel', name: 'billing_portal_checkout_cancel', methods: ['GET'])]
    public function cancel(Request $request): Response
    {
        return $this->render('billing/portal_checkout_cancel.html.twig', [
            'context' => $request->query->getString('context'),
        ]);
    }
}
