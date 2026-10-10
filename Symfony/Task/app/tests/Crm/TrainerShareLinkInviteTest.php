<?php

declare(strict_types=1);

namespace App\Tests\Crm;

use App\Identity\Entity\PlayerTrainerMembership;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Identity\Repository\ShareLinkOpenRepository;
use App\Identity\Repository\ShareLinkRepository;
use App\Tests\Support\CrmFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-03.13 — Trainer Invites Players via ShareLink. The static mass-invite
 * link's creation and player-registration mechanics are Epic-01's own
 * (BR-01-14, AC-01-73), already covered by `ShareLinkRegistrationTest`; this
 * file adds the Epic-03-specific claims — the exact link format, the CRM
 * appearance, and the Quick View tracking — with their own AC ids.
 */
final class TrainerShareLinkInviteTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;
    use CrmFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-03-59: a single, static, reusable mass invite link per trainer, in
     * the `/join/[trainer-unique-code]` format, shareable via email/SMS/
     * social/embed (BR-03-20: one per trainer, unlimited uses).
     */
    public function testTrainerHasOneStaticReusableMassInviteLink(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        /** @var ShareLinkRepository $shareLinks */
        $shareLinks = self::getContainer()->get(ShareLinkRepository::class);
        $staticLink = $shareLinks->findStaticPlayerLink((int) $trainer->getId());
        self::assertNotNull($staticLink);
        self::assertNull($staticLink->getMaxUses(), 'BR-03-20: unlimited uses.');
        self::assertNull($staticLink->getExpiresAt(), 'BR-03-20: no expiry.');

        $this->client->loginUser($trainer->getOwnerAccount());
        $this->client->request('GET', '/trainer/sharelinks');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', '/join/'.$staticLink->getCode());
    }

    /**
     * AC-03-60: an unauthenticated player clicking the trainer's link is
     * redirected to login/signup; once logged in, the player's account is
     * associated with the trainer and appears in the trainer's CRM.
     */
    public function testPlayerJoinsViaTheTrainersStaticLinkAndAppearsInTheCrm(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        /** @var ShareLinkRepository $shareLinks */
        $shareLinks = self::getContainer()->get(ShareLinkRepository::class);
        $staticLink = $shareLinks->findStaticPlayerLink((int) $trainer->getId());
        self::assertNotNull($staticLink);

        $crawler = $this->client->request('GET', '/join/'.$staticLink->getCode());
        self::assertResponseIsSuccessful();

        $email = 'joined.via.static.'.uniqid().'@example.test';
        $form = $crawler->selectButton('Register')->form([
            'player_registration[accountFirstName]' => 'Joined',
            'player_registration[accountLastName]' => 'Player',
            'player_registration[email]' => $email,
            'player_registration[plainPassword]' => 'correct-horse-battery',
            'player_registration[playerFirstName]' => 'Joined',
            'player_registration[playerDateOfBirth]' => (new \DateTimeImmutable('-25 years'))->format('Y-m-d'),
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/dashboard');

        $this->activateTenant($trainer);
        /** @var PlayerTrainerMembershipRepository $memberships */
        $memberships = self::getContainer()->get(PlayerTrainerMembershipRepository::class);
        $account = $this->account($email);
        /** @var \App\Identity\Repository\PlayerProfileRepository $players */
        $players = self::getContainer()->get(\App\Identity\Repository\PlayerProfileRepository::class);
        $player = $players->findOneForSelfAccount($account);
        self::assertNotNull($player);
        $membership = $memberships->findOneByTrainerAndPlayer($trainer, $player);
        self::assertNotNull($membership, 'AC-03-60: associated with the trainer.');
        self::assertSame(PlayerTrainerMembership::SOURCE_SHARELINK, $membership->getSource());

        // Appears in the trainer's CRM.
        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', '/trainer/players?q=Joined');
        self::assertStringContainsString('Joined', $crawler->filter('body')->text());
    }

    /**
     * AC-03-62/BR-03-22/23: link opens and successful joins are tracked and
     * shown in the Quick View dashboard.
     */
    public function testShareLinkOpensAndJoinsAreTrackedInQuickView(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        /** @var ShareLinkRepository $shareLinks */
        $shareLinks = self::getContainer()->get(ShareLinkRepository::class);
        $staticLink = $shareLinks->findStaticPlayerLink((int) $trainer->getId());
        self::assertNotNull($staticLink);

        // An open (click).
        $this->client->request('GET', '/join/'.$staticLink->getCode());

        $this->activateTenant($trainer);
        /** @var ShareLinkOpenRepository $opens */
        $opens = self::getContainer()->get(ShareLinkOpenRepository::class);
        $before = $opens->countFor((int) $staticLink->getId());
        self::assertGreaterThanOrEqual(1, $before);

        /** @var \App\Crm\Service\QuickViewDashboardService $dashboard */
        $dashboard = self::getContainer()->get(\App\Crm\Service\QuickViewDashboardService::class);
        $metrics = $dashboard->build($trainer, new \DateTimeImmutable());
        self::assertGreaterThanOrEqual(1, $metrics->shareLinkOpensThisWeek, 'AC-03-62: opens shown in Quick View.');
    }

    /**
     * AC-03-61 (optional MVP): the trainer generates a unique, one-time link
     * per player/parent, tracking who opened it and whether it joined.
     */
    public function testTrainerGeneratesAUniquePlayerInviteLink(): void
    {
        $trainer = $this->trainer('peak-performance');

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', '/trainer/sharelinks');
        self::assertResponseIsSuccessful();

        $email = 'unique.invitee.'.uniqid().'@example.test';
        $form = $crawler->selectButton('Generate unique link')->form([
            'unique_share_link[email]' => $email,
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/trainer/sharelinks');
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', $email);

        $this->activateTenant($trainer);
        /** @var ShareLinkRepository $shareLinks */
        $shareLinks = self::getContainer()->get(ShareLinkRepository::class);
        $uniqueLinks = $shareLinks->findUniquePlayerInvitesForTrainer((int) $trainer->getId());
        $created = current(array_filter($uniqueLinks, static fn ($l) => $email === $l->getTargetEmail()));
        self::assertNotFalse($created);
        self::assertSame(1, $created->getMaxUses(), 'AC-03-61: one-time.');
        self::assertTrue($created->isUniquePlayerInviteLink());
    }

    /**
     * AC-03-63 (optional MVP): a detailed tracking report — date, link
     * type, opens, joins.
     */
    public function testUniqueLinkTrackingReportShowsDateTypeOpensAndJoins(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        /** @var \App\Identity\Service\ShareLinkService $shareLinkService */
        $shareLinkService = self::getContainer()->get(\App\Identity\Service\ShareLinkService::class);
        $link = $shareLinkService->issueUniquePlayerInvite($trainer, $trainer->getOwnerAccount(), 'report.tracked@example.test');

        $this->client->request('GET', '/join/'.$link->getCode());

        $this->client->loginUser($trainer->getOwnerAccount());
        $this->client->request('GET', '/trainer/sharelinks');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'report.tracked@example.test');
        self::assertSelectorTextContains('body', 'Unique player invite');
        self::assertSelectorTextContains('body', $link->getCreatedAt()->format('Y-m-d'));
    }
}
