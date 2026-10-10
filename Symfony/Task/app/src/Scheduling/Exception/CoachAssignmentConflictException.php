<?php

declare(strict_types=1);

namespace App\Scheduling\Exception;

/**
 * AC-02-9/BR-02-13: the coach's declared availability conflicts with the
 * proposed event time. BR-02-15: the coach is already assigned to an
 * overlapping event. Either way: "the system shows a warning... the trainer
 * can override by entering a required reason" — this exception IS that
 * warning, translated by the controller into a form error the trainer can
 * resolve by supplying an override reason and resubmitting.
 */
final class CoachAssignmentConflictException extends \RuntimeException
{
    public static function availabilityConflict(string $coachName): self
    {
        return new self(sprintf('Coach %s is not available at this time. Continue anyway?', $coachName));
    }

    public static function overlappingAssignment(string $coachName): self
    {
        return new self(sprintf('Coach %s is already assigned to another event at this time. Continue anyway?', $coachName));
    }
}
