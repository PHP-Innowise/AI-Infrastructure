<?php

declare(strict_types=1);

namespace App\Scheduling\Exception;

/**
 * AC-02-27: "Event Full - No spots available" (free) / "Event Full" (paid).
 * Thrown from inside RsvpService's locked, authoritative capacity check
 * (AC-02-67) — the voter's own unlocked pre-check is fail-fast UX only, per
 * architect-architecture.md "Lock ordering"; this is the decision that
 * actually holds under concurrency.
 */
final class EventFullException extends \RuntimeException
{
    public static function forEventId(int $eventId): self
    {
        return new self(sprintf('Event #%d is full. No spots available.', $eventId));
    }
}
