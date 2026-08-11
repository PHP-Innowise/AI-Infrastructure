<?php

declare(strict_types=1);

namespace App\Tests\Growth;

use App\Growth\Service\ReferralRewardService;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Tests\Support\BillingFixtureHelpers;
use App\Tests\Support\CrmFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\GrowthFixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-06.03 — System Awards Referral Rewards (First Purchase Trigger).
 *
 * Exercises `ReferralRewardService` directly rather than through HTTP — the
 * reward trigger is `PaymentRecordSettled`-driven (an internal event, never
 * a controller action) — but stays on `WebTestCase` (not plain
 * `KernelTestCase`) for `assertQueuedEmailCount()`'s own mailer assertions.
 */
final class ReferralRewardTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;
    use CrmFixtureHelpers;
    use BillingFixtureHelpers;
    use GrowthFixtureHelpers;

    protected function setUp(): void
    {
        self::createClient();
    }

    /**
     * AC-06-8/9/10: the referee's first purchase, once the assist count
     * reaches the platform-wide threshold, grants a token to the referrer
     * with that trainer AND sends a notification.
     */
    public function testReachingTheThresholdGrantsATokenRewardAndNotifiesTheReferrer(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->setReferralRule(1, 1);

        $referrer = $this->createPlayerWithAccount($trainer, 'reward-referrer@example.test');
        $referee = $this->createPlayerWithAccount($trainer, 'reward-referee@example.test');
        $link = $this->createReferralLink($trainer, $referrer['player']);
        $referral = $this->createReferral($trainer, $link, $referrer['player'], $referee['player']);

        $balanceBefore = $this->tokenBalance($trainer, $referrer['account']);
        $payment = $this->createCompletedPaymentRecord($trainer, $referee['account']);

        /** @var ReferralRewardService $rewardService */
        $rewardService = self::getContainer()->get(ReferralRewardService::class);
        $rewardService->processQualifyingPurchase($referee['player'], $payment, new \DateTimeImmutable());

        $this->activateTenant($trainer);
        self::assertTrue($referral->isConverted(), 'AC-06-8: the referral converts on the referee\'s first purchase.');
        self::assertSame($payment->getId(), $referral->getFirstPurchasePaymentRecord()?->getId());

        // AC-06-9/10: 1 referral required = 1 token, granted to Player A
        // with that specific trainer.
        self::assertSame($balanceBefore + 1, $this->tokenBalance($trainer, $referrer['account']), 'AC-06-10: token added to the referrer\'s balance.');

        $assistCount = $this->assistCountFor($trainer, $referrer['player']);
        self::assertNotNull($assistCount);
        self::assertSame(0, $assistCount->getAssistCount(), 'BR-06-5: the counter resets once a reward fires.');

        self::assertQueuedEmailCount(1, message: 'AC-06-10: a reward notification is sent.');
    }

    /**
     * AC-06-11: "7 total assists under a 3:1 rule equals 2 tokens earned,
     * with 1 assist toward the next" — this codebase's own worked example,
     * reproduced exactly.
     */
    public function testAssistCountTracksTowardTheThresholdAcrossMultipleReferrals(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->setReferralRule(3, 1);

        $referrer = $this->createPlayerWithAccount($trainer, 'assist-referrer@example.test');
        $link = $this->createReferralLink($trainer, $referrer['player']);

        /** @var ReferralRewardService $rewardService */
        $rewardService = self::getContainer()->get(ReferralRewardService::class);
        $balanceBefore = $this->tokenBalance($trainer, $referrer['account']);

        for ($i = 1; $i <= 7; ++$i) {
            $referee = $this->createPlayerWithAccount($trainer, sprintf('assist-referee-%d@example.test', $i));
            $this->createReferral($trainer, $link, $referrer['player'], $referee['player']);
            $payment = $this->createCompletedPaymentRecord($trainer, $referee['account']);

            $rewardService->processQualifyingPurchase($referee['player'], $payment, new \DateTimeImmutable());
            $this->activateTenant($trainer);

            // After the 3rd and 6th, a reward should have just fired and
            // reset the counter; every other assist just increments it.
            $expectedCount = match (true) {
                0 === $i % 3 => 0,
                default => $i % 3,
            };
            self::assertSame($expectedCount, $this->assistCountFor($trainer, $referrer['player'])?->getAssistCount(), sprintf('Assist count after referral #%d.', $i));
        }

        // 7 assists at 3:1 = 2 full thresholds crossed (at 3 and 6) = 2 tokens.
        self::assertSame($balanceBefore + 2, $this->tokenBalance($trainer, $referrer['account']), 'AC-06-11: 2 tokens earned from 7 assists under a 3:1 rule.');
        self::assertSame(1, $this->assistCountFor($trainer, $referrer['player'])?->getAssistCount(), 'AC-06-11: 1 assist left toward the next reward.');
    }

    /**
     * AC-06-12: the reward triggers ONLY on the referee's FIRST purchase —
     * a second qualifying event for the same, already-converted referee
     * must not increment the assist count again.
     */
    public function testRewardDoesNotRetriggerOnASecondPurchaseByTheSameReferee(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->setReferralRule(5, 1);

        $referrer = $this->createPlayerWithAccount($trainer, 'no-retrigger-referrer@example.test');
        $referee = $this->createPlayerWithAccount($trainer, 'no-retrigger-referee@example.test');
        $link = $this->createReferralLink($trainer, $referrer['player']);
        $this->createReferral($trainer, $link, $referrer['player'], $referee['player']);

        /** @var ReferralRewardService $rewardService */
        $rewardService = self::getContainer()->get(ReferralRewardService::class);

        $firstPayment = $this->createCompletedPaymentRecord($trainer, $referee['account']);
        $rewardService->processQualifyingPurchase($referee['player'], $firstPayment, new \DateTimeImmutable());
        $this->activateTenant($trainer);
        self::assertSame(1, $this->assistCountFor($trainer, $referrer['player'])?->getAssistCount());

        // A second, later purchase by the SAME referee.
        $secondPayment = $this->createCompletedPaymentRecord($trainer, $referee['account']);
        $rewardService->processQualifyingPurchase($referee['player'], $secondPayment, new \DateTimeImmutable());

        $this->activateTenant($trainer);
        self::assertSame(1, $this->assistCountFor($trainer, $referrer['player'])?->getAssistCount(), 'AC-06-12: the second purchase does not trigger a second assist.');
    }

    /**
     * AC-06-8: "checks that Player A and Player B are active" — this
     * codebase's own reading (active PlayerTrainerMembership with this
     * trainer, per BR-06-7's per-player-trainer framing) blocks the reward
     * when the referee's membership is inactive.
     */
    public function testInactiveRefereeMembershipBlocksTheReward(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->setReferralRule(1, 1);

        $referrer = $this->createPlayerWithAccount($trainer, 'inactive-check-referrer@example.test');
        $referee = $this->createPlayerWithAccount($trainer, 'inactive-check-referee@example.test');
        $link = $this->createReferralLink($trainer, $referrer['player']);
        $referral = $this->createReferral($trainer, $link, $referrer['player'], $referee['player']);

        /** @var PlayerTrainerMembershipRepository $memberships */
        $memberships = self::getContainer()->get(PlayerTrainerMembershipRepository::class);
        $membership = $memberships->findOneByTrainerAndPlayer($trainer, $referee['player']);
        self::assertNotNull($membership);
        $membership->remove();
        $this->growthFlushForTest();

        $balanceBefore = $this->tokenBalance($trainer, $referrer['account']);
        $payment = $this->createCompletedPaymentRecord($trainer, $referee['account']);

        /** @var ReferralRewardService $rewardService */
        $rewardService = self::getContainer()->get(ReferralRewardService::class);
        $rewardService->processQualifyingPurchase($referee['player'], $payment, new \DateTimeImmutable());

        $this->activateTenant($trainer);
        self::assertFalse($referral->isConverted(), 'AC-06-8: the referral does not convert while the referee\'s membership is inactive.');
        self::assertSame($balanceBefore, $this->tokenBalance($trainer, $referrer['account']), 'No reward granted.');
    }

    /**
     * AC-06-9: "applies to ALL trainers with no per-trainer customization"
     * — the same platform-wide ratio setting governs a second, independent
     * trainer's own referrals.
     */
    public function testPlatformWideRatioAppliesToEveryTrainerIdentically(): void
    {
        $trainerA = $this->trainer('peak-performance');
        $trainerB = $this->trainer('baseline-athletics');

        $this->activateTenant($trainerA);
        $this->setReferralRule(2, 1);

        $this->activateTenant($trainerB);
        $referrerB = $this->createPlayerWithAccount($trainerB, 'ratio-b-referrer@example.test');
        $refereeB1 = $this->createPlayerWithAccount($trainerB, 'ratio-b-referee-1@example.test');
        $refereeB2 = $this->createPlayerWithAccount($trainerB, 'ratio-b-referee-2@example.test');
        $linkB = $this->createReferralLink($trainerB, $referrerB['player']);
        $this->createReferral($trainerB, $linkB, $referrerB['player'], $refereeB1['player']);
        $this->createReferral($trainerB, $linkB, $referrerB['player'], $refereeB2['player']);

        /** @var ReferralRewardService $rewardService */
        $rewardService = self::getContainer()->get(ReferralRewardService::class);
        $balanceBefore = $this->tokenBalance($trainerB, $referrerB['account']);

        $payment1 = $this->createCompletedPaymentRecord($trainerB, $refereeB1['account']);
        $rewardService->processQualifyingPurchase($refereeB1['player'], $payment1, new \DateTimeImmutable());
        $this->activateTenant($trainerB);
        self::assertSame($balanceBefore, $this->tokenBalance($trainerB, $referrerB['account']), 'Not yet at the 2-referral threshold.');

        $payment2 = $this->createCompletedPaymentRecord($trainerB, $refereeB2['account']);
        $rewardService->processQualifyingPurchase($refereeB2['player'], $payment2, new \DateTimeImmutable());
        $this->activateTenant($trainerB);
        self::assertSame($balanceBefore + 1, $this->tokenBalance($trainerB, $referrerB['account']), 'AC-06-9: the SAME platform-wide 2:1 ratio (set via trainer A\'s own context) applies to trainer B too.');
    }

    private function growthFlushForTest(): void
    {
        self::getContainer()->get(\Doctrine\ORM\EntityManagerInterface::class)->flush();
    }
}
