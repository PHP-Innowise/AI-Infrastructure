<?php

declare(strict_types=1);

namespace App\Scheduling\EventSubscriber;

use App\Billing\Entity\PaymentRecord;
use App\Billing\Event\PaymentRecordSettled;
use App\Billing\Repository\PaymentRecordRepository;
use App\Growth\Repository\CouponRepository;
use App\Growth\Service\CouponPricingService;
use App\Scheduling\Entity\Event;
use App\Scheduling\Entity\Rsvp;
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
        private PaymentRecordRepository $paymentRecords,
        private CouponRepository $coupons,
        private CouponPricingService $couponPricing,
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
            PaymentRecordSettled::OUTCOME_SUCCEEDED => $this->onSucceeded($event, $rsvp),
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

    private function onSucceeded(PaymentRecordSettled $event, Rsvp $rsvp): void
    {
        $this->rsvpService->confirmAfterAsyncPayment($rsvp);

        // AC-06-25: records the coupon use once the discounted payment has
        // actually succeeded — 'coupon_code' only appears in metadata when
        // RsvpService::rsvp() actually applied one (see
        // App\Scheduling\Billing\PaymentIntentRequest::$couponCode's own
        // docblock).
        $couponCode = isset($event->metadata['coupon_code']) ? (string) $event->metadata['coupon_code'] : null;

        if (null === $couponCode || '' === $couponCode) {
            return;
        }

        $coupon = $this->coupons->findOneByTrainerAndCode($rsvp->getTrainer(), $couponCode);

        if (null === $coupon) {
            return;
        }

        $paymentRecord = $this->paymentRecords->find($event->paymentRecordId);

        if (null === $paymentRecord) {
            return;
        }

        // The original (undiscounted) price is derivable at any time from
        // the event's own static pricing — no need to have carried it
        // through Stripe metadata alongside the coupon code.
        $original = $rsvp->getEvent()->priceForMethod(Event::PAYMENT_USD);
        $final = $paymentRecord->getAmountMinorUnits();
        $discount = max(0, $original - $final);

        $this->couponPricing->redeem($coupon, $rsvp->getPlayer(), $paymentRecord, $original, $discount, $final);
    }
}
