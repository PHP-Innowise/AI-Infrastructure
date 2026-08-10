<?php

declare(strict_types=1);

namespace App\Scheduling\Service;

use App\Identity\Entity\CoachMembership;
use App\Scheduling\Entity\CoachAssignment;
use App\Scheduling\Entity\Event;
use App\Scheduling\Repository\CoachAssignmentRepository;

/**
 * "Produce the coach's reachable-event and reachable-player constraint from
 * assignments" (architect-architecture.md "Cross-cutting services") — the
 * single source of "a coach assigned to zero events sees zero players"
 * (AC-03-64). Every coach-facing read in Scheduling (and, once they exist,
 * Crm/Content) consumes this rather than re-deriving reach; every coach
 * voter delegates to it rather than restating it
 * (architect-architecture.md:112-114).
 *
 * Lives in Scheduling, not Platform or Identity: the source of truth for
 * reach is `CoachAssignment`, a Scheduling entity, and Identity "must not
 * know about events" (architect-architecture.md "Module map"). Crm and
 * Content are both permitted to read Scheduling, matching the build order
 * (01 -> 02 -> {03, 04}).
 *
 * Reach = an assignment that is not Declined (pending or confirmed) — a
 * coach reaches "Events to Confirm" (still pending) as well as "Assigned
 * Sessions" (confirmed); a declined assignment is exactly the "removed from
 * the coach's view" state AC-02-36 describes.
 *
 * @see specs/architect-architecture.md "Coach scoping", "Cross-cutting services"
 * @see specs/security-voter-designer-design.md "Reach: CoachVisibilityService, consumed, never restated"
 */
final readonly class CoachVisibilityService
{
    public function __construct(
        private CoachAssignmentRepository $assignments,
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
}
