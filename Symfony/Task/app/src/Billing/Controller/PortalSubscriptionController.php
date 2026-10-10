<?php

declare(strict_types=1);

namespace App\Billing\Controller;

use App\Billing\Form\SubscriptionPurchaseType;
use App\Billing\Service\SubscriptionPurchaseService;
use App\Billing\Voter\TokenVoter;
use App\Identity\Entity\Account;
use App\Identity\Service\PlayerContextResolver;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * AC-05-29, BR-05-14: Player Subscriptions — "Unlimited Access".
 *
 * @see specs/api-designer-spec.md "Billing module" — player/parent portal table
 */
#[IsGranted('ROLE_PLAYER')]
final class PortalSubscriptionController extends AbstractController
{
    public function __construct(
        private readonly SubscriptionPurchaseService $purchaseService,
        private readonly PlayerContextResolver $playerContext,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    #[Route('/portal/subscription/purchase', name: 'billing_portal_subscription_purchase', methods: ['GET', 'POST'])]
    public function purchase(Request $request): Response
    {
        $this->denyAccessUnlessGranted(TokenVoter::TOKEN_SUBSCRIPTION_PURCHASE);

        $trainer = $this->activeTrainer();
        // Resolving the player keeps this route consistent with every
        // other portal purchase entry point, even though the purchase
        // itself is billed to the acting account, not a specific child —
        // an unlimited-access entitlement covers every child under this
        // parent-trainer pair equally (matching BR-05-2's own
        // parent-trainer token balance shape).
        $this->playerContext->resolve($request, $this->actor());

        $form = $this->createForm(SubscriptionPurchaseType::class);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{activationDate: \DateTimeInterface} $data */
            $data = $form->getData();

            try {
                $checkoutUrl = $this->purchaseService->purchase($trainer, $this->actor(), \DateTimeImmutable::createFromInterface($data['activationDate']));
            } catch (\DomainException $exception) {
                $this->addFlash('error', $exception->getMessage());

                return $this->redirectToRoute('billing_portal_subscription_purchase');
            }

            return $this->redirect($checkoutUrl);
        }

        return $this->render('billing/portal_subscription_purchase.html.twig', ['form' => $form, 'trainer' => $trainer]);
    }

    private function activeTrainer(): Trainer
    {
        /** @var Trainer $trainer */
        $trainer = $this->entityManager->getReference(Trainer::class, $this->tenantContext->requireTrainerId());

        return $trainer;
    }

    private function actor(): Account
    {
        /** @var Account $account */
        $account = $this->getUser();

        return $account;
    }
}
