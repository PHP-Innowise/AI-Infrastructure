<?php

declare(strict_types=1);

namespace App\Tests\Growth;

use App\Growth\Service\ReferralRewardService;
use App\Tests\Support\BillingFixtureHelpers;
use App\Tests\Support\CrmFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\GrowthFixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-06.04 — Trainer Views Referral Dashboard.
 */
final class ReferralDashboardTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;
    use CrmFixtureHelpers;
    use BillingFixtureHelpers;
    use GrowthFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-06-13: "Marketing" -> "Referrals" ("Get the Assist").
     */
    public function testTrainerCanReachTheReferralDashboard(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $this->client->request('GET', '/trainer/marketing/referrals');

        self::assertResponseIsSuccessful();
    }

    /**
     * AC-06-14/15/16: overview metrics, Top Referrers leaderboard, and the
     * Referral Activity Log all reflect real, seeded data.
     */
    public function testDashboardShowsAccurateMetricsLeaderboardAndActivityLog(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->setReferralRule(1, 1);

        $topReferrer = $this->createPlayerWithAccount($trainer, 'dashboard-top-referrer@example.test');
        $link = $this->createReferralLink($trainer, $topReferrer['player']);

        // Two referrals: one converts (a real, completed purchase), one
        // stays pending.
        $convertedReferee = $this->createPlayerWithAccount($trainer, 'dashboard-converted@example.test');
        $pendingReferee = $this->createPlayerWithAccount($trainer, 'dashboard-pending@example.test');
        $this->createReferral($trainer, $link, $topReferrer['player'], $convertedReferee['player']);
        $this->createReferral($trainer, $link, $topReferrer['player'], $pendingReferee['player']);

        $payment = $this->createCompletedPaymentRecord($trainer, $convertedReferee['account'], 3000);
        /** @var ReferralRewardService $rewardService */
        $rewardService = self::getContainer()->get(ReferralRewardService::class);
        $rewardService->processQualifyingPurchase($convertedReferee['player'], $payment, new \DateTimeImmutable());
        $this->activateTenant($trainer);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/marketing/referrals');

        self::assertResponseIsSuccessful();

        // AC-06-14: 2 referrals this month, 1 conversion, 50% rate, $30 revenue.
        self::assertSelectorTextContains('body', '$30.00', 'AC-06-14: total referral revenue.');

        // AC-06-15: Top Referrers.
        self::assertSelectorTextContains('body', 'Test Player', 'AC-06-15: the referrer appears on the leaderboard.');

        // AC-06-16: Activity Log lists both referrals, one Converted, one Pending.
        self::assertSelectorTextContains('body', 'Converted', 'AC-06-16: activity log shows Converted status.');
        self::assertSelectorTextContains('body', 'Pending', 'AC-06-16: activity log shows Pending status.');
    }
}
