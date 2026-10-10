<?php

declare(strict_types=1);

namespace App\Billing\Controller;

use App\Billing\Service\StripeGateway;
use App\Billing\Voter\PaymentMethodVoter;
use App\Identity\Entity\Account;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Routing\Generator\UrlGeneratorInterface;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * AC-05-20/21: payment methods are managed entirely via the Stripe Customer
 * Portal, no custom UI.
 *
 * @see specs/api-designer-spec.md "Billing module" — player/parent portal table
 */
#[IsGranted('ROLE_PLAYER')]
final class PortalPaymentMethodController extends AbstractController
{
    public function __construct(
        private readonly StripeGateway $stripeGateway,
        private readonly UrlGeneratorInterface $urlGenerator,
    ) {
    }

    #[Route('/portal/billing/payment-methods/manage', name: 'billing_portal_payment_methods_manage', methods: ['POST'])]
    public function manage(Request $request): Response
    {
        $this->denyAccessUnlessGranted(PaymentMethodVoter::PAYMENT_METHOD_MANAGE);

        if (!$this->isCsrfTokenValid('payment-methods-manage', $request->request->getString('_token'))) {
            throw $this->createAccessDeniedException('Invalid CSRF token.');
        }

        /** @var Account $actor */
        $actor = $this->getUser();

        $url = $this->stripeGateway->billingPortalUrlFor(
            $actor,
            $this->urlGenerator->generate('billing_portal_tokens', [], UrlGeneratorInterface::ABSOLUTE_URL),
        );

        return $this->redirect($url);
    }
}
