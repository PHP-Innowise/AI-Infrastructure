<?php

declare(strict_types=1);

namespace App\Billing\Controller;

use App\Billing\Entity\TokenPackage;
use App\Billing\Form\TokenPurchaseType;
use App\Billing\Repository\TokenEntryRepository;
use App\Billing\Repository\TokenPackageRepository;
use App\Billing\Service\PlayerAccountResolver;
use App\Billing\Service\TokenLedgerService;
use App\Billing\Service\TokenPurchaseService;
use App\Billing\Voter\TokenVoter;
use App\Identity\Entity\Account;
use App\Identity\Entity\ChildApprovalRequest;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Form\ApprovalDecisionType;
use App\Identity\Repository\PlayerProfileRepository;
use App\Identity\Service\ChildApprovalService;
use App\Identity\Service\PlayerContextResolver;
use App\Identity\Voter\ChildApprovalVoter;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-05.02: buying and viewing tokens.
 *
 * @see specs/api-designer-spec.md "Billing module" — player/parent portal table
 */
#[IsGranted('ROLE_PLAYER')]
final class PortalTokenController extends AbstractController
{
    public function __construct(
        private readonly TokenLedgerService $tokenLedger,
        private readonly TokenEntryRepository $tokenEntries,
        private readonly TokenPackageRepository $tokenPackages,
        private readonly TokenPurchaseService $purchaseService,
        private readonly PlayerAccountResolver $playerAccounts,
        private readonly PlayerContextResolver $playerContext,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
        private readonly ChildApprovalService $childApprovalService,
        private readonly PlayerProfileRepository $playerProfiles,
    ) {
    }

    /**
     * AC-05-4/BR-05-2: "You have N tokens with [Trainer]" — the balance
     * belongs to the PARENT, not the child (AC-05-6: a child viewing this
     * sees the parent's own balance).
     */
    #[Route('/portal/tokens', name: 'billing_portal_tokens', methods: ['GET'])]
    public function index(Request $request): Response
    {
        $trainer = $this->activeTrainer();
        $player = $this->playerContext->resolve($request, $this->actor());
        $parentAccount = $this->playerAccounts->resolve($player) ?? $this->actor();

        return $this->render('billing/portal_tokens.html.twig', [
            'balance' => $this->tokenLedger->balanceFor($trainer, $parentAccount),
            'trainer' => $trainer,
            'recentActivity' => $this->tokenEntries->findRecentForTrainerAndParent($trainer, $parentAccount),
        ]);
    }

    #[Route('/portal/tokens/purchase', name: 'billing_portal_tokens_purchase', methods: ['GET', 'POST'])]
    public function purchase(Request $request): Response
    {
        $this->denyAccessUnlessGranted(TokenVoter::TOKEN_PURCHASE);

        $trainer = $this->activeTrainer();
        $player = $this->playerContext->resolve($request, $this->actor());
        $packages = $this->tokenPackages->findActiveForTrainer($trainer);

        $form = $this->createForm(TokenPurchaseType::class, null, ['packages' => $packages]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{packageId: ?int, customTokenCount: ?int} $data */
            $data = $form->getData();

            if (null === $data['packageId'] && (null === $data['customTokenCount'] || $data['customTokenCount'] <= 0)) {
                $this->addFlash('error', 'Choose a package or enter a custom token amount.');

                return $this->redirectToRoute('billing_portal_tokens_purchase');
            }

            $checkoutUrl = null !== $data['packageId']
                ? $this->purchaseFromPackage($trainer, $player, $data['packageId'], $packages)
                : $this->purchaseService->purchaseCustomAmount($trainer, $player, $this->actor(), (int) $data['customTokenCount']);

            if (null === $checkoutUrl) {
                $this->addFlash('info', 'Your purchase request has been submitted and is pending parent approval.');

                return $this->redirectToRoute('billing_portal_tokens');
            }

            return $this->redirect($checkoutUrl);
        }

        return $this->render('billing/portal_tokens_purchase.html.twig', ['form' => $form, 'trainer' => $trainer]);
    }

