<?php

declare(strict_types=1);

namespace App\Content\EventSubscriber;

use App\Billing\Entity\PaymentRecord;
use App\Billing\Event\PaymentRecordSettled;
use App\Billing\Repository\PaymentRecordRepository;
use App\Content\Entity\Playlist;
use App\Content\Repository\PlaylistRepository;
use App\Content\Service\PurchasePlaylistAccessService;
use App\Growth\Repository\CouponRepository;
use App\Growth\Service\CouponPricingService;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Repository\PlayerProfileRepository;
use Symfony\Component\EventDispatcher\Attribute\AsEventListener;

/**
 * Content's own half of Billing's payment-outcome contract — see
 * `App\Scheduling\EventSubscriber\PaymentOutcomeSubscriber`'s own docblock
 * for the full reasoning. Filters on
 * `PaymentRecord::TYPE_CONTENT_PURCHASE`.
 *
 * Only reachable for a `usd` (Stripe Checkout) content purchase — a `token`
 * spend always resolves synchronously inside
 * `PurchasePlaylistAccessService::attemptGrant()`.
 *
 * The beneficiary player is not a `payment_record` column (that table links
 * a playlist and a payer account, never a player profile — see
 * `PaymentRecord`'s own docblock) — it travels as Stripe Checkout metadata
 * instead (`ContentPaymentIntentGateway::startCardCheckout()`), the same
 * mechanism `App\Billing\Event\PaymentRecordSettled::$metadata`'s own
 * docblock documents for the subscription activation date.
 */
final readonly class PaymentOutcomeSubscriber
{
    public function __construct(
        private PlaylistRepository $playlists,
        private PlayerProfileRepository $playerProfiles,
        private PaymentRecordRepository $paymentRecords,
        private PurchasePlaylistAccessService $purchaseService,
        private CouponRepository $coupons,
        private CouponPricingService $couponPricing,
    ) {
    }

    #[AsEventListener]
    public function onPaymentRecordSettled(PaymentRecordSettled $event): void
    {
        if (PaymentRecord::TYPE_CONTENT_PURCHASE !== $event->type
            || PaymentRecordSettled::OUTCOME_SUCCEEDED !== $event->outcome
            || null === $event->relatedPlaylistId
        ) {
            return;
        }

        $playlist = $this->playlists->find($event->relatedPlaylistId);
        $playerId = isset($event->metadata['player_id']) ? (int) $event->metadata['player_id'] : null;
        $player = null !== $playerId ? $this->playerProfiles->find($playerId) : null;
        $paymentRecord = $this->paymentRecords->find($event->paymentRecordId);
        $payer = $paymentRecord?->getPayerAccount();

        if (null === $playlist || null === $player || null === $payer) {
            return;
        }

        // AC-05-18: unlocks the playlist, attributed to whoever actually
        // paid (`payment_record.payer_account_id`) — the same account
        // `PurchasePlaylistAccessService::attemptGrant()` would have
        // resolved via `PlayerAccountResolver` at request time.
        $this->purchaseService->grantFor($playlist, $player, $payer, $event->paymentRecordId);

        // AC-06-25: records the coupon use once the discounted payment has
        // actually succeeded — 'coupon_code' only appears in metadata when
        // `PurchasePlaylistAccessService::purchase()` actually applied one
        // (see PaymentIntentRequest::$couponCode's own docblock).
        $this->recordCouponRedemptionIfPresent($event, $playlist, $player, $paymentRecord);
    }

    private function recordCouponRedemptionIfPresent(
        PaymentRecordSettled $event,
        Playlist $playlist,
        PlayerProfile $player,
        PaymentRecord $paymentRecord,
    ): void {
        $couponCode = isset($event->metadata['coupon_code']) ? (string) $event->metadata['coupon_code'] : null;

        if (null === $couponCode || '' === $couponCode) {
            return;
        }

        $coupon = $this->coupons->findOneByTrainerAndCode($playlist->getTrainer(), $couponCode);

        if (null === $coupon) {
            // The coupon existed at checkout time but is gone by the time
            // the webhook confirms — nothing sane to record against; the
            // discount was already applied to the charge itself regardless.
            return;
        }

        // The original (undiscounted) price is derivable at any time from
        // the playlist's own static pricing — no need to have carried it
        // through Stripe metadata alongside the coupon code.
        $original = $playlist->priceForMethod('usd');
        $final = $paymentRecord->getAmountMinorUnits();
        $discount = max(0, $original - $final);

        $this->couponPricing->redeem($coupon, $player, $paymentRecord, $original, $discount, $final);
    }
}
