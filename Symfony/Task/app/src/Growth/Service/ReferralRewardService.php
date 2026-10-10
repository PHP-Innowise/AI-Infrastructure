<?php

declare(strict_types=1);

namespace App\Growth\Service;

use App\Billing\Entity\PaymentRecord;
use App\Billing\Service\TokenLedgerService;
use App\Growth\Entity\Referral;
use App\Growth\Repository\ReferralAssistCountRepository;
use App\Growth\Repository\ReferralRepository;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use Doctrine\ORM\EntityManagerInterface;

/**
 * US-06.03: the first-purchase reward trigger. Growth's one and only
 * sanctioned reach into Billing — "to grant a reward entry"
 * (`architect-architecture.md` module map) — via
 * `TokenLedgerService::referralReward()`, never by constructing a
 * `TokenEntry` directly.
 *
 * @see specs/requirements-analyst-epic-06-marketing-growth-spec.md AC-06-8..12, BR-06-5..7
 */
final readonly class ReferralRewardService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private ReferralRepository $referrals,
        private ReferralAssistCountRepository $assistCounts,
        private ReferralRuleService $referralRules,
        private PlayerTrainerMembershipRepository $memberships,
        private PlayerAccountResolver $playerAccounts,
        private TokenLedgerService $tokenLedger,
        private GrowthMailer $mailer,
    ) {
    }

    /**
     * AC-06-8/12: called once per successful `event_rsvp` / `content_
     * purchase` / `token_purchase` `PaymentRecordSettled` success, for
     * whichever player benefited from it. A no-op unless that player is
     * still a PENDING referee — see `Referral`'s own docblock for why the
     * pending -> converted transition IS this codebase's "first purchase"
     * detection, with no separate purchase-history query needed.
     */
    public function processQualifyingPurchase(PlayerProfile $refereePlayer, PaymentRecord $paymentRecord, \DateTimeImmutable $at): void
    {
        $referral = $this->referrals->findOneByRefereePlayer($refereePlayer);

        if (null === $referral || !$referral->isPending()) {
            return;
        }

        // AC-06-8: "checks that Player A and Player B are active" — read
        // as active PLAYER-TRAINER MEMBERSHIP with this specific trainer
        // (BR-06-7's own "per player-trainer relationship, never globally"
        // framing governs the whole reward workflow, so "active" is
        // interpreted the same way here rather than against the global
        // Account status). The open question of what happens when this
        // check fails is unresolved by the epic; this codebase's choice is
        // silent skip — no referral conversion, no reward, no error — the
        // safest reading given no stated deferral/forfeit mechanism exists.
        if (!$this->bothPlayersActiveWithTrainer($referral)) {
            return;
        }

        $this->entityManager->wrapInTransaction(function () use ($referral, $paymentRecord, $at): void {
            if (!$referral->markConverted($paymentRecord, $at)) {
                // Already converted by a concurrent/duplicate delivery —
                // AC-06-12: reward triggers ONLY on the first purchase.
                return;
            }

            $this->entityManager->flush();

            $this->incrementAssistAndRewardIfThresholdReached($referral);
        });
    }

    private function bothPlayersActiveWithTrainer(Referral $referral): bool
    {
        return $this->isActiveMember($referral->getReferrerPlayer(), $referral)
            && $this->isActiveMember($referral->getRefereePlayer(), $referral);
    }

    private function isActiveMember(PlayerProfile $player, Referral $referral): bool
    {
        $membership = $this->memberships->findOneByTrainerAndPlayer($referral->getTrainer(), $player);

        return null !== $membership && $membership->isActive();
    }

    /**
     * BR-06-5: increments the referrer's assist count; once it reaches the
     * platform-wide threshold, grants the token reward and resets the
     * counter. Runs inside the caller's already-open transaction.
     */
    private function incrementAssistAndRewardIfThresholdReached(Referral $referral): void
    {
        $trainer = $referral->getTrainer();
        $referrer = $referral->getReferrerPlayer();

        $assistCount = $this->assistCounts->lockForUpdate($trainer, $referrer);
        $assistCount->increment();
        $this->entityManager->flush();

        $rule = $this->referralRules->current();

        // AC-06-30: `>=`, not `===` — a threshold lowered by a Super Admin
        // between increments must still fire on the very next qualifying
        // referral even if the preserved count already exceeds the new,
        // lower threshold.
        if ($assistCount->getAssistCount() < $rule->referralsRequired) {
            return;
        }

        $assistCount->resetAfterReward();
        $this->entityManager->flush();

        $referrerAccount = $this->playerAccounts->resolve($referrer);

        if (null === $referrerAccount) {
            // No account to credit (should not happen in practice — every
            // referrer link belongs to a real, registered player) — the
            // assist count has already been reset above; nothing more to
            // do without an account to grant tokens to.
            return;
        }

        $entry = $this->tokenLedger->referralReward(
            $trainer,
            $referrerAccount,
            $rule->tokensAwarded,
            (int) $referral->getId(),
            sprintf('Referral reward: %d assist(s) with %s', $rule->referralsRequired, $trainer->getBusinessName()),
        );
        \assert(null !== $entry->getId());

        $this->mailer->sendReferralRewardEarned($referrerAccount, $referral->getRefereePlayer(), $trainer, $rule->tokensAwarded);
    }
}
