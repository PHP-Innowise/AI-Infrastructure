<?php

declare(strict_types=1);

namespace App\Scheduling\EventSubscriber;

use App\Billing\Entity\PaymentRecord;
use App\Billing\Event\PaymentRecordSettled;
use App\Scheduling\Repository\RsvpRepository;
use App\Scheduling\Service\RsvpService;
use Symfony\Component\EventDispatcher\Attribute\AsEventListener;

/**
 * The "domain event dispatched from Billing and subscribed to by the
 * caller" half of architect-architecture.md's payment-outcome contract —
 * Billing never calls Scheduling directly (module map: "Billing... Must
 * not: Call Scheduling, Content, Forms or Growth"). Filters on
 * `PaymentRecord::TYPE_EVENT_RSVP`; every other type is silently ignored
 * (Content's own subscriber owns `content_purchase`).
 *
 * Only reachable for a `usd` (Stripe Checkout) RSVP — a `token` spend
 * always resolves synchronously, inside `RsvpService::rsvp()`'s own
 * transaction, and never reaches this subscriber at all.
 */
final readonly class PaymentOutcomeSubscriber
{
    public function __construct(
        private RsvpRepository $rsvps,
        private RsvpService $rsvpService,
    ) {
    }

    #[AsEventListener]
    public function onPaymentRecordSettled(PaymentRecordSettled $event): void
    {
        if (PaymentRecord::TYPE_EVENT_RSVP !== $event->type || null === $event->relatedRsvpId) {
            return;
        }

        $rsvp = $this->rsvps->find($event->relatedRsvpId);

        if (null === $rsvp) {
            return;
        }

        match ($event->outcome) {
            PaymentRecordSettled::OUTCOME_SUCCEEDED => $this->rsvpService->confirmAfterAsyncPayment($rsvp),
            PaymentRecordSettled::OUTCOME_FAILED => $this->rsvpService->failAfterAsyncPayment($rsvp),
            // OUTCOME_REFUNDED: transaction-history reconciliation is
            // PaymentRecord's own concern (already applied by the webhook
            // handler before this dispatches) — the RSVP's own cancellation
            // state was already set synchronously by whichever cancellation
            // path triggered the refund (cancel()/cancelForEventCancellation()),
            // so there is nothing further for Scheduling to do here.
            default => null,
        };
    }
}
