<?php

declare(strict_types=1);

namespace App\Scheduling\Voter;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Identity\Service\PlayerContextResolver;
use App\Platform\Tenancy\AdministrativeScope;
use App\Scheduling\Entity\Event;
use App\Scheduling\Service\EventEligibilityChecker;
use Symfony\Component\HttpFoundation\RequestStack;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * BR-02-1..6/12/13/19, AC-02-1..17, AC-02-43..54.
 *
 * `EVENT_EDIT`/`EVENT_CANCEL`/`EVENT_VIEW_RSVP_LIST` each carry a
 * `+ SUPER_ADMIN via AdministrativeScope` branch — Event Master "edits the
 * event as if they had created it" (AC-07-25) and overrides
 * scheduling-conflict warnings (AC-07-27/28) — matching
 * `specs/security-voter-designer-design.md`'s test-matrix row for this
 * voter exactly. `EVENT_DUPLICATE`/`EVENT_EXPORT_RSVPS`/
 * `EVENT_MANUAL_ADD_PLAYER` carry no Super Admin clause — no AC states that
 * capability, a natural absence rather than an unresolved mechanism (same
 * reasoning `PlaylistVoter`/`CouponVoter` use elsewhere in that design).
 *
 * `EVENT_VIEW`'s player branch is BR-02-5/6 in full: a public event grants
 * only if EventEligibilityChecker agrees (age/skill/gender, or — for the
 * trainer's own reads — nothing to check, ownership is enough); a private
 * event grants only if the current player context is invited. A structural
 * 404 (RLS/the Doctrine filter) already stops a cross-tenant id before this
 * voter ever runs, matching AC-02-7's "Access Denied" for a non-invited
 * player on a direct link — this voter's denial is what actually produces
 * that outcome for a same-tenant, non-invited attempt.
 *
 * @see specs/security-voter-designer-design.md "Scheduling module"
 * @see specs/requirements-analyst-epic-02-event-management-spec.md
 */
/**
 * @extends Voter<string, Event|null>
 */
final class EventVoter extends Voter
{
    public const EVENT_CREATE = 'EVENT_CREATE';
    public const EVENT_VIEW = 'EVENT_VIEW';
    public const EVENT_EDIT = 'EVENT_EDIT';
    public const EVENT_DUPLICATE = 'EVENT_DUPLICATE';
    public const EVENT_CANCEL = 'EVENT_CANCEL';
    public const EVENT_VIEW_RSVP_LIST = 'EVENT_VIEW_RSVP_LIST';
    public const EVENT_EXPORT_RSVPS = 'EVENT_EXPORT_RSVPS';
    public const EVENT_MANUAL_ADD_PLAYER = 'EVENT_MANUAL_ADD_PLAYER';

    /**
     * @var list<string>
     */
    private const SUPER_ADMIN_SCOPED_ATTRIBUTES = [self::EVENT_VIEW, self::EVENT_EDIT, self::EVENT_CANCEL, self::EVENT_VIEW_RSVP_LIST];

    public function __construct(
        private readonly RequestStack $requestStack,
        private readonly PlayerContextResolver $playerContext,
        private readonly PlayerTrainerMembershipRepository $memberships,
        private readonly EventEligibilityChecker $eligibility,
        private readonly AdministrativeScope $administrativeScope,
    ) {
    }

    protected function supports(string $attribute, mixed $subject): bool
    {
        if (self::EVENT_CREATE === $attribute) {
            return null === $subject;
        }

        return \in_array($attribute, [
            self::EVENT_VIEW, self::EVENT_EDIT, self::EVENT_DUPLICATE, self::EVENT_CANCEL,
            self::EVENT_VIEW_RSVP_LIST, self::EVENT_EXPORT_RSVPS, self::EVENT_MANUAL_ADD_PLAYER,
        ], true) && $subject instanceof Event;
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        $actor = $token->getUser();

        if (!$actor instanceof Account) {
            return false;
        }

        if (self::EVENT_CREATE === $attribute) {
            return AccountRole::Trainer === $actor->getRole();
        }

        \assert($subject instanceof Event);

        if (self::EVENT_VIEW === $attribute) {
            return $this->voteView($subject, $actor);
        }

        // Every other attribute here is trainer-ownership or
        // Super-Admin-via-AdministrativeScope only — never a coach
        // capability (editing is never a coach capability, per the test
        // matrix's own allow/deny pair for EVENT_EDIT).
        if (AccountRole::Trainer === $actor->getRole()) {
            // Structurally same-tenant already: RLS + the Doctrine filter
            // make a foreign-tenant Event invisible before this voter runs.
            return true;
        }

        if (AccountRole::SuperAdmin === $actor->getRole()
            && \in_array($attribute, self::SUPER_ADMIN_SCOPED_ATTRIBUTES, true)
        ) {
            return $this->administrativeScope->isOpenFor($subject->getTrainer());
        }

        return false;
    }

    private function voteView(Event $event, Account $actor): bool
    {
        if (AccountRole::Trainer === $actor->getRole()) {
            return true;
        }

        if (AccountRole::SuperAdmin === $actor->getRole()) {
            return $this->administrativeScope->isOpenFor($event->getTrainer());
        }

        if (AccountRole::Player !== $actor->getRole()) {
            return false;
        }

        $request = $this->requestStack->getCurrentRequest();

        if (null === $request) {
            return false;
        }

        $player = $this->playerContext->resolve($request, $actor);
        $membership = $this->memberships->findOneByTrainerAndPlayer($event->getTrainer(), $player);

        if (null === $membership || !$membership->isActive()) {
            return false;
        }

        return $this->eligibility->isEligible($event, $membership);
    }
}
