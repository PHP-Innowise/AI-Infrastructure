<?php

declare(strict_types=1);

namespace App\Tests\Growth;

use App\Growth\Repository\ReferralLinkRepository;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Identity\Service\MembershipService;
use App\Tests\Support\CrmFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\GrowthFixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-06.01 — Player Has Automatic Referral Link.
 */
final class ReferralLinkTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;
    use CrmFixtureHelpers;
    use GrowthFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-06-1: no "generate" action — the link already exists (in the
     * format platform.com/join/{trainer-slug}/{player-id}) the very first
     * time the player visits the page, with no separate creation step.
     */
    public function testPlayerSeesAnAutomaticReferralLinkWithNoGenerateStep(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        // A brand-new, isolated player — Pat is shared across this whole
        // suite (other Growth tests provision Pat's own referral link
        // too), so only a genuinely fresh player can prove "no ReferralLink
        // row exists before the first visit" as a real precondition.
        $fresh = $this->createPlayerWithAccount($trainer, 'no-generate-step-'.uniqid().'@example.test');

        /** @var ReferralLinkRepository $links */
        $links = self::getContainer()->get(ReferralLinkRepository::class);
        self::assertNull($links->findOneByTrainerAndPlayer($trainer, $fresh['player']), 'Precondition: no referral link exists before the first visit.');

        $this->client->loginUser($fresh['account']);
        $crawler = $this->client->request('GET', '/portal/referrals');

        self::assertResponseIsSuccessful();
        $expectedUrl = sprintf('/join/%s/%d', $trainer->getSlug(), $fresh['player']->getId());
        $renderedValue = (string) $crawler->filter('input#referral-link-input')->attr('value');
        self::assertStringContainsString($expectedUrl, $renderedValue, 'AC-06-1: platform.com/join/{trainer-slug}/{player-id} format.');

        // KernelBrowser reboots the kernel on every request (see
        // CouponManagementTest's own note on this) — $links above is bound
        // to a now-discarded container; re-fetched fresh here.
        $this->activateTenant($trainer);
        /** @var ReferralLinkRepository $freshLinks */
        $freshLinks = self::getContainer()->get(ReferralLinkRepository::class);
        self::assertNotNull($freshLinks->findOneByTrainerAndPlayer($trainer, $fresh['player']), 'AC-06-1: the link now exists, provisioned automatically on first read.');
    }

    /**
     * AC-06-2: the "Get the Assist" section shows the link, a Share button,
     * and referral-count stats.
     */
    public function testPortalShowsShareButtonAndReferralCountStats(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        // A freshly created referrer, not the shared Pat fixture — other
        // Growth tests (ReferralAttributionTest) also credit Pat as a
        // referrer under this same trainer within one suite run, which
        // would make this test's own exact "referred 2 players" count
        // depend on execution order.
        $referrer = $this->createPlayerWithAccount($trainer, 'stats-referrer-'.uniqid().'@example.test');
        $link = $this->createReferralLink($trainer, $referrer['player']);

        $friendA = $this->freshPlayerMembership($trainer, 'Friend Converted')->getPlayer();
        $friendB = $this->freshPlayerMembership($trainer, 'Friend Pending')->getPlayer();
        $this->createReferral($trainer, $link, $referrer['player'], $friendA);
        $this->createReferral($trainer, $link, $referrer['player'], $friendB);

        $this->client->loginUser($referrer['account']);
        $crawler = $this->client->request('GET', '/portal/referrals');

        self::assertResponseIsSuccessful();
        self::assertSelectorExists('button', 'AC-06-2: a Share button is present.');
        self::assertSelectorTextContains('body', "You've referred 2 players", 'AC-06-2: referral-count stats.');
    }

    /**
     * AC-06-3: a player training with multiple trainers has a separate
     * link per trainer, and each referral credits that specific trainer's
     * program.
     */
    public function testPlayerWithMultipleTrainersHasASeparateLinkPerTrainer(): void
    {
        $trainerA = $this->trainer('peak-performance');
        $trainerB = $this->trainer('baseline-athletics');

        $this->activateTenant($trainerA);
        $membershipA = $this->freshPlayerMembership($trainerA, 'Multi Trainer Player');
        $player = $membershipA->getPlayer();

        // Associate the SAME player with a second trainer.
        $this->activateTenant($trainerB);
        /** @var MembershipService $membershipService */
        $membershipService = self::getContainer()->get(MembershipService::class);
        $membershipService->associatePlayer($trainerB, $player, PlayerTrainerMembership::SOURCE_EVENT_REGISTRATION);

        $this->activateTenant($trainerA);
        $linkA = $this->createReferralLink($trainerA, $player);
        $this->activateTenant($trainerB);
        $linkB = $this->createReferralLink($trainerB, $player);

        self::assertNotSame($linkA->getId(), $linkB->getId(), 'AC-06-3: a separate ReferralLink row per trainer.');

        // Referring a friend under trainer A must not affect trainer B's count.
        $this->activateTenant($trainerA);
        $friend = $this->freshPlayerMembership($trainerA, 'Friend Of Multi Trainer')->getPlayer();
        $this->createReferral($trainerA, $linkA, $player, $friend);

        /** @var ReferralLinkRepository $links */
        $links = self::getContainer()->get(ReferralLinkRepository::class);
        $statsA = self::getContainer()->get(\App\Growth\Repository\ReferralRepository::class)->statsForReferrer($trainerA, $player);
        self::assertSame(1, $statsA['total'], 'AC-06-3: the referral under trainer A is credited to trainer A.');

        $this->activateTenant($trainerB);
        $statsB = self::getContainer()->get(\App\Growth\Repository\ReferralRepository::class)->statsForReferrer($trainerB, $player);
        self::assertSame(0, $statsB['total'], 'AC-06-3: trainer B\'s own count is untouched.');
        self::assertNotNull($links->findOneByTrainerAndPlayer($trainerB, $player));
    }
}
