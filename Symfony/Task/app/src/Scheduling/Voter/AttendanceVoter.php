<?php

declare(strict_types=1);

namespace App\Scheduling\Voter;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Identity\Repository\CoachMembershipRepository;
use App\Platform\Tenancy\AdministrativeScope;
use App\Scheduling\Entity\Event;
use App\Scheduling\Service\CoachVisibilityService;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * BR-02-16..18, AC-02-38..42.
 *
 * Trainer branch: any time, overrides the coach's entries (BR-02-18). Coach
 * branch: the event must be inside CoachVisibilityService's reachable set
 * for the acting coach (never re-derived from Rsvp/AttendanceRecord
 * directly — "a voter restating a visibility rule is a defect"), the event
 * must have started (BR-02-16), and it must still be the same calendar day
 * as the event's own start, in the trainer's own timezone — after that the
 * coach's view is read-only (BR-02-18) though this attribute is write-only,
 * so a coach outside the window is denied outright rather than granted
 * read-only.
 *
 * Super Admin: specs/security-voter-designer-design.md leaves this
 * mechanism explicitly unresolved upstream ("no AdministrativeScope clause
 * is named... treated the same as the CRM Master gap" — Open questions #1).
 * This design resolves it by the closest settled analogy in the same
 * document — EventVoter::EVENT_EDIT's `+ SUPER_ADMIN via
 * AdministrativeScope` branch — rather than leaving Super Admin unable to
 * exercise BR-02-18's own stated "at any time" grant. Recorded as an
 * extrapolation, not a verbatim rule, in the coder's final report.
 *
 * @see specs/security-voter-designer-design.md "Scheduling module", Open questions #1
 */
/**
 * @extends Voter<string, Event>
 */
final class AttendanceVoter extends Voter
{
    public const ATTENDANCE_RECORD = 'ATTENDANCE_RECORD';

    public function __construct(
        private readonly CoachMembershipRepository $coachMemberships,
        private readonly CoachVisibilityService $coachVisibility,
        private readonly AdministrativeScope $administrativeScope,
    ) {
    }

    protected function supports(string $attribute, mixed $subject): bool
    {
        return self::ATTENDANCE_RECORD === $attribute && $subject instanceof Event;
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        $actor = $token->getUser();

        if (!$actor instanceof Account) {
            return false;
        }

        if (AccountRole::Trainer === $actor->getRole()) {
            return true;
        }

        if (AccountRole::SuperAdmin === $actor->getRole()) {
            return $this->administrativeScope->isOpenFor($subject->getTrainer());
        }

        if (AccountRole::Coach !== $actor->getRole()) {
            return false;
        }

        $membership = $this->coachMemberships->findOneForAccountInActiveTenant($actor);

        if (null === $membership || !$this->coachVisibility->canReachEvent($membership, $subject)) {
            return false;
        }

        return $this->isSameDayAsEventStart($subject, new \DateTimeImmutable());
    }

    /**
     * BR-02-16: only after the event has started. BR-02-18: only through
     * midnight of that same calendar day, in the trainer's own timezone.
     */
    private function isSameDayAsEventStart(Event $event, \DateTimeImmutable $now): bool
    {
        if (!$event->hasStarted($now)) {
            return false;
        }

        $tz = $event->getTrainer()->getTimezone();
        $eventDay = $event->getStartsAt()->setTimezone($tz)->format('Y-m-d');
        $today = $now->setTimezone($tz)->format('Y-m-d');

        return $eventDay === $today;
    }
}
