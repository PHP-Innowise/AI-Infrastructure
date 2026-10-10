<?php

declare(strict_types=1);

namespace App\Scheduling\Service;

use App\Identity\Entity\CoachMembership;
use App\Identity\Entity\PlayerProfile;
use App\Scheduling\Entity\CoachAssignment;
use App\Scheduling\Entity\Event;
use App\Scheduling\Repository\CoachAssignmentRepository;
use App\Scheduling\Repository\RsvpRepository;

/**
 * "Produce the coach's reachable-event and reachable-player constraint from
 * assignments" (architect-architecture.md "Cross-cutting services") — the
 * single source of "a coach assigned to zero events sees zero players"
 * (AC-03-64). Every coach-facing read in Scheduling and Crm consumes this
 * rather than re-deriving reach; every coach voter delegates to it rather
 * than restating it (architect-architecture.md:112-114).
 *
 * Lives in Scheduling, not Platform, Identity, or Crm: the source of truth
 * for reach is `CoachAssignment`, a Scheduling entity, and Identity "must not
 * know about events" (architect-architecture.md "Module map"). Crm is
 * permitted to read Scheduling, matching the build order (01 -> 02 -> 03).
 *
 * Reach = an assignment that is not Declined (pending or confirmed) — a
 * coach reaches "Events to Confirm" (still pending) as well as "Assigned
 * Sessions" (confirmed); a declined assignment is exactly the "removed from
 * the coach's view" state AC-02-36 describes.
 *
 * `reachablePlayerIds()`/`canReachPlayer()` (Epic-03) reduce reachable events
 * to reachable players via `RsvpRepository::confirmedPlayerIdsForEvents()` —
 * a player is reachable exactly when they hold a confirmed RSVP to an event
 * this coach reaches, matching AC-03-43's "sees only players who have
 * RSVP'd to events where the coach is assigned". Never re-derived from
 * `AttendanceRecord` directly, and never restated by `PlayerVoter` — the
 * same "single source of coach reach" invariant `canReachEvent()`/
 * `reachableEventIds()` already established for Scheduling now covers Crm.
 *
 * @see specs/architect-architecture.md "Coach scoping", "Cross-cutting services"
 * @see specs/security-voter-designer-design.md "Reach: CoachVisibilityService, consumed, never restated"
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md AC-03-43/44/64/65
 */
final readonly class CoachVisibilityService
{
    public function __construct(
        private CoachAssignmentRepository $assignments,
        private RsvpRepository $rsvps,
    ) {
    }

    public function canReachEvent(CoachMembership $coach, Event $event): bool
    {
        $assignment = $this->assignments->findOneByEventAndCoach($event, $coach);

        return null !== $assignment && !$assignment->isDeclined();
    }

    /**
     * @return list<int> event ids
     */
    public function reachableEventIds(CoachMembership $coach): array
    {
        return array_map(
            static fn (CoachAssignment $assignment): int => (int) $assignment->getEvent()->getId(),
            [...$this->assignments->findPendingForCoach($coach), ...$this->assignments->findConfirmedForCoach($coach)],
        );
    }

    /**
     * AC-03-43/64: the trainer's full roster is never queried here — only
     * whichever players hold a confirmed RSVP against this coach's own
     * reachable events.
     *
     * @return list<int> player ids
     */
    public function reachablePlayerIds(CoachMembership $coach): array
    {
        return $this->rsvps->confirmedPlayerIdsForEvents($this->reachableEventIds($coach));
    }

    /**
     * AC-03-44's note/architect-architecture.md Open architecture risk 6: a
     * coach viewing a player with no shared session history is unreachable,
     * not an empty profile — this is the single predicate that denial rests
     * on, consumed by `PlayerVoter` rather than restated there.
     */
    public function canReachPlayer(CoachMembership $coach, PlayerProfile $player): bool
    {
        return \in_array((int) $player->getId(), $this->reachablePlayerIds($coach), true);
    }
}
