<?php

declare(strict_types=1);

namespace App\Scheduling\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Repository\AvailabilityWindowRepository;
use App\Identity\Repository\CoachMembershipRepository;
use App\Identity\Repository\PlayerProfileRepository;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Platform\Entity\Trainer;
use App\Scheduling\Dto\EventInput;
use App\Scheduling\Dto\EventSnapshot;
use App\Scheduling\Entity\CoachAssignment;
use App\Scheduling\Entity\Event;
use App\Scheduling\Entity\EventDuplicationRecord;
use App\Scheduling\Entity\EventInvitation;
use App\Scheduling\Entity\Rsvp;
use App\Scheduling\Exception\CapacityBelowRsvpCountException;
use App\Scheduling\Exception\CoachAssignmentConflictException;
use App\Scheduling\Repository\CoachAssignmentRepository;
use App\Scheduling\Repository\EventDuplicationRecordRepository;
use App\Scheduling\Repository\EventInvitationRepository;
use App\Scheduling\Repository\EventRepository;
use App\Scheduling\Repository\RsvpRepository;
use Doctrine\ORM\EntityManagerInterface;

/**
 * US-02.01/02/03/04/05/13/14: creating, editing, duplicating and canceling
 * events, plus the availability-count widget (AC-02-12).
 *
 * @see specs/requirements-analyst-epic-02-event-management-spec.md
 */
