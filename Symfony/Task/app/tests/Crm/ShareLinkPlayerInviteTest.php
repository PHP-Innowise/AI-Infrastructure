<?php

declare(strict_types=1);

namespace App\Tests\Crm;

use App\Crm\Voter\PlayerVoter;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Identity\Repository\ShareLinkRepository;
use App\Tests\Support\CrmFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\Security\Core\Authorization\AuthorizationCheckerInterface;

/**
 * US-03.11 — Coach Invites Player via ShareLink.
 */
final class ShareLinkPlayerInviteTest extends WebTestCase
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
     * AC-03-50: clicking "Invite Player" generates a unique ShareLink in the
     * `/invite/[unique-code]` format.
     */
    public function testCoachGeneratesAUniquePlayerInviteLink(): void
    {
        $trainer = $this->trainer('peak-performance');
        $coachAccount = $this->createCoach($trainer, 'inviter.coach.'.uniqid().'@example.test');

        $this->client->loginUser($coachAccount);
        $crawler = $this->client->request('GET', '/coach/players/invite');
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Generate invite link')->form([
            'invite_player[email]' => 'invited.player@example.test',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/coach/players/invite');
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', 'invite/');

        $this->activateTenant($trainer);
        /** @var ShareLinkRepository $shareLinks */
        $shareLinks = self::getContainer()->get(ShareLinkRepository::class);
        $invites = $shareLinks->findCoachPlayerInvitesCreatedBy($coachAccount);
        self::assertCount(1, $invites);
        self::assertTrue($invites[0]->isCoachPlayerInviteLink());
        self::assertSame(1, $invites[0]->getMaxUses());
    }

    /**
     * AC-03-51: an unauthenticated player clicking the link is redirected
     * to register/log in; once logged in, is associated with the trainer's
     * organization and appears in the trainer's CRM.
     */
    public function testUnauthenticatedPlayerRegistersViaTheCoachInviteAndAppearsInTheCrm(): void
    {
        $trainer = $this->trainer('peak-performance');
        $coachAccount = $this->createCoach($trainer, 'reg.coach.'.uniqid().'@example.test');
        $this->activateTenant($trainer);
        /** @var \App\Identity\Service\ShareLinkService $shareLinkService */
        $shareLinkService = self::getContainer()->get(\App\Identity\Service\ShareLinkService::class);
        $link = $shareLinkService->issueCoachPlayerInvite($trainer, $coachAccount, null);

        $crawler = $this->client->request('GET', '/invite/'.$link->getCode());
        self::assertResponseIsSuccessful();
        self::assertSelectorExists('form');

        $email = 'newly.invited.'.uniqid().'@example.test';
        $form = $crawler->selectButton('Register')->form([
            'player_registration[accountFirstName]' => 'Newly',
            'player_registration[accountLastName]' => 'Invited',
            'player_registration[email]' => $email,
            'player_registration[plainPassword]' => 'correct-horse-battery',
            'player_registration[playerFirstName]' => 'Newly',
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
        self::assertNotNull($membership, 'AC-03-51: associated with the trainer\'s organization.');
        self::assertTrue($membership->isActive());
    }

    /**
     * AC-03-52: opens are tracked, and the coach can view their sent
     * invitations with status (pending, accepted).
     */
    public function testCoachSeesTheirInvitationOpenAndAcceptedStatus(): void
    {
        $trainer = $this->trainer('peak-performance');
        $coachAccount = $this->createCoach($trainer, 'status.coach.'.uniqid().'@example.test');
        $this->activateTenant($trainer);
        /** @var \App\Identity\Service\ShareLinkService $shareLinkService */
        $shareLinkService = self::getContainer()->get(\App\Identity\Service\ShareLinkService::class);
        $link = $shareLinkService->issueCoachPlayerInvite($trainer, $coachAccount, 'tracked@example.test');

        // Open (click), tracked.
        $this->client->request('GET', '/invite/'.$link->getCode());

        $this->client->loginUser($coachAccount);
        $crawler = $this->client->request('GET', '/coach/players/invite');
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Pending');

        $this->activateTenant($trainer);
        /** @var \App\Identity\Repository\ShareLinkOpenRepository $opens */
        $opens = self::getContainer()->get(\App\Identity\Repository\ShareLinkOpenRepository::class);
        self::assertGreaterThanOrEqual(1, $opens->countFor((int) $link->getId()), 'AC-03-52: opens are tracked.');
        unset($crawler);

        // The test client reboots the kernel on each request, so both
        // $link and $shareLinkService (fetched before any request ran) are
        // bound to a now-discarded EntityManager — re-fetch fresh from the
        // container/repository rather than reusing either stale reference,
        // the same pattern CoachInvitationTest's own resend test uses.
        /** @var \App\Identity\Repository\ShareLinkRepository $shareLinks */
        $shareLinks = self::getContainer()->get(\App\Identity\Repository\ShareLinkRepository::class);
        $freshLink = $shareLinks->findOneByCode($link->getCode());
        self::assertNotNull($freshLink);
        /** @var \App\Identity\Service\ShareLinkService $freshShareLinkService */
        $freshShareLinkService = self::getContainer()->get(\App\Identity\Service\ShareLinkService::class);
        $freshShareLinkService->recordUse($freshLink);

        $this->client->request('GET', '/coach/players/invite');
        self::assertSelectorTextContains('body', 'Accepted');
    }

    /**
     * AC-03-53: the coach cannot apply labels or flags to a player they
     * invite, and can only view them once they appear in the coach's own
     * assigned sessions.
     */
    public function testCoachCannotManageAnInvitedPlayerUntilTheyShareASession(): void
    {
        $trainer = $this->trainer('peak-performance');
        $coachAccount = $this->createCoach($trainer, 'limits.coach.'.uniqid().'@example.test');
        $this->activateTenant($trainer);

        /** @var \App\Identity\Service\MembershipService $memberships */
        $memberships = self::getContainer()->get(\App\Identity\Service\MembershipService::class);
        $invitedPlayer = new \App\Identity\Entity\PlayerProfile('InvitedNoSession'.uniqid(), new \DateTimeImmutable('-15 years'));
        /** @var \Doctrine\ORM\EntityManagerInterface $em */
        $em = self::getContainer()->get(\Doctrine\ORM\EntityManagerInterface::class);
        $em->persist($invitedPlayer);
        $em->flush();
        $membership = $memberships->associatePlayer($trainer, $invitedPlayer, PlayerTrainerMembership::SOURCE_SHARELINK);

        $this->client->loginUser($coachAccount);
        /** @var AuthorizationCheckerInterface $authChecker */
        $authChecker = self::getContainer()->get(AuthorizationCheckerInterface::class);

        self::assertFalse(
            $authChecker->isGranted(PlayerVoter::PLAYER_VIEW, $membership),
            'AC-03-53: not viewable until the player appears in the coach\'s own assigned sessions.',
        );
        self::assertFalse(
            $authChecker->isGranted(PlayerVoter::PLAYER_LABEL_MANAGE, $membership),
            'AC-03-53: the inviting coach can never apply labels (trainer-only).',
        );
    }
}
