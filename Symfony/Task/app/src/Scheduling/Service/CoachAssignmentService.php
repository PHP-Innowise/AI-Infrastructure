<?php

declare(strict_types=1);

namespace App\Scheduling\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\CoachMembership;
use App\Identity\Repository\AvailabilityWindowRepository;
use App\Identity\Service\CoachAvailabilityConflictChecker;
use App\Platform\Service\AuditLogger;
use App\Scheduling\Entity\CoachAssignment;
use App\Scheduling\Entity\CoachAvailabilityOverride;
use App\Scheduling\Entity\Event;
use App\Scheduling\Exception\CoachAssignmentConflictException;
use App\Scheduling\Repository\CoachAssignmentRepository;
use App\Scheduling\Repository\CoachAvailabilityOverrideRepository;
use Doctrine\ORM\EntityManagerInterface;

/**
 * BR-02-13/14/15: assigning a coach to an event, and the coach's own
 * confirm/decline.
 *
 * Q-02.02's resolved default (per the task brief, matching
 * specs/requirements-analyst-open-questions.md, "Always explicit") is
 * always-explicit confirmation — no time-based auto-confirm after 48 hours.
 * AC-02-37 describes that auto-confirm as "an optional trainer preference
 * setting"; this service deliberately does not implement it, and no
 * Symfony Scheduler task exists for it. See the coder's final report for
 * this recorded as a spec-versus-resolved-default conflict, and
 * EpicCompletionCriteriaTest for where AC-02-37 is marked skipped with this
 * same reasoning rather than faked.
 *
 * @see specs/requirements-analyst-epic-02-event-management-spec.md BR-02-13..15, AC-02-8..11, AC-02-34..37
 */
