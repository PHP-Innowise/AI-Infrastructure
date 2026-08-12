<?php

declare(strict_types=1);

namespace App\Scheduling\Service;

use App\Billing\Entity\PaymentRecord;
use App\Growth\Entity\Coupon;
use App\Growth\Exception\InvalidCouponException;
use App\Growth\Service\CouponPricingService;
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
use App\Scheduling\Exception\InsufficientFundsException;
use App\Scheduling\Repository\EventRepository;
use App\Scheduling\Repository\RsvpRepository;
use Doctrine\DBAL\Exception\UniqueConstraintViolationException;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\SecurityBundle\Security;

/**
 * US-02.07/08: registering for and canceling out of an event.
 *
 * **Lock ordering (architect-architecture.md "Lock ordering"), now real**:
 * the fixed global order is the token balance row first, then the event
 * row. `rsvp()` calls `PaymentIntentGateway::lockFundingForUpdate()` — a
 * no-op for `usd`/`free`, a real `SELECT ... FOR UPDATE` on the token
 * balance row for `token` — as the FIRST statement inside its transaction,
 * strictly before `EventRepository::lockForUpdate()`. A bypass-granted
 * TOKEN payment then completes fully inside that same transaction
 * (`completeTokenPaymentWithinTransaction()`): the spend and the RSVP's own
 * confirmation commit atomically, so there is never a confirmed RSVP with
 * no tokens taken, or tokens taken with no RSVP. A `usd` payment never
 * holds either lock while talking to Stripe — Checkout Session creation
 * happens strictly AFTER this transaction commits
 * (`completeConfirmationOrPayment()`), matching the architecture's own
 * "no lock is ever held across a network call" reasoning for trainer
 * cancellation's own refund fan-out.
 *
 * AC-02-67's unlocked-vs-locked split: RsvpVoter::RSVP_CREATE does a fast,
 * non-authoritative capacity pre-check for UX; the count taken here, after
 * EventRepository::lockForUpdate(), is the one that actually decides —
 * "An unlocked capacity pre-check gives fail-fast UX... the locked check
 * inside the transaction is the one that decides" (architect-architecture.md).
 *
 * @see specs/requirements-analyst-epic-02-event-management-spec.md BR-02-7..11, AC-02-23..33
 * @see specs/requirements-analyst-epic-05-payments-tokens-spec.md AC-05-7..17, BR-05-2
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
        private CouponPricingService $couponPricing,
    ) {
    }

    /**
     * @throws AlreadyRegisteredException
     * @throws EventFullException
     * @throws InsufficientFundsException
     * @throws InvalidCouponException
     */
    public function rsvp(Event $event, PlayerProfile $player, Account $actor, string $paymentMethod, ?string $couponCode = null): Rsvp
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

        // AC-05-8: asked before the transaction opens, so an unaffordable
        // RSVP creates nothing at all — no row to reactivate later, no
        // payment record, no half-registration the player has to wonder
        // about. Only the bypass-granted token path pays here; a child's
        // unapproved request pays at approval time
        // (completeAfterParentApproval()), where the parent's balance is
        // what matters and is checked then.
        if ($bypassGranted && Event::PAYMENT_TOKEN === $paymentMethod) {
            $shortfall = $this->paymentGateway->findFundingShortfall(
                $event->getTrainer(),
                $actor,
                $paymentMethod,
                $event->priceForMethod($paymentMethod),
            );

            if (null !== $shortfall) {
                throw InsufficientFundsException::forShortfall($shortfall);
            }
        }

        $rsvp = $this->entityManager->wrapInTransaction(function () use ($event, $player, $actor, $paymentMethod, $bypassGranted): Rsvp {
            // Architecture "Lock ordering": the token balance row, always
            // first — see the class docblock.
            $this->paymentGateway->lockFundingForUpdate($event->getTrainer(), $actor, $paymentMethod);

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

            // AC-05-7/9: a bypass-granted token spend completes HERE —
            // inside the same transaction and lock scope as the RSVP row
            // and the event capacity check, "no payment-processing delay,"
            // and atomic with the RSVP's own confirmation. usd/free are
            // untouched by this branch (free auto-confirmed at
            // construction already; usd is handled after this transaction
            // commits, in completeConfirmationOrPayment()).
            if ($bypassGranted && Event::PAYMENT_TOKEN === $paymentMethod && Rsvp::STATUS_CONFIRMED !== $rsvp->getStatus()) {
                $this->completeTokenPaymentWithinTransaction($rsvp, $actor);
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

        if (Event::PAYMENT_TOKEN === $paymentMethod) {
            // Already resolved (Succeeded or, on an insufficient-balance
            // race, still Pending Payment) inside the transaction above —
            // only the notification (a side effect that must never run
            // inside an open DB transaction) happens out here.
            if (Rsvp::STATUS_CONFIRMED === $rsvp->getStatus()) {
                $this->mailer->sendRsvpConfirmed($rsvp);
            }

            return $rsvp;
        }

        $this->completeConfirmationOrPayment($rsvp, $actor, $couponCode);

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
     * AC-05-34: the async webhook handler
     * (App\Billing\MessageHandler\ProcessStripeWebhookEventHandler, via
     * App\Scheduling\EventSubscriber\PaymentOutcomeSubscriber) calls this
     * once `payment_intent.succeeded` confirms a `usd` RSVP that stayed
     * Pending Payment when rsvp()/completeAfterParentApproval() first ran.
     * A no-op if the RSVP is somehow already confirmed (redelivery safety —
     * the webhook receipt's own unique constraint is the primary guard, this
     * is a second, cheap one).
     */
    public function confirmAfterAsyncPayment(Rsvp $rsvp): void
    {
        if (Rsvp::STATUS_CONFIRMED === $rsvp->getStatus()) {
            return;
        }

        $this->entityManager->wrapInTransaction(function () use ($rsvp): void {
            $rsvp->confirm(new \DateTimeImmutable());
            $this->entityManager->flush();
        });

        $this->mailer->sendRsvpConfirmed($rsvp);
    }

    /**
     * BR-05-8/BR-05-9: the async webhook handler calls this once
     * `payment_intent.payment_failed` arrives (or the 7-day pending-payment
     * sweep gives up) for a `usd` RSVP — "the RSVP is not confirmed and the
     * spot remains available." Cancels rather than leaving it stuck at
     * Pending Payment forever (which would otherwise go on holding a spot —
     * see `Rsvp::CAPACITY_HOLDING_STATUSES`).
     */
    public function failAfterAsyncPayment(Rsvp $rsvp): void
    {
        if (Rsvp::STATUS_CONFIRMED === $rsvp->getStatus() || Rsvp::STATUS_CANCELED === $rsvp->getStatus()) {
            return;
        }

        $this->markCanceledAndNotify($rsvp, 'Payment failed or was not completed in time.');
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
     * AC-05-7: called only from WITHIN rsvp()'s own open transaction (see
     * that method's own docblock on lock ordering) — the token balance row
     * is already locked by the time this runs. Confirms the RSVP in the
     * same commit as the ledger spend on success; on Failed (an
     * insufficient-balance race the caller's own pre-check did not catch)
     * leaves the RSVP at Pending Payment, matching BR-05-8's "not confirmed,
     * spot remains available" — except a token RSVP's spot genuinely does
     * NOT remain available under this codebase's own capacity model (Pending
     * Payment still holds a spot, see Rsvp::CAPACITY_HOLDING_STATUSES) — an
     * acknowledged, narrow gap recorded in the coder's final report.
     */
    private function completeTokenPaymentWithinTransaction(Rsvp $rsvp, Account $payer): void
    {
        $event = $rsvp->getEvent();
        $amount = $event->priceForMethod($rsvp->getPaymentMethod());
        $request = new PaymentIntentRequest($rsvp->getTrainer(), $rsvp, $payer, $rsvp->getPaymentMethod(), $amount);
        $result = $this->paymentGateway->requestPayment($request);
        $this->attachPaymentRecordIfPresent($rsvp, $result->paymentRecordId);

        if (PaymentIntentOutcome::Succeeded === $result->outcome) {
            $rsvp->confirm(new \DateTimeImmutable());
        }

        $this->entityManager->flush();
    }

    /**
     * BR-02-8/free events confirm at construction already; a paid RSVP that
     * bypassed child approval still needs its payment step. Reached for
     * `usd` (always) and, defensively, `token` (never in practice — the
     * bypass-granted token path is fully resolved inside rsvp()'s own
     * transaction before this method could ever be reached for it, per the
     * class docblock's lock-ordering reasoning) — kept general rather than
     * usd-only so `completeAfterParentApproval()`'s own token path (a
     * parent-approved child's token purchase, which does NOT go through
     * rsvp()'s inline branch) still resolves correctly.
     *
     * $couponCode is only ever non-null on the immediate, bypass-granted
     * `usd` path from `rsvp()` itself — `completeAfterParentApproval()`
     * never carries one through (no column preserves a coupon code across
     * the approval window, matching
     * `App\Content\Service\PurchasePlaylistAccessService`'s identical
     * "child's original choice is not preserved" precedent for its own
     * payment method).
     *
     * @throws InvalidCouponException
     */
    private function completeConfirmationOrPayment(Rsvp $rsvp, Account $payer, ?string $couponCode = null): void
    {
        if (Rsvp::STATUS_CONFIRMED === $rsvp->getStatus()) {
            $this->mailer->sendRsvpConfirmed($rsvp);

            return;
        }

        $event = $rsvp->getEvent();
        $originalAmount = $event->priceForMethod($rsvp->getPaymentMethod());
        [$amount, $appliedCouponCode] = $this->applyCouponIfPresent($event, $rsvp->getPlayer(), $rsvp->getPaymentMethod(), $originalAmount, $couponCode);
        $request = new PaymentIntentRequest($rsvp->getTrainer(), $rsvp, $payer, $rsvp->getPaymentMethod(), $amount, $appliedCouponCode);
        $result = $this->paymentGateway->requestPayment($request);

        if (null !== $result->paymentRecordId) {
            $this->attachPaymentRecordIfPresent($rsvp, $result->paymentRecordId);
            $this->entityManager->flush();
        }

        // AC-05-10: "redirected to Stripe Checkout" — carried back to the
        // controller via the entity's own transient property (never
        // persisted — see Rsvp::$pendingCheckoutUrl's own docblock).
        if (null !== $result->redirectUrl) {
            $rsvp->setPendingCheckoutUrl($result->redirectUrl);
        }

        if (PaymentIntentOutcome::Succeeded === $result->outcome) {
            $rsvp->confirm(new \DateTimeImmutable());
            $this->entityManager->flush();
            $this->mailer->sendRsvpConfirmed($rsvp);
        }
    }

    /**
     * AC-06-20..23, BR-06-9: coupons only ever discount a CARD (usd) RSVP —
     * see `App\Content\Service\PurchasePlaylistAccessService::
     * applyCouponIfPresent()`'s own docblock for the identical reasoning. A
     * token-funded RSVP silently ignores any submitted coupon code.
     *
     * @return array{0: int, 1: ?string} [amount to actually charge, the
     *                                    coupon code to carry through to
     *                                    checkout metadata]
     *
     * @throws InvalidCouponException
     */
    private function applyCouponIfPresent(Event $event, PlayerProfile $player, string $paymentMethod, int $originalAmount, ?string $couponCode): array
    {
        if (Event::PAYMENT_USD !== $paymentMethod || null === $couponCode || '' === trim($couponCode)) {
            return [$originalAmount, null];
        }

        $quote = $this->couponPricing->quote($event->getTrainer(), $couponCode, $player, Coupon::APPLIES_TO_EVENTS, $originalAmount);

        if (!$quote->valid) {
            // AC-06-23: "Invalid or expired code" — no discount applied,
            // no payment attempted.
            throw InvalidCouponException::withMessage($quote->message);
        }

        \assert(null !== $quote->finalAmountMinorUnits && null !== $quote->coupon);

        return [$quote->finalAmountMinorUnits, $quote->coupon->getCode()];
    }

    /**
     * Shared by cancel(), remove(), and completeCancellationAfterApproval()
     * / cancelAfterParentDenial() / failAfterAsyncPayment(): mark the row
     * canceled and send the one notification every one of those callers
     * needs. Refund-eligibility decisions stay in each caller, since they
     * differ (24-hour policy for a player-initiated cancellation vs.
     * unconditional for a trainer-initiated one, none for a payment
     * failure) — this only owns the mechanics common to all of them.
     */
    private function markCanceledAndNotify(Rsvp $rsvp, ?string $reason): void
    {
        $this->entityManager->wrapInTransaction(function () use ($rsvp, $reason): void {
            $rsvp->cancel($reason, new \DateTimeImmutable());
            $this->entityManager->flush();
        });

        $this->mailer->sendRsvpCanceled($rsvp);
    }

    /**
     * `Rsvp::$paymentRecord` is a real ORM relation (see that entity's own
     * docblock for why it stopped being a deferred scalar column) — the
     * gateway only ever hands back an id, so this resolves the actual
     * managed entity via a reference (no extra SELECT) before attaching it.
     */
    private function attachPaymentRecordIfPresent(Rsvp $rsvp, ?int $paymentRecordId): void
    {
        if (null === $paymentRecordId) {
            return;
        }

        /** @var PaymentRecord $paymentRecord */
        $paymentRecord = $this->entityManager->getReference(PaymentRecord::class, $paymentRecordId);
        $rsvp->attachPaymentRecord($paymentRecord);
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
        $result = $this->paymentGateway->requestRefund($request);

        // AC-05-14/16/17: the confirmation email is sent only once the
        // gateway actually reports success.
        if (PaymentIntentOutcome::Succeeded === $result->outcome) {
            $this->mailer->sendRefundConfirmation($rsvp);
        }
    }
}
