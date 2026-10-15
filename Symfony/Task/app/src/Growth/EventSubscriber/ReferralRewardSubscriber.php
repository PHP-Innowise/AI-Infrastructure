<?php

declare(strict_types=1);

namespace App\Growth\EventSubscriber;

use App\Billing\Entity\PaymentRecord;
use App\Billing\Event\PaymentRecordSettled;
use App\Billing\Repository\PaymentRecordRepository;
use App\Growth\Repository\ReferralRepository;
use App\Growth\Service\PlayerAccountResolver;
use App\Growth\Service\ReferralRewardService;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Repository\PlayerProfileRepository;
use Symfony\Component\EventDispatcher\Attribute\AsEventListener;

/**
 * US-06.03: Growth's half of the "domain event dispatched from Billing and
 * subscribed to by the caller" contract — see
 * `App\Scheduling\EventSubscriber\PaymentOutcomeSubscriber`'s own docblock
 * for the full reasoning (identical shape). Billing never calls Growth
 * directly; this listens to the SAME `PaymentRecordSettled` event
 * Scheduling's and Content's own subscribers already consume, filtering for
 * the three purchase types AC-06-8 names as first-purchase triggers:
 * `event_rsvp`, `content_purchase`, `token_purchase`.
 *
 * **Resolving "which player benefited" per type** — `PaymentRecordSettled`
 * itself does not carry a uniform beneficiary-player field (see
 * `PaymentRecord`'s own docblock: "the beneficiary player is not a
 * payment_record column"):
 *
 * - `event_rsvp`: via `PaymentRecord::$relatedRsvp->getPlayer()` — a real
 *   Doctrine association Billing's own entity already carries (the same
 *   "ledger/payment entity references the domain entity it originated
 *   from" shape `TokenEntry::$relatedEvent` and `PaymentRecord::
 *   $relatedRsvp`/`$relatedPlaylist` themselves already establish), not a
 *   read through Scheduling's own repository or service.
 * - `content_purchase`: via `$event->metadata['player_id']` — the same
 *   Stripe Checkout metadata mechanism Content's own subscriber already
 *   reads.
 * - `token_purchase`: **no natural per-player linkage exists.** A token
 *   purchase funds the (payer account, trainer) balance, never a specific
 *   child (`TokenBalance`'s own shape) — a purchase cannot be attributed to
 *   ONE beneficiary child among several a parent may have referred under
 *   the same trainer. This codebase's resolution: every still-PENDING
 *   referral under this trainer whose referee resolves (via
 *   `PlayerAccountResolver`) to the SAME payer account is treated as
 *   converted by this purchase. This can over-credit in the (expected to
 *   be rare) case of one parent account having multiple pending referrals
 *   under one trainer — recorded as a known, documented limitation in the
 *   coder's final report rather than silently assumed away.
 */
final readonly class ReferralRewardSubscriber
{
    private const QUALIFYING_TYPES = [
        PaymentRecord::TYPE_EVENT_RSVP,
        PaymentRecord::TYPE_CONTENT_PURCHASE,
        PaymentRecord::TYPE_TOKEN_PURCHASE,
    ];

    public function __construct(
        private PaymentRecordRepository $paymentRecords,
        private PlayerProfileRepository $playerProfiles,
        private ReferralRepository $referrals,
        private PlayerAccountResolver $playerAccounts,
        private ReferralRewardService $rewardService,
    ) {
    }

    #[AsEventListener]
    public function onPaymentRecordSettled(PaymentRecordSettled $event): void
    {
        if (PaymentRecordSettled::OUTCOME_SUCCEEDED !== $event->outcome) {
            return;
        }

        if (!\in_array($event->type, self::QUALIFYING_TYPES, true)) {
            return;
        }

        $paymentRecord = $this->paymentRecords->find($event->paymentRecordId);

        if (null === $paymentRecord) {
            return;
        }

        $at = new \DateTimeImmutable();

        foreach ($this->resolveBeneficiaryPlayers($event, $paymentRecord) as $player) {
            $this->rewardService->processQualifyingPurchase($player, $paymentRecord, $at);
        }
    }

    /**
     * @return list<PlayerProfile>
     */
    private function resolveBeneficiaryPlayers(PaymentRecordSettled $event, PaymentRecord $paymentRecord): array
    {
        return match ($event->type) {
            PaymentRecord::TYPE_EVENT_RSVP => $this->fromRelatedRsvp($paymentRecord),
            PaymentRecord::TYPE_CONTENT_PURCHASE => $this->fromMetadataPlayerId($event),
            PaymentRecord::TYPE_TOKEN_PURCHASE => $this->fromTokenPurchasePayer($paymentRecord),
            default => [],
        };
    }

    /**
     * @return list<PlayerProfile>
     */
    private function fromRelatedRsvp(PaymentRecord $paymentRecord): array
    {
        $player = $paymentRecord->getRelatedRsvp()?->getPlayer();

        return null !== $player ? [$player] : [];
    }

    /**
     * @return list<PlayerProfile>
     */
    private function fromMetadataPlayerId(PaymentRecordSettled $event): array
    {
        $playerId = isset($event->metadata['player_id']) ? (int) $event->metadata['player_id'] : null;

        if (null === $playerId) {
            return [];
        }

        $player = $this->playerProfiles->find($playerId);

        return null !== $player ? [$player] : [];
    }

    /**
     * @return list<PlayerProfile>
     */
    private function fromTokenPurchasePayer(PaymentRecord $paymentRecord): array
    {
        $payer = $paymentRecord->getPayerAccount();

        if (null === $payer) {
            return [];
        }

        $matches = [];

        foreach ($this->referrals->findPendingForTrainer($paymentRecord->getTrainer()) as $pendingReferral) {
            $refereePlayer = $pendingReferral->getRefereePlayer();

            if ($this->playerAccounts->resolve($refereePlayer)?->getId() === $payer->getId()) {
                $matches[] = $refereePlayer;
            }
        }

        return $matches;
    }
}
