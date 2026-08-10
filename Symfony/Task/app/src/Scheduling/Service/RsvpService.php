<?php

declare(strict_types=1);

namespace App\Scheduling\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\ChildApprovalRequest;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Service\ChildActionAttempt;
use App\Identity\Service\ChildApprovalService;
use App\Identity\Service\FundingMethod;
use App\Identity\Voter\ChildApprovalVoter;
use App\Platform\Service\AuditLogger;
use App\Scheduling\Billing\PaymentIntentGateway;
use App\Scheduling\Billing\PaymentIntentOutcome;
use App\Scheduling\Billing\PaymentIntentRequest;
use App\Scheduling\Entity\Event;
use App\Scheduling\Entity\Rsvp;
use App\Scheduling\Exception\AlreadyRegisteredException;
use App\Scheduling\Exception\EventFullException;
use App\Scheduling\Repository\EventRepository;
use App\Scheduling\Repository\RsvpRepository;
use Doctrine\DBAL\Exception\UniqueConstraintViolationException;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\SecurityBundle\Security;

/**
 * US-02.07/08: registering for and canceling out of an event.
 *
 * Lock ordering (architect-architecture.md "Lock ordering"): the fixed
 * global order is the token balance row first, then the event row. Billing
 * does not exist yet, so there is no balance row to lock — but the CODE
 * SHAPE here still resolves funding/child-approval (where a future balance
 * lock would happen, inside the payment-intent gateway call) BEFORE ever
 * locking the event row, so the ordering is already correct the moment
 * Epic-05 wires a real gateway behind PaymentIntentGateway. The event lock
 * is always the LAST lock taken in this sequence.
 *
 * AC-02-67's unlocked-vs-locked split: RsvpVoter::RSVP_CREATE does a fast,
 * non-authoritative capacity pre-check for UX; the count taken here, after
 * EventRepository::lockForUpdate(), is the one that actually decides —
 * "An unlocked capacity pre-check gives fail-fast UX... the locked check
 * inside the transaction is the one that decides" (architect-architecture.md).
 *
 * @see specs/requirements-analyst-epic-02-event-management-spec.md BR-02-7..11, AC-02-23..33
 */
