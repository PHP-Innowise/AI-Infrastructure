<?php

declare(strict_types=1);

namespace App\Scheduling\Exception;

/**
 * AC-02-28/BR-02-7: "Already registered" — a player cannot RSVP twice to the
 * same event. Translated from the database's own unique-constraint
 * violation (`uniq_rsvp_event_player`) the same way
 * `CoachAlreadyActiveElsewhereException` translates
 * `uniq_coach_membership_active_account` — the constraint is the final
 * backstop under concurrency, this exception is the readable surface over it.
 */
final class AlreadyRegisteredException extends \RuntimeException
{
    public static function forEventId(int $eventId): self
    {
        return new self(sprintf('Already registered for event #%d.', $eventId));
    }
}