final readonly class CoachAssignmentService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private CoachAssignmentRepository $assignments,
        private CoachAvailabilityOverrideRepository $overrides,
        private AvailabilityWindowRepository $availabilityWindows,
        private CoachAvailabilityConflictChecker $conflictChecker,
        private AuditLogger $auditLogger,
        private SchedulingMailer $mailer,
    ) {
    }

    /**
     * AC-02-8..11: assigns (or re-assigns) a coach. Throws
     * CoachAssignmentConflictException when the proposed time conflicts with
     * the coach's declared availability (BR-02-13) or an already-assigned
     * overlapping event (BR-02-15) and no override reason was supplied; the
     * controller resubmits with $overrideReason once the trainer confirms.
     *
     * @throws CoachAssignmentConflictException
     */
    public function assign(Event $event, CoachMembership $coach, Account $actor, ?string $overrideReason, bool $skipConflictChecks = false): CoachAssignment
    {
        return $this->entityManager->wrapInTransaction(function () use ($event, $coach, $actor, $overrideReason, $skipConflictChecks): CoachAssignment {
            $now = new \DateTimeImmutable();

            if (!$skipConflictChecks) {
                $this->guardNoConflict($event, $coach, $overrideReason);
            }

            $this->supersedeAnyOtherCurrentAssignment($event, $coach, $now);

            $assignment = $this->assignments->findOneByEventAndCoach($event, $coach);

            if (null === $assignment) {
                $assignment = new CoachAssignment($event->getTrainer(), $event, $coach, $now);
                $this->assignments->add($assignment);
            } else {
                $assignment->reassign($now);
            }

            if (null !== $overrideReason && '' !== trim($overrideReason)) {
                $this->overrides->add(new CoachAvailabilityOverride($event->getTrainer(), $event, $coach, $actor, $overrideReason));
                $this->auditLogger->record($actor, 'event.coach_conflict_override', 'Event', $event->getId(), $event->getTrainer(), [
                    'coachMembershipId' => $coach->getId(),
                    'reason' => $overrideReason,
                ]);
            }

            $this->entityManager->flush();
            $this->mailer->sendCoachAssigned($assignment);

            return $assignment;
        });
    }

    /**
     * AC-02-35.
     */
    public function confirm(CoachAssignment $assignment): void
    {
        $assignment->confirm(new \DateTimeImmutable());
        $this->entityManager->flush();
        $this->mailer->sendCoachAssignmentConfirmedToTrainer($assignment);
    }

    /**
     * AC-02-36: "Option B - Decline" — optional reason, trainer notified to
     * find a replacement.
     */
    public function decline(CoachAssignment $assignment, ?string $reason): void
    {
        $assignment->decline($reason);
        $this->entityManager->flush();
        $this->mailer->sendCoachAssignmentDeclinedToTrainer($assignment);
    }

    /**
     * AC-07-27: a read-only pre-check — never throws, never runs inside
     * `wrapInTransaction()` — so a caller can decide what to do about a
     * conflict BEFORE calling `assign()`, rather than attempting-and-
     * catching around it. This distinction is load-bearing, not
     * stylistic: Doctrine's own `EntityManager::wrapInTransaction()` closes
     * the EntityManager on ANY exception escaping its callback (its own
     * implementation — `close()` before `rollback()`), which makes a
     * same-request retry after catching `CoachAssignmentConflictException`
     * from a first `assign()` attempt permanently break every later
     * Doctrine call in that request. `EventMasterController`'s Super Admin
     * override path calls this method first for exactly that reason — see
     * `EventService::assignCoachIfRequested()`'s own docblock.
     */
    public function detectConflict(Event $event, CoachMembership $coach): ?CoachAssignmentConflictException
    {
        // Explicitly normalized to PHP's own runtime-default timezone (NOT
        // the trainer's) before deriving day-of-week/time-of-day:
        // AvailabilityWindow (Epic-01) carries no timezone of its own —
        // "My Times" is entered and stored as a bare wall-clock value with
        // no conversion anywhere in that form (DayAvailabilityType has no
        // model_timezone/view_timezone option), so it is implicitly in
        // PHP's runtime default (UTC in this stack — see AvailabilityTest's
        // own bare '1970-01-01 HH:MM:SS' literals). $event->getStartsAt()
        // itself is not reliable for this without normalizing first: it
        // carries the trainer's own timezone when freshly built from
        // EventType's form data (create) but UTC once reloaded from
        // Doctrine (edit) — two different callers of this same method would
        // otherwise compare against two different zones. Explicit
        // normalization here mirrors the same limitation Epic-01's own
        // availability feature already has (never trainer-aware) rather
        // than introducing a new inconsistency — recorded in the coder's
        // final report, not silently fixed by editing Epic-01's own form.
        $utc = new \DateTimeZone(date_default_timezone_get());
        $normalizedStart = $event->getStartsAt()->setTimezone($utc);
        $normalizedEnd = $event->getEndsAt()->setTimezone($utc);
        $dayOfWeek = (int) $normalizedStart->format('w');
        $timeStart = new \DateTimeImmutable('1970-01-01 '.$normalizedStart->format('H:i:s'));
        $timeEnd = new \DateTimeImmutable('1970-01-01 '.$normalizedEnd->format('H:i:s'));

        $declared = $this->availabilityWindows->findForCoach($coach);

        if ($this->conflictChecker->conflictsWith($declared, $dayOfWeek, $timeStart, $timeEnd)) {
            return CoachAssignmentConflictException::availabilityConflict($this->coachDisplayName($coach));
        }

        $overlapping = $this->assignments->findOverlappingForCoach($coach, $event->getStartsAt(), $event->getEndsAt(), $event);

        if ([] !== $overlapping) {
            return CoachAssignmentConflictException::overlappingAssignment($this->coachDisplayName($coach));
        }

        return null;
    }

    /**
     * @throws CoachAssignmentConflictException
     */
    private function guardNoConflict(Event $event, CoachMembership $coach, ?string $overrideReason): void
    {
        $hasOverride = null !== $overrideReason && '' !== trim($overrideReason);
        $conflict = $this->detectConflict($event, $coach);

        if (null !== $conflict && !$hasOverride) {
            throw $conflict;
        }
    }

    /**
     * See CoachAssignment's own docblock: reassigning to a different coach
     * declines whichever row currently holds the event's operative
     * assignment, with a system-authored reason distinguishable from a
     * genuine coach decline.
     */
    private function supersedeAnyOtherCurrentAssignment(Event $event, CoachMembership $newCoach, \DateTimeImmutable $now): void
    {
        $current = $this->assignments->findCurrentForEvent($event);

        if (null === $current || $current->getCoachMembership() === $newCoach) {
            return;
        }

        $current->decline(CoachAssignment::REASON_REASSIGNED);
        $this->mailer->sendCoachReassignmentNotice($current, false);
    }

    private function coachDisplayName(CoachMembership $coach): string
    {
        $account = $coach->getAccount();

        return $account->getProfile()?->getFullName() ?? $account->getEmail();
    }
}