final readonly class EventService
{
    /**
     * AC-02-61: a defensive engineering cap, not a spec-stated number —
     * the epic's own example ("Every Tuesday for 3 months") is ~13
     * occurrences; 104 (two years, weekly) comfortably covers any sane
     * bulk-scheduling use while still refusing a fat-fingered repeatUntil
     * decades out that would otherwise silently create hundreds of rows in
     * one request.
     */
    private const MAX_RECURRING_OCCURRENCES = 104;

    public function __construct(
        private EntityManagerInterface $entityManager,
        private EventRepository $events,
        private RsvpRepository $rsvps,
        private EventInvitationRepository $invitations,
        private EventDuplicationRecordRepository $duplicationRecords,
        private CoachAssignmentRepository $coachAssignments,
        private CoachMembershipRepository $coachMemberships,
        private PlayerProfileRepository $playerProfiles,
        private PlayerTrainerMembershipRepository $memberships,
        private AvailabilityWindowRepository $availabilityWindows,
        private CoachAssignmentService $coachAssignmentService,
        private RsvpService $rsvpService,
        private SchedulingMailer $mailer,
    ) {
    }

    /**
     * AC-02-1..3.
     */
    public function create(Trainer $trainer, Account $actor, EventInput $input): Event
    {
        return $this->entityManager->wrapInTransaction(function () use ($trainer, $actor, $input): Event {
            $event = $this->buildEvent($trainer, $input);
            $this->events->add($event);
            $this->entityManager->flush();

            $this->syncInvitationsIfPrivate($event, $input, $actor);
            $this->assignCoachIfRequested($event, $input, $actor);

            return $event;
        });
    }

    /**
     * AC-02-14..17/BR-02-19: the date/time must differ from the original
     * (AC-02-15); every other field is pre-filled but freely editable by the
     * caller via $input. RSVPs and attendance are never copied — this
     * builds an entirely new Event row.
     */
    public function duplicate(Event $original, Account $actor, EventInput $input): Event
    {
        return $this->entityManager->wrapInTransaction(function () use ($original, $actor, $input): Event {
            // AC-02-15: compared to minute precision in a single common
            // timezone, matching what the datetime-local widget can
            // actually express — $input carries the trainer's own
            // timezone (EventType's model_timezone) while the entity's
            // stored value comes back from Doctrine in whatever offset
            // Postgres reported, and either side may carry seconds a form
            // round-trip can never reproduce. Neither difference may make
            // an untouched field look "changed" and silently defeat this
            // guard.
            $tz = $original->getTrainer()->getTimezone();
            $unchanged = $input->startsAt->setTimezone($tz)->format('Y-m-d H:i') === $original->getStartsAt()->setTimezone($tz)->format('Y-m-d H:i')
                && $input->endsAt->setTimezone($tz)->format('Y-m-d H:i') === $original->getEndsAt()->setTimezone($tz)->format('Y-m-d H:i');

            if ($unchanged) {
                throw new \InvalidArgumentException('The date/time must be changed when duplicating an event.');
            }

            $new = $this->buildEvent($original->getTrainer(), $input);
            $this->events->add($new);
            $this->entityManager->flush();

            $this->syncInvitationsIfPrivate($new, $input, $actor);
            $this->assignCoachIfRequested($new, $input, $actor);

            $this->duplicationRecords->add(new EventDuplicationRecord($original->getTrainer(), $original, $new, $actor));
            $this->entityManager->flush();

            return $new;
        });
    }

    /**
     * AC-02-50..54: any non-past field is editable; capacity may not drop
     * below the current confirmed RSVP count; date/time, location and price
     * changes fan out targeted notifications.
     *
     * @throws CapacityBelowRsvpCountException
     */
    public function update(Event $event, Account $actor, EventInput $input): void
    {
        // Snapshotted BEFORE the transaction below mutates $event in place
        // (EventRepository::lockForUpdate() returns the same managed
        // instance via Doctrine's identity map) — notifyOfChanges() needs
        // the pre-edit values to diff against, not the post-edit ones.
        $before = EventSnapshot::from($event);

        $this->entityManager->wrapInTransaction(function () use ($event, $input): void {
            $now = new \DateTimeImmutable();

            if (!$event->isEditable($now)) {
                throw new \LogicException('This event can no longer be edited — it is canceled or has already started.');
            }

            $lockedEvent = $this->events->lockForUpdate((int) $event->getId())
                ?? throw new \LogicException('Event no longer exists.');

            $confirmedCount = $this->rsvps->countHeld($lockedEvent);

            if (!$lockedEvent->canReduceCapacityTo($input->capacity, $confirmedCount)) {
                throw CapacityBelowRsvpCountException::forEventId((int) $lockedEvent->getId(), $input->capacity, $confirmedCount);
            }

            $lockedEvent->update(
                $input->title,
                $input->eventType,
                $input->startsAt,
                $input->endsAt,
                $input->location,
                $input->capacity,
                $input->visibility,
                $input->description,
                $input->minAge,
                $input->maxAge,
                $input->skillLevels,
                $input->genders,
            );

            $this->entityManager->flush();
        });

        $this->notifyOfChanges($event, $before, $input);
        $this->syncInvitationsIfPrivate($event, $input, $actor);
        $this->assignCoachIfRequested($event, $input, $actor);
    }

    /**
     * AC-02-61: "Every Tuesday for 3 months" — bulk-generates independent
     * weekly event instances from $input's own startsAt/endsAt as the first
     * occurrence, repeating weekly up to and including the calendar date of
     * $repeatUntil. Each generated Event is its own row from buildEvent(),
     * with its own optional private invitations and coach assignment —
     * nothing links them together afterward, matching the epic's own "each
     * generated event is independent — it can have a different coach and
     * can be edited or canceled separately from the others." All-or-nothing:
     * the whole batch shares one transaction, same as create()/duplicate()
     * — a coach conflict on any single occurrence rolls back the entire
     * batch rather than silently creating a partial series, so the trainer
     * always sees one coherent form error instead of untangling which of N
     * events actually got created.
     *
     * Compared at calendar-date granularity (not full timestamp): $input's
     * startsAt/endsAt and $repeatUntil are both collected by forms sharing
     * the trainer's own model_timezone (RecurringEventType), so "through
     * this date" means the occurrence landing ON $repeatUntil's date is
     * still included, regardless of what time-of-day that occurrence falls
     * at.
     *
     * @return list<Event>
     *
     * @throws \InvalidArgumentException        repeatUntil precedes the first occurrence, or the pattern would exceed self::MAX_RECURRING_OCCURRENCES
     * @throws CoachAssignmentConflictException
     */
    public function createRecurring(Trainer $trainer, Account $actor, EventInput $input, \DateTimeImmutable $repeatUntil): array
    {
        if ($repeatUntil->format('Y-m-d') < $input->startsAt->format('Y-m-d')) {
            throw new \InvalidArgumentException('The "repeat until" date must be on or after the first occurrence.');
        }

        return $this->entityManager->wrapInTransaction(function () use ($trainer, $actor, $input, $repeatUntil): array {
            $created = [];
            $occurrence = $input;
            $count = 0;
            $repeatUntilDate = $repeatUntil->format('Y-m-d');

            do {
                if (++$count > self::MAX_RECURRING_OCCURRENCES) {
                    throw new \InvalidArgumentException(sprintf('A recurring pattern cannot generate more than %d occurrences.', self::MAX_RECURRING_OCCURRENCES));
                }

                $event = $this->buildEvent($trainer, $occurrence);
                $this->events->add($event);
                $this->entityManager->flush();

                $this->syncInvitationsIfPrivate($event, $occurrence, $actor);
                $this->assignCoachIfRequested($event, $occurrence, $actor);

                $created[] = $event;

                $occurrence = $occurrence->withDates(
                    $occurrence->startsAt->modify('+1 week'),
                    $occurrence->endsAt->modify('+1 week'),
                );
            } while ($occurrence->startsAt->format('Y-m-d') <= $repeatUntilDate);

            return $created;
        });
    }

    /**
     * AC-02-46..49/BR-02-12: the event's own cancellation commits in one
     * transaction (closing the door on new RSVPs); refunds fan out per RSVP
     * afterward — see RsvpService::cancelForEventCancellation()'s own
     * docblock for why this is deliberately two phases.
     */
    public function cancel(Event $event, string $reason, Account $actor): void
    {
        $affected = $this->entityManager->wrapInTransaction(function () use ($event, $reason, $actor): array {
            $now = new \DateTimeImmutable();
            $lockedEvent = $this->events->lockForUpdate((int) $event->getId())
                ?? throw new \LogicException('Event no longer exists.');

            if (!$lockedEvent->isCancelable($now)) {
                throw new \LogicException('This event can no longer be canceled — it has already started or completed.');
            }

            $lockedEvent->cancel($reason, $actor, $now);

            $rsvps = $this->rsvps->findForEvent($lockedEvent);
            $assignments = $this->coachAssignments->findAllNonDeclinedForEvent($lockedEvent);

            foreach ($assignments as $assignment) {
                $assignment->decline('The event was canceled.');
            }

            $this->entityManager->flush();

            return ['rsvps' => $rsvps, 'assignments' => $assignments];
        });

        /** @var list<CoachAssignment> $assignments */
        $assignments = $affected['assignments'];
        foreach ($assignments as $assignment) {
            $this->mailer->sendEventCanceledToCoach($assignment);
        }

        /** @var list<Rsvp> $rsvps */
        $rsvps = $affected['rsvps'];
        foreach ($rsvps as $rsvp) {
            $this->rsvpService->cancelForEventCancellation($rsvp, $actor);
        }
    }

    /**
     * AC-02-12: "15 of 20 eligible players available at this time" — reuses
     * Epic-01's own AvailabilityWindowRepository (BR-02-20's cross-epic
     * dependency: "the underlying availability data is Epic-01's").
     *
     * @return array{eligible: int, available: int}
     */
    public function availabilityCount(Trainer $trainer, \DateTimeImmutable $at): array
    {
        // Normalized to PHP's own runtime-default timezone, NOT the
        // trainer's — see CoachAssignmentService::guardNoConflict()'s own
        // comment: AvailabilityWindow (Epic-01) carries no timezone of its
        // own ("My Times" is entered and stored as a bare wall-clock value,
        // implicitly PHP's runtime default — UTC in this stack), so
        // comparing it against a trainer-local-converted candidate time
        // would compare two different zones.
        $normalized = $at->setTimezone(new \DateTimeZone(date_default_timezone_get()));
        $dayOfWeek = (int) $normalized->format('w');
        // AvailabilityWindowRepository::findPlayerIdsAvailableAt() compares
        // against a bare TIME column, so the parameter must be a
        // 1970-01-01-based time-of-day value, matching how
        // AvailabilityWindow's own stored start/end times are represented
        // (see AvailabilityTest::testTrainerCanFilterPlayersByAvailabilityAndSeeBestTimes()
        // for the same convention on the Epic-01 side) — never a real
        // calendar date, which would compare a TIME column against a
        // TIMESTAMPTZ value.
        $timeOfDay = new \DateTimeImmutable('1970-01-01 '.$normalized->format('H:i:s'));

        $eligibleIds = array_map(
            static fn ($m): int => (int) $m->getPlayer()->getId(),
            $this->memberships->findActiveForActiveTenant(),
        );
        $availableIds = $this->availabilityWindows->findPlayerIdsAvailableAt($dayOfWeek, $timeOfDay);

        return [
            'eligible' => \count($eligibleIds),
            'available' => \count(array_intersect($eligibleIds, $availableIds)),
        ];
    }

    private function buildEvent(Trainer $trainer, EventInput $input): Event
    {
        $event = new Event(
            $trainer,
            $input->title,
            $input->eventType,
            $input->startsAt,
            $input->endsAt,
            $input->location,
            $input->capacity,
            $input->visibility,
            $input->description,
            $input->minAge,
            $input->maxAge,
            $input->skillLevels,
            $input->genders,
        );
        $event->setUsdPricing($input->usdPricingEnabled, $input->usdPriceMinorUnits);
        $event->setTokenPricing($input->tokenPricingEnabled, $input->tokenPrice);

        return $event;
    }

    /**
     * AC-02-5/BR-02-6: individual player selection only. Replaces the whole
     * guest list each time — simpler than diffing, and this is a small,
     * trainer-curated list, not a high-churn collection.
     */
    private function syncInvitationsIfPrivate(Event $event, EventInput $input, Account $actor): void
    {
        if (!$event->isPrivate()) {
            return;
        }

        foreach ($this->invitations->findForEvent($event) as $existing) {
            $this->entityManager->remove($existing);
        }
        $this->entityManager->flush();

        foreach ($input->invitedPlayerIds as $playerId) {
            $player = $this->playerProfiles->find($playerId);

            if ($player instanceof PlayerProfile) {
                $this->invitations->add(new EventInvitation($event->getTrainer(), $event, $player, $actor));
            }
        }

        $this->entityManager->flush();
    }

    private function assignCoachIfRequested(Event $event, EventInput $input, Account $actor): void
    {
        if (null === $input->coachMembershipId) {
            return;
        }

        $coach = $this->coachMemberships->find($input->coachMembershipId)
            ?? throw new \InvalidArgumentException('Unknown coach.');

        $this->coachAssignmentService->assign($event, $coach, $actor, $input->coachOverrideReason);
    }

    /**
     * AC-02-51: targeted notifications on edit. Diffs against $before (the
     * pre-edit snapshot), never against $event directly — see
     * EventSnapshot's own docblock for why.
     */
    private function notifyOfChanges(Event $event, EventSnapshot $before, EventInput $input): void
    {
        // Compared at minute precision in a common (trainer) timezone, the
        // same technique and for the same reason as duplicate()'s own
        // "must the date/time change" guard: $before->startsAt comes back
        // from Doctrine carrying whatever offset/sub-minute precision
        // Postgres reported, while $input->startsAt is built from the
        // datetime-local widget's own minute-precision, trainer-timezone
        // value — a naive != on two DateTimeImmutable values with
        // differing precision would flag "changed" even when the trainer
        // only touched an unrelated field (e.g. price), spuriously sending
        // an "event details changed" email nobody asked for on top of the
        // genuine "price changed" one. Found via
        // EventEditTest::testPriceIncreaseNotifiesPlayersWithCancelOption().
        $tz = $event->getTrainer()->getTimezone();
        $dateOrLocationChanged = $before->startsAt->setTimezone($tz)->format('Y-m-d H:i') !== $input->startsAt->setTimezone($tz)->format('Y-m-d H:i')
            || $before->endsAt->setTimezone($tz)->format('Y-m-d H:i') !== $input->endsAt->setTimezone($tz)->format('Y-m-d H:i')
            || $before->location !== $input->location;

        $usdIncreased = $input->usdPricingEnabled && $input->usdPriceMinorUnits > $before->usdPriceMinorUnits;
        $tokenIncreased = $input->tokenPricingEnabled && $input->tokenPrice > $before->tokenPrice;
        $priceChanged = $before->usdPricingEnabled !== $input->usdPricingEnabled
            || $before->tokenPricingEnabled !== $input->tokenPricingEnabled
            || $before->usdPriceMinorUnits !== $input->usdPriceMinorUnits
            || $before->tokenPrice !== $input->tokenPrice;

        if (!$dateOrLocationChanged && !$priceChanged) {
            return;
        }

        foreach ($this->rsvps->findForEvent($event) as $rsvp) {
            if ($dateOrLocationChanged) {
                $this->mailer->sendEventDetailsChanged($rsvp, $event);
            }

            if ($priceChanged) {
                $this->mailer->sendPriceChanged($rsvp, $event, $usdIncreased || $tokenIncreased);
            }
        }
    }
}
