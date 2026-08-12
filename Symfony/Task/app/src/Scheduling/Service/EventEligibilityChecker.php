<?php

declare(strict_types=1);

namespace App\Scheduling\Service;

use App\Identity\Entity\Gender;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Identity\Entity\SkillLevel;
use App\Scheduling\Entity\Event;
use App\Scheduling\Repository\EventInvitationRepository;

/**
 * BR-02-5/6: whether a player may see/RSVP to an event — age, skill level,
 * gender restrictions for a public event; invitation membership for a
 * private one. One place for this so the Training Calendar listing, the RSVP
 * voter and the RSVP-list availability column never restate it separately
 * (architect-architecture.md "A voter restating a visibility rule is a
 * defect", generalized here to any repeated eligibility check).
 *
 * A restricted axis (skill level or gender) the player has not recorded a
 * value for is treated as NOT matching — a documented, conservative choice:
 * the epic states restrictions are checked against the player's own
 * attributes but never says what an unset attribute should do against a
 * restricted event, so this errs toward not showing a mismatch as a false
 * positive.
 *
 * @see specs/requirements-analyst-epic-02-event-management-spec.md BR-02-5, BR-02-6, AC-02-19, AC-02-28
 */
final readonly class EventEligibilityChecker
{
    public function __construct(
        private EventInvitationRepository $invitations,
    ) {
    }

    /**
     * BR-02-6: private events are shown only to invited players, "even if a
     * non-invited player would otherwise be eligible by age/skill/gender" —
     * so a private, non-invited event fails regardless of the age/skill/
     * gender checks below.
     */
    public function isEligible(Event $event, PlayerTrainerMembership $membership): bool
    {
        if ($event->isPrivate()) {
            return $this->invitations->isPlayerInvited($event, $membership->getPlayer());
        }

        return $this->matchesAge($event, $membership)
            && $this->matchesSkillLevel($event, $membership)
            && $this->matchesGender($event, $membership);
    }

    private function matchesAge(Event $event, PlayerTrainerMembership $membership): bool
    {
        if (null === $event->getMinAge() && null === $event->getMaxAge()) {
            return true;
        }

        $age = $membership->getPlayer()->ageOn($event->getStartsAt());

        if (null !== $event->getMinAge() && $age < $event->getMinAge()) {
            return false;
        }

        if (null !== $event->getMaxAge() && $age > $event->getMaxAge()) {
            return false;
        }

        return true;
    }

    /**
     * Compared through `SkillLevel::matches()`, not with `in_array(...,
     * true)`. Both sides are chosen from one list now, but rows written
     * before that was true still hold whatever a trainer typed, and a player
     * quietly missing from their own calendar because of one capital letter
     * is the failure this method exists to avoid.
     */
    private function matchesSkillLevel(Event $event, PlayerTrainerMembership $membership): bool
    {
        $restriction = $event->getSkillLevels();

        if (null === $restriction || [] === $restriction) {
            return true;
        }

        $playerSkill = $membership->getSkillLevel();

        foreach ($restriction as $allowed) {
            if (SkillLevel::matches($playerSkill, $allowed)) {
                return true;
            }
        }

        return false;
    }

    /**
     * Through `Gender::matches()`, for the reason `matchesSkillLevel()` above
     * uses `SkillLevel::matches()`: this restriction was free text compared
     * strictly against a value the player picks from a list, so an event
     * restricted to "Female" excluded every player, whose profile says
     * `female`.
     */
    private function matchesGender(Event $event, PlayerTrainerMembership $membership): bool
    {
        $restriction = $event->getGenders();

        if (null === $restriction || [] === $restriction) {
            return true;
        }

        $playerGender = $membership->getPlayer()->getGender();

        foreach ($restriction as $allowed) {
            if (Gender::matches($playerGender, $allowed)) {
                return true;
            }
        }

        return false;
    }
}
