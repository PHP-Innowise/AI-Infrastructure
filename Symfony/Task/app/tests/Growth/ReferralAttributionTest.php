<?php

declare(strict_types=1);

namespace App\Tests\Growth;

use App\Growth\Entity\Referral;
use App\Growth\Repository\ReferralRepository;
use App\Growth\Service\ReferralAttributionService;
use App\Tests\Support\CrmFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\GrowthFixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\HttpFoundation\Request;

/**
 * US-06.02 — Player Refers Friend.
 */
final class ReferralAttributionTest extends WebTestCase
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
     * AC-06-4/5: clicking the link lands the friend on the trainer's own
     * registration page; after they register, a Pending referral is
     * recorded, crediting the trainer whose link was clicked.
     */
    public function testFriendClickingLinkAndRegisteringCreatesAPendingReferral(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $pat = $this->patPlayer();

        $joinCrawler = $this->client->request('GET', sprintf('/join/%s/%d', $trainer->getSlug(), $pat->getId()));
        self::assertResponseRedirects(null, 302, 'AC-06-4: the click redirects to the registration page.');
        self::assertNotNull($this->client->getCookieJar()->get(ReferralAttributionService::COOKIE_NAME), 'AC-06-5: the click is tracked via the attribution cookie.');

        $registerCrawler = $this->client->followRedirect();
        self::assertResponseIsSuccessful();
        self::assertSelectorExists('form[name="player_registration"]', 'AC-06-4: taken straight to the registration page.');

        $form = $registerCrawler->selectButton('Register')->form([
            'player_registration[accountFirstName]' => 'Riley',
            'player_registration[accountLastName]' => 'Referred',
            'player_registration[email]' => 'riley.referred@example.test',
            'player_registration[plainPassword]' => 'correct-horse-battery',
            'player_registration[playerFirstName]' => 'Riley',
            'player_registration[playerDateOfBirth]' => sprintf('%d-05-10', ((int) date('Y')) - 28),
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/dashboard');

        $this->activateTenant($trainer);
        /** @var ReferralRepository $referrals */
        $referrals = self::getContainer()->get(ReferralRepository::class);
        $referee = $this->account('riley.referred@example.test');
        /** @var \App\Identity\Repository\PlayerProfileRepository $playerProfiles */
        $playerProfiles = self::getContainer()->get(\App\Identity\Repository\PlayerProfileRepository::class);
        $refereePlayer = $playerProfiles->findOneForSelfAccount($referee);
        self::assertNotNull($refereePlayer);

        $referral = $referrals->findOneByRefereePlayer($refereePlayer);
        self::assertNotNull($referral, 'AC-06-5: a referral row was recorded after registration.');
        self::assertTrue($referral->isPending(), 'AC-06-5: status is Pending, awaiting first purchase.');
        self::assertSame($pat->getId(), $referral->getReferrerPlayer()->getId());
        self::assertSame($trainer->getId(), $referral->getTrainer()->getId());
    }

    /**
     * AC-06-6/BR-06-2: last-click wins — clicking a second referral link
     * before registering credits the SECOND (most recent) referrer.
     */
    public function testLastClickWinsWhenMultipleLinksAreClickedBeforeRegistering(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $pat = $this->patPlayer();
        $secondReferrer = $this->freshPlayerMembership($trainer, 'Second Referrer')->getPlayer();

        // First click: Pat's link.
        $this->client->request('GET', sprintf('/join/%s/%d', $trainer->getSlug(), $pat->getId()));
        // Second click, before registering: the other player's link.
        $this->client->request('GET', sprintf('/join/%s/%d', $trainer->getSlug(), $secondReferrer->getId()));

        $registerCrawler = $this->client->followRedirect();
        $form = $registerCrawler->selectButton('Register')->form([
            'player_registration[accountFirstName]' => 'Casey',
            'player_registration[accountLastName]' => 'LastClick',
            'player_registration[email]' => 'casey.lastclick@example.test',
            'player_registration[plainPassword]' => 'correct-horse-battery',
            'player_registration[playerFirstName]' => 'Casey',
            'player_registration[playerDateOfBirth]' => sprintf('%d-03-03', ((int) date('Y')) - 26),
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/dashboard');

        $this->activateTenant($trainer);
        /** @var ReferralRepository $referrals */
        $referrals = self::getContainer()->get(ReferralRepository::class);
        $refereePlayer = self::getContainer()->get(\App\Identity\Repository\PlayerProfileRepository::class)
            ->findOneForSelfAccount($this->account('casey.lastclick@example.test'));
        self::assertNotNull($refereePlayer);

        $referral = $referrals->findOneByRefereePlayer($refereePlayer);
        self::assertNotNull($referral);
        self::assertSame($secondReferrer->getId(), $referral->getReferrerPlayer()->getId(), 'BR-06-2: the LAST clicked link is credited.');
    }

    /**
     * AC-06-6/BR-06-1: attribution expires after the (configurable)
     * window — tested directly at the service boundary with a crafted
     * cookie payload, the precise seam this business rule lives at.
     */
    public function testAttributionExpiresAfterTheConfiguredWindow(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->setReferralAttributionWindowDays(30);

        $referrer = $this->freshPlayerMembership($trainer, 'Window Referrer')->getPlayer();
        $link = $this->createReferralLink($trainer, $referrer);
        $referee = $this->freshPlayerMembership($trainer, 'Window Referee')->getPlayer();

        $request = Request::create('/join/'.$trainer->getSlug().'/register');
        $request->cookies->set(ReferralAttributionService::COOKIE_NAME, json_encode([
            'referralLinkId' => $link->getId(),
            'trainerId' => $trainer->getId(),
            'clickedAt' => (new \DateTimeImmutable('-31 days'))->format(\DATE_ATOM),
        ], \JSON_THROW_ON_ERROR));

        /** @var ReferralAttributionService $attribution */
        $attribution = self::getContainer()->get(ReferralAttributionService::class);
        $result = $attribution->completeAttribution($request, $trainer, $referee);

        self::assertNull($result, 'AC-06-6: a click older than the attribution window gives no credit.');

        /** @var ReferralRepository $referrals */
        $referrals = self::getContainer()->get(ReferralRepository::class);
        self::assertNull($referrals->findOneByRefereePlayer($referee));
    }

    /**
     * AC-06-6: a click WITHIN the window still attributes correctly —
     * the positive control for the expiry test above.
     */
    public function testAttributionSucceedsWithinTheConfiguredWindow(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->setReferralAttributionWindowDays(30);

        $referrer = $this->freshPlayerMembership($trainer, 'Within Window Referrer')->getPlayer();
        $link = $this->createReferralLink($trainer, $referrer);
        $referee = $this->freshPlayerMembership($trainer, 'Within Window Referee')->getPlayer();

        $request = Request::create('/join/'.$trainer->getSlug().'/register');
        $request->cookies->set(ReferralAttributionService::COOKIE_NAME, json_encode([
            'referralLinkId' => $link->getId(),
            'trainerId' => $trainer->getId(),
            'clickedAt' => (new \DateTimeImmutable('-29 days'))->format(\DATE_ATOM),
        ], \JSON_THROW_ON_ERROR));

        /** @var ReferralAttributionService $attribution */
        $attribution = self::getContainer()->get(ReferralAttributionService::class);
        $result = $attribution->completeAttribution($request, $trainer, $referee);

        self::assertInstanceOf(Referral::class, $result);
        self::assertTrue($result->isPending());
    }

    /**
     * AC-06-7/BR-06-3: registering with a DIFFERENT trainer than the one
     * whose link was clicked gives no referral credit.
     */
    public function testRegisteringWithADifferentTrainerGivesNoCredit(): void
    {
        $trainerA = $this->trainer('peak-performance');
        $trainerB = $this->trainer('baseline-athletics');

        $this->activateTenant($trainerA);
        $referrer = $this->freshPlayerMembership($trainerA, 'Cross Trainer Referrer')->getPlayer();
        $link = $this->createReferralLink($trainerA, $referrer);

        $this->activateTenant($trainerB);
        $refereeUnderB = $this->freshPlayerMembership($trainerB, 'Cross Trainer Referee')->getPlayer();

        $request = Request::create('/join/baseline-athletics/register');
        $request->cookies->set(ReferralAttributionService::COOKIE_NAME, json_encode([
            'referralLinkId' => $link->getId(),
            'trainerId' => $trainerA->getId(),
            'clickedAt' => (new \DateTimeImmutable('-1 day'))->format(\DATE_ATOM),
        ], \JSON_THROW_ON_ERROR));

        /** @var ReferralAttributionService $attribution */
        $attribution = self::getContainer()->get(ReferralAttributionService::class);
        // Registered trainer (B) differs from the cookie's trainer (A).
        $result = $attribution->completeAttribution($request, $trainerB, $refereeUnderB);

        self::assertNull($result, 'AC-06-7: registering with a different trainer than the clicked link gives no credit.');
    }

    /**
     * BR-06-3: a player cannot refer themselves — enforced structurally by
     * `Referral`'s own constructor guard (the database CHECK is the
     * authoritative backstop under concurrency).
     */
    public function testReferralEntityBlocksSelfReferral(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $player = $this->freshPlayerMembership($trainer, 'Self Referrer')->getPlayer();
        $link = $this->createReferralLink($trainer, $player);

        $this->expectException(\InvalidArgumentException::class);
        $this->expectExceptionMessage('BR-06-3');

        new Referral($trainer, $link, $player, $player, new \DateTimeImmutable(), new \DateTimeImmutable());
    }

    /**
     * AC-06-7: if the friend already has an account, the link still works
     * (it resolves and redirects normally) but no referral credit is
     * given — an existing account never goes through
     * `PlayerRegistrationService::registerViaShareLink()`, so
     * `PlayerRegistered` never fires and no Referral row is ever created
     * for them.
     */
    public function testFriendWhoAlreadyHasAnAccountGetsNoReferralCredit(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        // A brand-new, isolated referrer — 'peak-performance' itself is
        // shared across this whole suite, but a referrer created fresh
        // right here can only ever be credited by THIS test's own action,
        // so `statsForReferrer` scoped to this one referrer is immune to
        // whatever other tests do with the same trainer.
        $referrer = $this->freshPlayerMembership($trainer, 'Existing Account Referrer')->getPlayer();

        $this->client->request('GET', sprintf('/join/%s/%d', $trainer->getSlug(), $referrer->getId()));
        self::assertResponseRedirects(null, 302, 'AC-06-7: the link still works (resolves and redirects normally) for a friend who already has an account.');
        $registrationPageUrl = (string) $this->client->getResponse()->headers->get('Location');

        // The "friend" is actually an already-registered account. Whatever
        // that account can or cannot do next (blocked as a child,
        // redirected to associate, etc.) is Identity's own concern —
        // structurally, `PlayerRegistrationService::registerViaShareLink()`
        // (the only thing that ever dispatches `PlayerRegistered`) is never
        // reached for an account that already exists, so no Referral row
        // can ever be created for them.
        $coach = $this->account('coach@practiceperfect.test');
        $this->client->loginUser($coach);
        $this->client->request('GET', $registrationPageUrl);

        // Re-fetched fresh: KernelBrowser reboots the kernel on every
        // request, so a repository obtained before the requests above
        // would be bound to a discarded container.
        $this->activateTenant($trainer);
        /** @var ReferralRepository $referrals */
        $referrals = self::getContainer()->get(ReferralRepository::class);
        self::assertSame(0, $referrals->statsForReferrer($trainer, $referrer)['total'], 'AC-06-7: an existing account visiting the link creates no referral.');
    }
}
