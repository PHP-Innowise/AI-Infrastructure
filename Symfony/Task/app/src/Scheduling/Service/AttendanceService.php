<?php

declare(strict_types=1);

namespace App\Scheduling\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\CoachMembership;
use App\Identity\Repository\PlayerProfileRepository;
use App\Scheduling\Entity\AttendanceEdit;
use App\Scheduling\Entity\AttendanceRecord;
use App\Scheduling\Entity\Event;
use App\Scheduling\Entity\Rsvp;
use App\Scheduling\Repository\AttendanceEditRepository;
use App\Scheduling\Repository\AttendanceRecordRepository;
use App\Scheduling\Repository\CoachAssignmentRepository;
use App\Scheduling\Repository\RsvpRepository;
use Doctrine\ORM\EntityManagerInterface;

/**
 * US-02.11: recording and editing attendance.
 *
 * Who may call this at all, and when (same-day-only for a coach, any time
 * for a trainer/Super Admin — BR-02-18), is entirely AttendanceVoter's job,
 * checked by the controller before this service ever runs — this class only
 * owns the data operation and the one structural precondition BR-02-16
 * states outright (the event must have started, and must have an assigned
 * coach to attribute the record to).
 *
 * @see specs/requirements-analyst-epic-02-event-management-spec.md BR-02-16..18, AC-02-38..42
 */
final readonly class AttendanceService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private AttendanceRecordRepository $records,
        private AttendanceEditRepository $edits,
        private RsvpRepository $rsvps,
        private CoachAssignmentRepository $assignments,
        private PlayerProfileRepository $playerProfiles,
    ) {
    }

    /**
     * AC-02-38/39/41/42: one save call for the whole roster.
     *
     * @param array<int, string> $statusByPlayerId player id => AttendanceRecord::STATUS_*
     */
    public function recordAttendance(Event $event, Account $actor, array $statusByPlayerId): void
    {
        $this->entityManager->wrapInTransaction(function () use ($event, $actor, $statusByPlayerId): void {
            $now = new \DateTimeImmutable();

            if (!$event->hasStarted($now)) {
                throw new \LogicException('Attendance cannot be marked for an event that has not started yet.');
            }

            $assignment = $this->assignments->findCurrentForEvent($event)
                ?? throw new \LogicException('This event has no assigned coach; attendance cannot be recorded.');

            foreach ($statusByPlayerId as $playerId => $status) {
                $player = $this->playerProfiles->find($playerId);

                if (null === $player) {
                    continue;
                }

                $rsvp = $this->rsvps->findOneByEventAndPlayer($event, $player);

                // AC-02-42: a player who canceled their RSVP is not shown in
                // (or written to) the attendance list.
                if (null === $rsvp || Rsvp::STATUS_CONFIRMED !== $rsvp->getStatus()) {
                    continue;
                }

                $this->recordOne($event, $rsvp, $status, $assignment->getCoachMembership(), $actor, $now);
            }

            $this->entityManager->flush();
        });
    }

    private function recordOne(
        Event $event,
        Rsvp $rsvp,
        string $status,
        CoachMembership $recordedByCoach,
        Account $editor,
        \DateTimeImmutable $now,
    ): void {
        $existing = $this->records->findOneByEventAndPlayer($event, $rsvp->getPlayer());

        if (null === $existing) {
            $this->records->add(new AttendanceRecord($event->getTrainer(), $event, $rsvp->getPlayer(), $rsvp, $status, $recordedByCoach, $now));

            return;
        }

        if ($existing->getStatus() === $status) {
            return;
        }

        // BR-02-18: every change past the first save is logged (who, when,
        // old value, new value).
        $this->edits->add(new AttendanceEdit($event->getTrainer(), $existing, $existing->getStatus(), $status, $editor, $now));
        $existing->changeStatus($status);
    }
}
