<?php

declare(strict_types=1);

namespace App\Crm\Dto;

/**
 * US-03.08's Quick View dashboard, assembled in one place —
 * `QuickViewDashboardService`'s sole return type. Every figure here is
 * calculated from the database at render time (AC-03-41), never stored.
 *
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md AC-03-34..42, BR-03-16..19
 */
final readonly class QuickViewMetrics
{
    /**
     * @param array<string, int> $rsvpsThisWeekByType
     * @param list<array{playerId: int, name: string, sessionCount: int}> $topPlayers
     * @param array<string, int> $flagCountsByType
     * @param list<array{coachMembershipId: int, name: string, sessionsCount: int, hoursTotal: float}> $coachHours
     */
    public function __construct(
        public int $eventsThisWeek,
        public int $eventsLastWeek,
        public array $rsvpsThisWeekByType,
        public array $topPlayers,
        public array $flagCountsByType,
        public int $shareLinkOpensThisWeek,
        public int $newPlayersThisWeek,
        public array $coachHours,
        public ?float $attendanceRateThisWeek,
        public ?float $attendanceRateLastWeek,
        public ?float $noShowRateThisWeek,
    ) {
    }

    public function eventsDeltaVsLastWeek(): int
    {
        return $this->eventsThisWeek - $this->eventsLastWeek;
    }
}