    /**
     * @param list<TokenPackage> $packages
     */
    private function purchaseFromPackage(Trainer $trainer, PlayerProfile $player, int $packageId, array $packages): ?string
    {
        foreach ($packages as $package) {
            if ($package->getId() === $packageId) {
                return $this->purchaseService->purchasePackage($trainer, $player, $this->actor(), $package);
            }
        }

        throw $this->createNotFoundException('Unknown token package.');
    }

    /**
     * AC-05-6: deciding a pending ACTION_TOKEN_PURCHASE request — distinct
     * from `App\Identity\Controller\ApprovalController`'s own generic
     * decide routes because Identity must never call into Billing directly
     * (module dependency direction), mirroring
     * `PortalReservationController::decideApproval()`'s and
     * `PortalContentController::decidePurchaseApproval()`'s own precedent.
     *
     * Unlike the RSVP case, `TokenPurchaseService::completeAfterParentApproval()`
     * always returns a real Checkout URL (never null — a token purchase is
     * always card-funded, BR-05-1/AC-05-4), so approval redirects straight
     * there unconditionally, with no `?->getPendingCheckoutUrl()` shape to
     * check first.
     */
    #[Route('/portal/tokens/purchase-approvals/{approval<\d+>}/approve', name: 'billing_portal_token_purchase_approval_approve', methods: ['GET', 'POST'])]
    public function purchaseApprovalApprove(Request $request, ChildApprovalRequest $approval): Response
    {
        return $this->decidePurchaseApproval($request, $approval, true);
    }

    #[Route('/portal/tokens/purchase-approvals/{approval<\d+>}/deny', name: 'billing_portal_token_purchase_approval_deny', methods: ['GET', 'POST'])]
    public function purchaseApprovalDeny(Request $request, ChildApprovalRequest $approval): Response
    {
        return $this->decidePurchaseApproval($request, $approval, false);
    }

    private function decidePurchaseApproval(Request $request, ChildApprovalRequest $approval, bool $approving): Response
    {
        $this->denyAccessUnlessGranted(ChildApprovalVoter::CHILD_APPROVAL_DECIDE, $approval);

        if (ChildApprovalRequest::ACTION_TOKEN_PURCHASE !== $approval->getActionType()) {
            throw $this->createNotFoundException('Not a token-purchase approval request.');
        }

        $form = $this->createForm(ApprovalDecisionType::class, null, ['label' => $approving ? 'Approve' : 'Deny']);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{note: ?string} $data */
            $data = $form->getData();

            if ($approving) {
                $this->childApprovalService->approve($approval, $data['note']);
                $checkoutUrl = $this->completePurchaseAfterApproval($approval);

                if (null !== $checkoutUrl) {
                    return $this->redirect($checkoutUrl);
                }
            } else {
                $this->childApprovalService->deny($approval, $data['note']);
            }

            $this->addFlash('success', $approving ? 'Request approved.' : 'Request denied.');

            return $this->redirectToRoute('identity_portal_approvals_index');
        }

        return $this->render('identity/approval_decide.html.twig', [
            'form' => $form,
            'approval' => $approval,
            'approving' => $approving,
        ]);
    }

    private function completePurchaseAfterApproval(ChildApprovalRequest $approval): ?string
    {
        $packageId = $approval->getRequestedTokenPackageId();

        if (null === $packageId) {
            return null;
        }

        $package = $this->tokenPackages->find($packageId);

        if (null === $package) {
            return null;
        }

        $child = $this->playerProfiles->find($approval->getChildPlayer()->getId());

        if (null === $child) {
            return null;
        }

        return $this->purchaseService->completeAfterParentApproval($approval->getTrainer(), $child, $approval->getParentAccount(), $package);
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