final readonly class RsvpService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private EventRepository $events,
        private RsvpRepository $rsvps,
        private Security $security,
        private ChildApprovalService $childApprovalService,
        private PaymentIntentGateway $paymentGateway,
        private PlayerAccountResolver $playerAccounts,
        private SchedulingMailer $mailer,
        private AuditLogger $auditLogger,
    ) {
    }

    /**
     * @throws AlreadyRegisteredException
     * @throws EventFullException
     */
    public function rsvp(Event $event, PlayerProfile $player, Account $actor, string $paymentMethod): Rsvp
    {
        if (null !== $this->rsvps->findActiveOneByEventAndPlayer($event, $player)) {
            throw AlreadyRegisteredException::forEventId((int) $event->getId());
        }

        $fundingMethod = $this->fundingMethodForPaymentMethod($paymentMethod);

        // BR-02-10/AC-02-23: resolved here, before the event lock — see the
        // class docblock on lock ordering.
        $bypassGranted = $this->security->isGranted(
            ChildApprovalVoter::CHILD_APPROVAL_BYPASS,
            new ChildActionAttempt($actor, $player, $fundingMethod),
        );

        $rsvp = $this->entityManager->wrapInTransaction(function () use ($event, $player, $paymentMethod, $bypassGranted): Rsvp {
            $now = new \DateTimeImmutable();
            $lockedEvent = $this->events->lockForUpdate((int) $event->getId())
                ?? throw new \LogicException('Event no longer exists.');

            if (Event::STATUS_ACTIVE !== $lockedEvent->getStatus() || $lockedEvent->hasStarted($now)) {
                throw new \LogicException('Cannot RSVP to a past or canceled event.');
            }

            // AC-02-67: authoritative, locked capacity check.
            if ($this->rsvps->countHeld($lockedEvent) >= $lockedEvent->getCapacity()) {
                throw EventFullException::forEventId((int) $lockedEvent->getId());
            }

            // BR-02-7's unique constraint has no status qualifier (see
            // Rsvp::reactivate()'s own docblock): a previously-canceled row
            // for this exact (event, player) pair is reused, never
            // re-inserted — the DB would reject a second row outright.
            $rsvp = $this->rsvps->findOneByEventAndPlayer($lockedEvent, $player);

            if (null !== $rsvp) {
                $rsvp->reactivate($paymentMethod, $now);
            } else {
                $rsvp = new Rsvp($lockedEvent->getTrainer(), $lockedEvent, $player, $paymentMethod, $now);
                $this->rsvps->add($rsvp);
            }

            if (!$bypassGranted) {
                // BR-02-10: pending parent approval regardless of price.
                $rsvp->markPendingParentApproval();
            }

            try {
                $this->entityManager->flush();
            } catch (UniqueConstraintViolationException) {
                throw AlreadyRegisteredException::forEventId((int) $lockedEvent->getId());
            }

            return $rsvp;
        });

        if (!$bypassGranted) {
            $this->childApprovalService->requestApproval(
                $rsvp->getTrainer(),
                $player,
                ChildApprovalRequest::ACTION_RSVP,
                (int) $rsvp->getId(),
            );

            return $rsvp;
        }

        $this->completeConfirmationOrPayment($rsvp, $actor);

        return $rsvp;
    }

    /**
     * AC-02-29/31/33, BR-02-11: player/parent-initiated cancellation.
     * Blocked after the event has started (AC-02-30). AC-02-33: the same
     * parent-approval gate as a purchase (BR-02-10) — a child's own attempt
     * that is not bypassed waits for a parent's decision instead of
     * canceling immediately; see completeCancellationAfterApproval() for
     * where an approved request actually applies the cancellation, and
     * cancelAfterParentDenial() for a denied one. Refund eligibility (once
     * the cancellation does apply) follows the 24-hour policy, distinct
     * from a trainer-initiated cancellation's unconditional refund
     * (self::cancelForEventCancellation()).
     */
    public function cancel(Rsvp $rsvp, Account $actor, ?string $reason): void
    {
        if ($rsvp->getEvent()->hasStarted(new \DateTimeImmutable())) {
            throw new \LogicException('Cannot cancel an RSVP after the event has started.');
        }

        $bypassGranted = $this->security->isGranted(
            ChildApprovalVoter::CHILD_APPROVAL_BYPASS,
            new ChildActionAttempt($actor, $rsvp->getPlayer(), $this->fundingMethodForPaymentMethod($rsvp->getPaymentMethod())),
        );

        if (!$bypassGranted) {
            $this->childApprovalService->requestApproval(
                $rsvp->getTrainer(),
                $rsvp->getPlayer(),
                ChildApprovalRequest::ACTION_RSVP_CANCELLATION,
                (int) $rsvp->getId(),
            );

            return;
        }

        $wasPaidAndConfirmed = $rsvp->isPaid() && $rsvp->isConfirmed();
        $eligibleForRefund = $wasPaidAndConfirmed && $this->isAtLeast24HoursOut($rsvp->getEvent());

        $this->markCanceledAndNotify($rsvp, $reason);

        if ($eligibleForRefund) {
            $this->attemptRefund($rsvp, $actor);
        }
    }

    /**
     * AC-02-23..26: the parent approved a pending ACTION_RSVP request —
     * called by the approval-decision controller right after
     * ChildApprovalService::approve() commits that decision. Proceeds
     * exactly where rsvp() would have gone had the bypass been granted
     * immediately.
     */
    public function completeAfterParentApproval(Rsvp $rsvp, Account $payer): void
    {
        $this->entityManager->wrapInTransaction(function () use ($rsvp): void {
            $rsvp->approveAfterParentApproval(new \DateTimeImmutable());
            $this->entityManager->flush();
        });

        $this->completeConfirmationOrPayment($rsvp, $payer);
    }

    /**
     * AC-02-23..26: the parent denied a pending ACTION_RSVP request — the
     * held spot is released, same as any other canceled RSVP (AC-02-67).
     * Nothing was ever paid or confirmed at this point (the RSVP has been
     * sitting at Pending Parent Approval since construction), so no refund
     * step applies.
     */
    public function cancelAfterParentDenial(Rsvp $rsvp): void
    {
        if ($rsvp->isCanceled()) {
            return;
        }

        $this->markCanceledAndNotify($rsvp, 'The parent denied this request.');
    }

    /**
     * AC-02-33: the parent approved a pending ACTION_RSVP_CANCELLATION
     * request — cancel()'s own gate has already been satisfied by that
     * decision, so this applies the cancellation directly (same 24-hour
     * refund policy as a direct, ungated player cancellation) rather than
     * looping back through cancel()'s own bypass check.
     */
    public function completeCancellationAfterApproval(Rsvp $rsvp, Account $actor): void
    {
        if ($rsvp->isCanceled()) {
            return;
        }

        $wasPaidAndConfirmed = $rsvp->isPaid() && $rsvp->isConfirmed();
        $eligibleForRefund = $wasPaidAndConfirmed && $this->isAtLeast24HoursOut($rsvp->getEvent());

        $this->markCanceledAndNotify($rsvp, 'Parent approved the cancellation.');

        if ($eligibleForRefund) {
            $this->attemptRefund($rsvp, $actor);
        }
    }

    /**
     * BR-02-12: trainer-initiated event cancellation — always a full
     * refund, regardless of timing. Called once per affected RSVP, each in
     * its own transaction, AFTER the event's own cancellation has already
     * committed (architect-architecture.md "Trainer cancellation refunds":
     * "it locks the event, marks it Cancelled and commits... then refunds
     * each player... one transaction per RSVP").
     */
    public function cancelForEventCancellation(Rsvp $rsvp, Account $actor): void
    {
        if ($rsvp->isCanceled()) {
            return;
        }

        $wasPaidAndConfirmed = $rsvp->isPaid() && $rsvp->isConfirmed();

        $this->entityManager->wrapInTransaction(function () use ($rsvp): void {
            $rsvp->cancel('The event was canceled by the trainer.', new \DateTimeImmutable());
            $this->entityManager->flush();
        });

        $this->mailer->sendEventCanceledToPlayer($rsvp);

        if ($wasPaidAndConfirmed) {
            $this->attemptRefund($rsvp, $actor);
        }
    }

    /**
     * Q-02.09's resolved default (per the task brief, matching
     * specs/requirements-analyst-open-questions.md): a trainer may manually
     * add a player to a full event; the override is recorded in the audit
     * log (AuditLogger, already established by Epic-01). Bypasses the
     * capacity check entirely — Q-02.09 grants this without qualification —
     * but still takes the event lock, since two concurrent manual-adds
     * against the very last real spot must still serialize correctly
     * against each other and against an ordinary player RSVP.
     */
    public function manuallyAdd(Event $event, PlayerProfile $player, Account $trainerActor): Rsvp
    {
        if (null !== $this->rsvps->findActiveOneByEventAndPlayer($event, $player)) {
            throw AlreadyRegisteredException::forEventId((int) $event->getId());
        }

        $rsvp = $this->entityManager->wrapInTransaction(function () use ($event, $player, $trainerActor): Rsvp {
            $now = new \DateTimeImmutable();
            $lockedEvent = $this->events->lockForUpdate((int) $event->getId())
                ?? throw new \LogicException('Event no longer exists.');

            $wasFull = $this->rsvps->countHeld($lockedEvent) >= $lockedEvent->getCapacity();

            // See rsvp()'s own comment: reuse a previously-canceled row for
            // this (event, player) pair rather than re-inserting.
            $rsvp = $this->rsvps->findOneByEventAndPlayer($lockedEvent, $player);

            if (null !== $rsvp) {
                $rsvp->reactivate(Rsvp::METHOD_FREE, $now);
            } else {
                $rsvp = new Rsvp($lockedEvent->getTrainer(), $lockedEvent, $player, Rsvp::METHOD_FREE, $now);
                $this->rsvps->add($rsvp);
            }

            $rsvp->confirm($now);

            try {
                $this->entityManager->flush();
            } catch (UniqueConstraintViolationException) {
                throw AlreadyRegisteredException::forEventId((int) $lockedEvent->getId());
            }

            if ($wasFull) {
                $this->auditLogger->record($trainerActor, 'event.manual_add_over_capacity', 'Event', $lockedEvent->getId(), $lockedEvent->getTrainer(), [
                    'playerId' => $player->getId(),
                    'capacity' => $lockedEvent->getCapacity(),
                ]);
                $this->entityManager->flush();
            }

            return $rsvp;
        });

        $this->mailer->sendRsvpConfirmed($rsvp);

        return $rsvp;
    }

    /**
     * US-02.12: the trainer removes a player from the roster (distinct from
     * RsvpVoter::RSVP_CANCEL, the player's own cancellation) — issues a
     * refund if the RSVP was paid, per AC-02-45.
     */
    public function remove(Rsvp $rsvp, Account $trainerActor): void
    {
        $wasPaidAndConfirmed = $rsvp->isPaid() && $rsvp->isConfirmed();

        $this->markCanceledAndNotify($rsvp, 'Removed by the trainer.');

        if ($wasPaidAndConfirmed) {
            $this->attemptRefund($rsvp, $trainerActor);
        }
    }

    /**
     * BR-02-8/free events confirm at construction already; a paid RSVP that
     * bypassed child approval still needs its payment step.
     */
    private function completeConfirmationOrPayment(Rsvp $rsvp, Account $payer): void
    {
        if (Rsvp::STATUS_CONFIRMED === $rsvp->getStatus()) {
            $this->mailer->sendRsvpConfirmed($rsvp);

            return;
        }

        $event = $rsvp->getEvent();
        $amount = $event->priceForMethod($rsvp->getPaymentMethod());
        $request = new PaymentIntentRequest($rsvp->getTrainer(), $rsvp, $payer, $rsvp->getPaymentMethod(), $amount);
        $outcome = $this->paymentGateway->requestPayment($request);

        // AC-02-25: shown the price and "redirected to payment" — with the
        // shipped NoopPaymentIntentGateway this always stays Pending, which
        // is the honest, correct state until Epic-05 exists (see that
        // class's own docblock). AC-02-26's confirm-on-success path is
        // still fully exercised — by tests supplying a stub gateway that
        // returns Succeeded, proving this branch — even though production
        // never reaches it today.
        if (PaymentIntentOutcome::Succeeded === $outcome) {
            $rsvp->confirm(new \DateTimeImmutable());
            $this->entityManager->flush();
            $this->mailer->sendRsvpConfirmed($rsvp);
        }
    }

    /**
     * Shared by cancel(), remove(), and completeCancellationAfterApproval()
     * / cancelAfterParentDenial(): mark the row canceled and send the one
     * notification every one of those callers needs. Refund-eligibility
     * decisions stay in each caller, since they differ (24-hour policy for
     * a player-initiated cancellation vs. unconditional for a
     * trainer-initiated one) — this only owns the mechanics common to all
     * of them.
     */
    private function markCanceledAndNotify(Rsvp $rsvp, ?string $reason): void
    {
        $this->entityManager->wrapInTransaction(function () use ($rsvp, $reason): void {
            $rsvp->cancel($reason, new \DateTimeImmutable());
            $this->entityManager->flush();
        });

        $this->mailer->sendRsvpCanceled($rsvp);
    }

    private function fundingMethodForPaymentMethod(string $paymentMethod): FundingMethod
    {
        return match ($paymentMethod) {
            Rsvp::METHOD_FREE => FundingMethod::Free,
            Rsvp::METHOD_USD => FundingMethod::Usd,
            Rsvp::METHOD_TOKEN => FundingMethod::Token,
            default => throw new \InvalidArgumentException(sprintf('Unknown payment method "%s".', $paymentMethod)),
        };
    }

    /**
     * BR-02-11: full refund if canceled >= 24 hours before the event start,
     * none if < 24 hours — computed in the trainer's own timezone is not
     * necessary here since this is a fixed-duration comparison between two
     * absolute instants, not a calendar-day boundary.
     */
    private function isAtLeast24HoursOut(Event $event): bool
    {
        $now = new \DateTimeImmutable();
        $hoursUntilStart = ($event->getStartsAt()->getTimestamp() - $now->getTimestamp()) / 3600;

        return $hoursUntilStart >= 24;
    }

    private function attemptRefund(Rsvp $rsvp, Account $actor): void
    {
        $payer = $this->playerAccounts->resolve($rsvp->getPlayer()) ?? $actor;
        $event = $rsvp->getEvent();
        $amount = $event->priceForMethod($rsvp->getPaymentMethod());
        $request = new PaymentIntentRequest($rsvp->getTrainer(), $rsvp, $payer, $rsvp->getPaymentMethod(), $amount);
        $outcome = $this->paymentGateway->requestRefund($request);

        // AC-02-32: the confirmation email is sent only once the gateway
        // actually reports success — see NoopPaymentIntentGateway's own
        // docblock for why production never reaches this today.
        if (PaymentIntentOutcome::Succeeded === $outcome) {
            $this->mailer->sendRefundConfirmation($rsvp);
        }
    }
}
