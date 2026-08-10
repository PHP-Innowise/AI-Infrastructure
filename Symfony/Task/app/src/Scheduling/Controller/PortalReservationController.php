<?php

declare(strict_types=1);

namespace App\Scheduling\Controller;

use App\Identity\Entity\Account;
use App\Identity\Entity\ChildApprovalRequest;
use App\Identity\Form\ApprovalDecisionType;
use App\Identity\Service\ChildApprovalService;
use App\Identity\Service\PlayerContextResolver;
use App\Identity\Voter\ChildApprovalVoter;
use App\Scheduling\Entity\Rsvp;
use App\Scheduling\Repository\RsvpRepository;
use App\Scheduling\Service\PlayerAccountResolver;
use App\Scheduling\Service\RsvpService;
use App\Scheduling\Voter\RsvpVoter;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-02.06/07/08: "My Reservations" and canceling out of one.
 *
 * @see specs/api-designer-spec.md "Scheduling module" — player/parent portal table
 */
#[IsGranted('ROLE_PLAYER')]
final class PortalReservationController extends AbstractController
{
    public function __construct(
        private readonly RsvpRepository $rsvps,
        private readonly RsvpService $rsvpService,
        private readonly PlayerContextResolver $playerContext,
        private readonly ChildApprovalService $childApprovalService,
        private readonly PlayerAccountResolver $playerAccounts,
    ) {
    }

    #[Route('/portal/reservations', name: 'scheduling_portal_reservations', methods: ['GET'])]
    public function index(Request $request): Response
    {
        $player = $this->playerContext->resolve($request, $this->actor());
        $now = new \DateTimeImmutable();
        $rows = $this->rsvps->findForPlayer($player);

        return $this->render('scheduling/portal_reservations.html.twig', [
            'upcoming' => array_filter($rows, static fn (Rsvp $r): bool => !$r->getEvent()->hasStarted($now) && !$r->isCanceled()),
            'past' => array_filter($rows, static fn (Rsvp $r): bool => $r->getEvent()->hasStarted($now) || $r->isCanceled()),
        ]);
    }

    /**
     * AC-02-29..33, BR-02-11.
     */
    #[Route('/portal/rsvps/{rsvp}/cancel', name: 'scheduling_portal_rsvp_cancel', methods: ['POST'])]
    public function cancel(Request $request, Rsvp $rsvp): Response
    {
        $this->denyAccessUnlessGranted(RsvpVoter::RSVP_CANCEL, $rsvp);

        if (!$this->isCsrfTokenValid('rsvp-cancel'.$rsvp->getId(), $request->request->getString('_token'))) {
            throw $this->createAccessDeniedException('Invalid CSRF token.');
        }

        $this->rsvpService->cancel($rsvp, $this->actor(), 'Canceled by player.');
        $this->addFlash('success', 'Your RSVP has been canceled.');

        return $this->redirectToRoute('scheduling_portal_reservations');
    }

    /**
     * AC-02-23..26/33: deciding an RSVP-related (creation or cancellation)
     * approval request. Distinct from ApprovalController's own generic
     * decide routes (Identity) because an RSVP-related decision needs a
     * Scheduling-owned follow-up (RsvpService::completeAfterParentApproval()
     * and friends) that Identity must never call directly — Scheduling
     * depends on Identity, never the reverse. Reuses
     * ChildApprovalService/ApprovalDecisionType/the approval_decide template
     * as-is; only the post-decision step differs from ApprovalController's.
     */
    #[Route('/portal/rsvps/approvals/{approval}/approve', name: 'scheduling_portal_rsvp_approval_approve', methods: ['GET', 'POST'])]
    public function approveApproval(Request $request, ChildApprovalRequest $approval): Response
    {
        return $this->decideApproval($request, $approval, true);
    }

    #[Route('/portal/rsvps/approvals/{approval}/deny', name: 'scheduling_portal_rsvp_approval_deny', methods: ['GET', 'POST'])]
    public function denyApproval(Request $request, ChildApprovalRequest $approval): Response
    {
        return $this->decideApproval($request, $approval, false);
    }

    private function decideApproval(Request $request, ChildApprovalRequest $approval, bool $approving): Response
    {
        $this->denyAccessUnlessGranted(ChildApprovalVoter::CHILD_APPROVAL_DECIDE, $approval);

        if (!\in_array($approval->getActionType(), [ChildApprovalRequest::ACTION_RSVP, ChildApprovalRequest::ACTION_RSVP_CANCELLATION], true)) {
            throw $this->createNotFoundException('Not an RSVP-related approval request.');
        }

        $form = $this->createForm(ApprovalDecisionType::class, null, ['label' => $approving ? 'Approve' : 'Deny']);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{note: ?string} $data */
            $data = $form->getData();

            if ($approving) {
                $this->childApprovalService->approve($approval, $data['note']);
            } else {
                $this->childApprovalService->deny($approval, $data['note']);
            }

            $this->applyDecisionToRsvp($approval, $approving);
            $this->addFlash('success', $approving ? 'Request approved.' : 'Request denied.');

            return $this->redirectToRoute('identity_portal_approvals_index');
        }

        return $this->render('identity/approval_decide.html.twig', [
            'form' => $form,
            'approval' => $approval,
            'approving' => $approving,
        ]);
    }

    private function applyDecisionToRsvp(ChildApprovalRequest $approval, bool $approving): void
    {
        $rsvpId = $approval->getRsvpId();

        if (null === $rsvpId) {
            return;
        }

        $rsvp = $this->rsvps->find($rsvpId);

        if (null === $rsvp) {
            return;
        }

        $payer = $this->playerAccounts->resolve($rsvp->getPlayer()) ?? $approval->getParentAccount();

        match (true) {
            $approving && ChildApprovalRequest::ACTION_RSVP === $approval->getActionType() => $this->rsvpService->completeAfterParentApproval($rsvp, $payer),
            !$approving && ChildApprovalRequest::ACTION_RSVP === $approval->getActionType() => $this->rsvpService->cancelAfterParentDenial($rsvp),
            $approving && ChildApprovalRequest::ACTION_RSVP_CANCELLATION === $approval->getActionType() => $this->rsvpService->completeCancellationAfterApproval($rsvp, $payer),
            // A denied cancellation request: no state change — the RSVP
            // simply stays as it was.
            default => null,
        };
    }

    private function actor(): Account
    {
        /** @var Account $account */
        $account = $this->getUser();

        return $account;
    }
}
