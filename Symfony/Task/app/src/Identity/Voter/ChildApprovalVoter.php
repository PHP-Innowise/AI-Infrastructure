<?php

declare(strict_types=1);

namespace App\Identity\Voter;

use App\Identity\Entity\Account;
use App\Identity\Entity\ChildApprovalRequest;
use App\Identity\Repository\ParentChildLinkRepository;
use App\Identity\Service\ChildActionAttempt;
use App\Identity\Service\FundingMethod;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * Two unrelated attributes sharing one class because they both govern the
 * same business relationship (BR-01-17..20).
 *
 * `CHILD_APPROVAL_BYPASS` answers "does this attempt execute now, or wait
 * for a parent" — never itself a grant/deny of the underlying action (a
 * child genuinely CAN RSVP, AC-01-29). Denial does not mean 403: the calling
 * workflow (once Epic-02/05 exist) branches to creating a
 * `ChildApprovalRequest` instead of throwing. See
 * `specs/security-voter-designer-design.md` "Parent-child approval" for the
 * full truth table this method implements verbatim.
 *
 * `CHILD_APPROVAL_DECIDE` is the parent's own act of approving/denying an
 * already-pending request (AC-01-26). It independently re-applies the
 * 48-hour cutoff (not only the read-side "Expired" label) so a late parent
 * can never override an outcome BR-01-18 already auto-settled.
 *
 * @see specs/security-voter-designer-design.md "Parent-child approval"
 * @see specs/requirements-analyst-epic-01-user-management-spec.md BR-01-17..20, AC-01-25..29
 */
/**
 * @extends Voter<string, ChildApprovalRequest|ChildActionAttempt>
 */
final class ChildApprovalVoter extends Voter
{
    public const CHILD_APPROVAL_DECIDE = 'CHILD_APPROVAL_DECIDE';
    public const CHILD_APPROVAL_BYPASS = 'CHILD_APPROVAL_BYPASS';

    public function __construct(
        private readonly ParentChildLinkRepository $parentChildLinks,
    ) {
    }

    protected function supports(string $attribute, mixed $subject): bool
    {
        return match ($attribute) {
            self::CHILD_APPROVAL_DECIDE => $subject instanceof ChildApprovalRequest,
            self::CHILD_APPROVAL_BYPASS => $subject instanceof ChildActionAttempt,
            default => false,
        };
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        return match (true) {
            self::CHILD_APPROVAL_DECIDE === $attribute && $subject instanceof ChildApprovalRequest => $this->voteDecide($subject, $token),
            self::CHILD_APPROVAL_BYPASS === $attribute && $subject instanceof ChildActionAttempt => $this->voteBypass($subject),
            default => false,
        };
    }

    private function voteDecide(ChildApprovalRequest $request, TokenInterface $token): bool
    {
        $actor = $token->getUser();

        if (!$actor instanceof Account) {
            return false;
        }

        $link = $this->parentChildLinks->findByChildPlayer($request->getChildPlayer());

        if (null === $link || $link->getParentAccount() !== $actor) {
            return false;
        }

        return !$request->isExpired(new \DateTimeImmutable());
    }

    private function voteBypass(ChildActionAttempt $attempt): bool
    {
        // BR-01-17 scopes the whole workflow to minors — an adult's own
        // attempt is never gated.
        if (!$attempt->beneficiary->isChild()) {
            return true;
        }

        $link = $this->parentChildLinks->findByChildPlayer($attempt->beneficiary);

        // The parent's own action already IS the approval — never queued.
        if (null !== $link && $link->getParentAccount() === $attempt->actor) {
            return true;
        }

        // From here the actor is (or claims to be) the child's own login. A
        // stranger with no ParentChildLink edge at all is stopped earlier, by
        // the action's own base voter (RsvpVoter/TokenVoter ownership check),
        // before this attribute is ever consulted.
        return match ($attempt->fundingMethod) {
            // BR-01-18: no toggle reaches USD, ever.
            FundingMethod::Usd => false,
            // BR-02-10: free RSVPs are not exempted either.
            FundingMethod::Free => false,
            // BR-01-19/AC-01-27: bypass only if this specific child's toggle is ON.
            FundingMethod::Token => null !== $link && $link->allowsTokenSpendingWithoutApproval(),
        };
    }
}
