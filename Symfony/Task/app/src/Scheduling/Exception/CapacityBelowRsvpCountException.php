<?php

declare(strict_types=1);

namespace App\Scheduling\Exception;

/**
 * AC-02-53/BR-02-4: "This will exceed capacity. Remove players or increase
 * capacity." — decreasing an event's capacity below its current confirmed
 * RSVP count is blocked until resolved.
 */
final class CapacityBelowRsvpCountException extends \RuntimeException
{
    public static function forEventId(int $eventId, int $requestedCapacity, int $currentCount): self
    {
        return new self(sprintf(
            'This will exceed capacity. Remove players or increase capacity. Event #%d has %d confirmed RSVPs; requested capacity was %d.',
            $eventId,
            $currentCount,
            $requestedCapacity,
        ));
    }
}
