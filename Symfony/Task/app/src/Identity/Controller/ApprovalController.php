<?php

declare(strict_types=1);

namespace App\Identity\Controller;

use App\Identity\Entity\Account;
use App\Identity\Entity\ChildApprovalRequest;
use App\Identity\Form\ApprovalDecisionType;
use App\Identity\Service\ChildApprovalService;
use App\Identity\Voter\ChildApprovalVoter;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-01.05: the parent's pending-approvals inbox and their decisions.
 */
#[IsGranted('ROLE_PLAYER')]
final class ApprovalController extends AbstractController
{
    public function __construct(
        private readonly ChildApprovalService $childApprovalService,
    ) {
    }

    /**
     * AC-01-25: expired-but-unprocessed rows show "Expired", computed at
     * read time from `ChildApprovalRequest::displayStatus()`.
     */
    #[Route('/portal/approvals', name: 'identity_portal_approvals_index', methods: ['GET'])]
    public function index(): Response
    {
        /** @var Account $parent */
        $parent = $this->getUser();

        return $this->render('identity/approvals_index.html.twig', [
            'requests' => $this->childApprovalService->findForParent($parent),
            'now' => new \DateTimeImmutable(),
        ]);
    }

    #[Route('/portal/approvals/{approval<\d+>}/approve', name: 'identity_portal_approval_approve', methods: ['GET', 'POST'])]
    public function approve(Request $request, ChildApprovalRequest $approval): Response
    {
        return $this->decide($request, $approval, true);
    }

    #[Route('/portal/approvals/{approval<\d+>}/deny', name: 'identity_portal_approval_deny', methods: ['GET', 'POST'])]
    public function deny(Request $request, ChildApprovalRequest $approval): Response
    {
        return $this->decide($request, $approval, false);
    }

    private function decide(Request $request, ChildApprovalRequest $approval, bool $approving): Response
    {
        $this->denyAccessUnlessGranted(ChildApprovalVoter::CHILD_APPROVAL_DECIDE, $approval);

        // AC-02-23..26/33: an RSVP-related request (actionType 'rsvp' or
        // 'rsvp_cancellation') needs Scheduling's own post-decision step —
        // deciding it here would flip the request's own status without ever
        // confirming/canceling the underlying Rsvp, leaving it stuck.
        // Identity must not call into Scheduling directly (module
        // dependency direction), so this route simply refuses the request
        // and sends the parent to the one that can:
        // PortalReservationController's own decide actions. The approvals
        // index template already links there directly for these types;
        // this is defense against a stale or hand-typed URL.
        if (\in_array($approval->getActionType(), [ChildApprovalRequest::ACTION_RSVP, ChildApprovalRequest::ACTION_RSVP_CANCELLATION], true)) {
            return $this->redirectToRoute($approving ? 'scheduling_portal_rsvp_approval_approve' : 'scheduling_portal_rsvp_approval_deny', ['approval' => $approval->getId()]);
        }

        // Epic-04, same reasoning: a content-purchase request needs
        // Content's own post-decision step (granting a PlaylistAccessGrant
        // on approval) — Identity must not call into Content directly.
        if (ChildApprovalRequest::ACTION_CONTENT_PURCHASE === $approval->getActionType()) {
            return $this->redirectToRoute($approving ? 'content_portal_purchase_approval_approve' : 'content_portal_purchase_approval_deny', ['approval' => $approval->getId()]);
        }

        // Epic-05, same reasoning again: a token-purchase request needs
        // Billing's own post-decision step (starting the Stripe Checkout
        // session on approval, AC-05-6) — Identity must not call into
        // Billing directly.
        if (ChildApprovalRequest::ACTION_TOKEN_PURCHASE === $approval->getActionType()) {
            return $this->redirectToRoute($approving ? 'billing_portal_token_purchase_approval_approve' : 'billing_portal_token_purchase_approval_deny', ['approval' => $approval->getId()]);
        }

        $form = $this->createForm(ApprovalDecisionType::class, null, ['label' => $approving ? 'Approve' : 'Deny']);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{note: ?string} $data */
            $data = $form->getData();

            if ($approving) {
                $this->childApprovalService->approve($approval, $data['note']);
                $this->addFlash('success', 'Request approved.');
            } else {
                $this->childApprovalService->deny($approval, $data['note']);
                $this->addFlash('success', 'Request denied.');
            }

            return $this->redirectToRoute('identity_portal_approvals_index');
        }

        return $this->render('identity/approval_decide.html.twig', [
            'form' => $form,
            'approval' => $approval,
            'approving' => $approving,
        ]);
    }
}
