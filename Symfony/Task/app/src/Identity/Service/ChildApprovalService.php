<?php

declare(strict_types=1);

namespace App\Identity\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\ChildApprovalRequest;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Repository\ChildApprovalRequestRepository;
use App\Identity\Repository\ParentChildLinkRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;

/**
 * US-01.05: a child's RSVP or purchase attempt that needs a parent's
 * sign-off, and the parent's decision on it.
 *
 * The trigger for *creating* a request — a child selecting a paid event
 * (AC-01-25) — belongs to Epic-02/05's own RSVP and purchase workflows, which
 * do not exist yet; `requestApproval()` is the primitive those workflows will
 * call once they do (see `ChildApprovalVoter::CHILD_APPROVAL_BYPASS`'s own
 * consuming-workflow contract). What Epic-01 owns outright and this class
 * fully implements: the request's lifecycle (pending/approved/denied,
 * BR-01-20's notes, the 48-hour read-time expiry) and the per-child bypass
 * toggle it is gated by.
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md US-01.05, AC-01-25..28, BR-01-18..20
 */
final readonly class ChildApprovalService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private ChildApprovalRequestRepository $requests,
        private ParentChildLinkRepository $parentChildLinks,
        private TenantContext $tenantContext,
        private IdentityMailer $mailer,
    ) {
    }

    public function requestApproval(
        Trainer $trainer,
        PlayerProfile $child,
        string $actionType,
        ?int $rsvpId = null,
        ?int $requestedTokenPackageId = null,
        ?int $requestedPlaylistId = null,
    ): ChildApprovalRequest {
        $link = $this->parentChildLinks->findByChildPlayer($child);

        if (null === $link) {
            throw new \LogicException('Cannot request approval for a player with no parent on record.');
        }

        return $this->entityManager->wrapInTransaction(function () use ($trainer, $child, $link, $actionType, $rsvpId, $requestedTokenPackageId, $requestedPlaylistId): ChildApprovalRequest {
            $this->tenantContext->activateFor($trainer);

            $request = new ChildApprovalRequest(
                $trainer,
                $child,
                $link->getParentAccount(),
                $actionType,
                $rsvpId,
                $requestedTokenPackageId,
                $requestedPlaylistId,
            );
            $this->requests->add($request);
            $this->entityManager->flush();

            // AC-01-25: notified by email and in-app. The in-app half is the
            // pending-approvals inbox itself (identity_portal_approvals_index);
            // this is the email half.
            $this->mailer->sendApprovalRequested($request);

            return $request;
        });
    }

    /**
     * AC-01-26, BR-01-20.
     *
     * Fixed to actually persist the decision: this previously mutated
     * $request in memory and returned without ever calling flush() — masked
     * in-process by Doctrine's identity map (a same-request re-fetch by id
     * returns the same managed instance regardless), but a genuine defect
     * for the decision to survive past the current request at all. Found
     * incidentally while wiring Epic-02's own consumer of this same method
     * (RsvpService's post-decision hooks, called right after this by
     * PortalReservationController, need their own writes to commit
     * together with this one).
     */
    public function approve(ChildApprovalRequest $request, ?string $note = null): void
    {
        $this->entityManager->wrapInTransaction(function () use ($request, $note): void {
            $this->tenantContext->activateFor($request->getTrainer());
            $request->approve($note);
            $this->entityManager->flush();
        });
        $this->mailer->sendApprovalDecided($request);
    }

    public function deny(ChildApprovalRequest $request, ?string $note = null): void
    {
        $this->entityManager->wrapInTransaction(function () use ($request, $note): void {
            $this->tenantContext->activateFor($request->getTrainer());
            $request->deny($note);
            $this->entityManager->flush();
        });
        $this->mailer->sendApprovalDecided($request);
    }

    /**
     * AC-01-25/26, BR-01-18: the parent's pending-approvals inbox, with each
     * row's expiry already folded in — `ChildApprovalRequest::displayStatus()`
     * reads "expired" once the 48-hour window has passed, computed at read
     * time, never written back. There is deliberately no
     * "expirePastDeadline()"-shaped method here that flips `status` to
     * denied on a timer: architecture "Time-based state is derived at read
     * time" is explicit that a status column is never mutated by a sweep —
     * only a genuine parent decision (approve()/deny() above) ever changes
     * `status`.
     *
     * @return list<ChildApprovalRequest>
     */
    public function findForParent(Account $parent): array
    {
        return $this->requests->findForParent($parent);
    }
}
