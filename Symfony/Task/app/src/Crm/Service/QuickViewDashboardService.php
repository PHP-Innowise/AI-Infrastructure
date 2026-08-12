<?php

declare(strict_types=1);

namespace App\Crm\Service;

use App\Crm\Dto\QuickViewMetrics;
use App\Crm\Repository\PlayerFlagRepository;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Identity\Repository\ShareLinkOpenRepository;
use App\Platform\Entity\Trainer;
use App\Scheduling\Repository\AttendanceRecordRepository;
use App\Scheduling\Repository\CoachAssignmentRepository;
use App\Scheduling\Repository\EventRepository;
use App\Scheduling\Repository\RsvpRepository;

/**
 * Composes the Quick View dashboard (US-03.08) from every module's own
 * repository — the one place in Crm that legitimately orchestrates several
 * reads into a single screen, rather than a plain pass-through a controller
 * could do alone.
 *
 * **Weeks are Monday-Sunday, in the trainer's own timezone** — the same
 * timezone convention `TrainerEventController`/`AppFixtures` already use for
 * event-time math, and the reason `Trainer::getTimezone()` exists at all
 * (specs/database-designer-schema.md "`trainer`": "required by... BR-05-14").
 * Neither US-03.08 nor the epic-level AC states a week boundary explicitly;
 * Monday-Sunday is this service's own inference, recorded in the coder's
 * final report.
 *
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md AC-03-34..42, BR-03-16..19
 */
final readonly class QuickViewDashboardService
{
    public function __construct(
        private EventRepository $events,
        private RsvpRepository $rsvps,
        private AttendanceRecordRepository $attendanceRecords,
        private CoachAssignmentRepository $coachAssignments,
        private PlayerFlagRepository $flags,
        private ShareLinkOpenRepository $shareLinkOpens,
        private PlayerTrainerMembershipRepository $playerMemberships,
    ) {
    }

    public function build(Trainer $trainer, \DateTimeImmutable $now): QuickViewMetrics
    {
        $tz = $trainer->getTimezone();
        $local = $now->setTimezone($tz);
        // ISO-8601 'N' = 1 (Monday) .. 7 (Sunday).
        $thisWeekStart = $local->modify(sprintf('-%d days', $local->format('N') - 1))->setTime(0, 0);
        $thisWeekEnd = $thisWeekStart->modify('+7 days');
        $lastWeekStart = $thisWeekStart->modify('-7 days');
        $lastWeekEnd = $thisWeekStart;

        $trainerId = (int) $trainer->getId();

        $thisWeekTally = $this->attendanceRecords->attendanceTallyBetween($trainerId, $thisWeekStart, $thisWeekEnd);
        $lastWeekTally = $this->attendanceRecords->attendanceTallyBetween($trainerId, $lastWeekStart, $lastWeekEnd);

        return new QuickViewMetrics(
            eventsThisWeek: $this->events->countStartingBetween($thisWeekStart, $thisWeekEnd),
            eventsLastWeek: $this->events->countStartingBetween($lastWeekStart, $lastWeekEnd),
            rsvpsThisWeekByType: $this->rsvps->countByEventTypeRequestedBetween($thisWeekStart, $thisWeekEnd),
            // BR-03-16/17: fixed 90-day window, independent of the week
            // boundaries above.
            topPlayers: $this->attendanceRecords->topPlayers($trainerId, 10, $now),
            flagCountsByType: $this->flags->countActiveByTypeForActiveTenant(),
            shareLinkOpensThisWeek: $this->shareLinkOpens->countForActiveTenantBetween($thisWeekStart, $thisWeekEnd),
            newPlayersThisWeek: $this->playerMemberships->countNewViaShareLinkBetween($thisWeekStart, $thisWeekEnd),
            coachHours: $this->coachAssignments->hoursSummaryForActiveTenant(),
            attendanceRateThisWeek: $this->rate($thisWeekTally),
            attendanceRateLastWeek: $this->rate($lastWeekTally),
            noShowRateThisWeek: $this->noShowRate($thisWeekTally),
        );
    }

    /**
     * @param array{attended: int, absent: int, total: int} $tally
     */
    private function rate(array $tally): ?float
    {
        return 0 === $tally['total'] ? null : round(100 * $tally['attended'] / $tally['total'], 1);
    }

    /**
     * @param array{attended: int, absent: int, total: int} $tally
     */
    private function noShowRate(array $tally): ?float
    {
        return 0 === $tally['total'] ? null : round(100 * $tally['absent'] / $tally['total'], 1);
    }
}
