<?php

declare(strict_types=1);

namespace App\Scheduling\Billing;

use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;

/**
 * The narrow seam between Scheduling and Billing (Epic-05). Scheduling owns
 * this interface; `App\Billing\Service\SchedulingPaymentIntentGateway` is
 * Epic-05's real implementation (Stripe for `usd`, `TokenLedgerService` for
 * `token`), registered in `config/services.yaml` in place of the Epic-02
 * shipped `NoopPaymentIntentGateway` (removed — see its own git history).
 *
 * `RsvpService` is the only caller.
 */
interface PaymentIntentGateway
{
    /**
     * Architecture "Lock ordering": for a `token`-funded request, acquires
     * the token balance row lock — the FIRST lock in the fixed global order
     * (balance, then event). A no-op for `usd`/`free` requests, which touch
     * no balance row at all. `RsvpService::rsvp()` calls this BEFORE its own
     * `EventRepository::lockForUpdate()`, inside the same transaction, so
     * the two locks are genuinely acquired in the documented order rather
     * than merely claimed to be — see that method's own docblock.
     *
     * Takes primitives, not a `PaymentIntentRequest`, deliberately: this is
     * called before the `Rsvp` row exists (its own creation is itself
     * inside the locked section, after the event row is locked), so a
     * request shape requiring one would be circular.
     */
    public function lockFundingForUpdate(Trainer $trainer, Account $payer, string $paymentMethod): void;

    /**
     * AC-05-8: whether the payer can fund $amount at all, asked BEFORE any
     * RSVP row or payment record exists. Returns null when they can, or by
     * how much they fall short when they cannot.
     *
     * Deliberately unlocked, and therefore non-authoritative — the same
     * split AC-02-67 already draws for capacity: this is the fast check that
     * gives the player an honest answer and leaves nothing half-created,
     * while the locked check inside `requestPayment()` is the one that
     * actually decides. A balance that empties between the two is a race the
     * caller still handles (the RSVP stays Pending Payment), not a case this
     * method has to win.
     *
     * Takes primitives for the same reason `lockFundingForUpdate()` does:
     * it is called before the `Rsvp` exists.
     */
    public function findFundingShortfall(Trainer $trainer, Account $payer, string $paymentMethod, int $amount): ?FundingShortfall;

    /**
     * BR-02-9: request that a charge (or token spend) be taken for a paid
     * RSVP. Must not confirm the RSVP itself — the caller does that only on
     * `PaymentIntentOutcome::Succeeded`.
     */
    public function requestPayment(PaymentIntentRequest $request): PaymentIntentResult;

    /**
     * BR-02-11/12: request a refund for a canceled paid RSVP. The caller
     * has already decided eligibility (the 24-hour policy, or an
     * unconditional trainer-cancellation refund) before calling this — the
     * gateway only attempts the transfer.
     */
    public function requestRefund(PaymentIntentRequest $request): PaymentIntentResult;
}
