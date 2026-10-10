<?php

declare(strict_types=1);

namespace App\Identity\Service;

use App\Identity\Entity\AvailabilityWindow;

/**
 * AC-01-47/BR-01-26: "when a trainer assigns a coach to an event at a time
 * conflicting with the coach's stated availability, the system warns the
 * trainer... and requires a text reason to override."
 *
 * This class is deliberately only the pure conflict *detection* — "is this
 * proposed slot outside what the coach declared available." The trigger
 * (assigning a coach to an `Event`) and the override log
 * (`coach_availability_override`, which references `event`) both belong to
 * Epic-02's own `CoachAssignment` workflow: `event` and `coach_assignment` do
 * not exist yet, and `coach_availability_override` is itself scheduled for
 * Epic-02's migration batch (M2), not Epic-01's (M1), per
 * specs/database-designer-schema.md "Migration ordering". Epic-02's
 * assignment service is the intended caller of `conflictsWith()` once it
 * exists — recorded as a scope boundary in the coder's final report, not
 * silently invented here.
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md AC-01-47, BR-01-26
 */
final class CoachAvailabilityConflictChecker
{
    /**
     * True if the proposed (day, start, end) does NOT fall entirely inside a
     * declared *available* window — i.e. the trainer should see a warning.
     * An empty availability set (the coach never set "My Times") also
     * conflicts: there is nothing on record that covers the proposed slot.
     *
     * @param list<AvailabilityWindow> $declaredAvailability
     */
    public function conflictsWith(array $declaredAvailability, int $dayOfWeek, \DateTimeImmutable $start, \DateTimeImmutable $end): bool
    {
        foreach ($declaredAvailability as $window) {
            if (!$window->isAvailable()) {
                continue;
            }

            if ($window->getDayOfWeek() === $dayOfWeek
                && $window->getStartTime() <= $start
                && $window->getEndTime() >= $end
            ) {
                return false;
            }
        }

        return true;
    }
}
