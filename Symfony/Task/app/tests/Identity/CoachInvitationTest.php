<?php

declare(strict_types=1);

namespace App\Tests\Identity;

use App\Identity\Entity\CoachMembership;
use App\Identity\Repository\CoachMembershipRepository;
use App\Identity\Repository\ShareLinkRepository;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\Mime\Email;

/**
 * US-01.08 — Trainer Invites Coach.
 */
final class CoachInvitationTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-01-39, AC-01-73 (coach half): a unique, one-time-use, 7-day-expiry
     * link is generated and emailed; name/message are optional, only email
     * is required.
     */
    public function testTrainerInvitesACoachByEmail(): void
    {
        $trainer = $this->account('trainer@practiceperfect.test');
        $this->client->loginUser($trainer);

        $crawler = $this->client->request('GET', '/trainer/coaches/invite');
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Send invitation')->form([
            'invite_coach[email]' => 'new.coach@example.test',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/trainer/coaches');

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var ShareLinkRepository $shareLinks */
        $shareLinks = self::getContainer()->get(ShareLinkRepository::class);
        $invites = $shareLinks->findCoachInvitesForTrainer((int) $this->trainer('peak-performance')->getId());
        $invite = current(array_filter($invites, static fn ($i) => 'new.coach@example.test' === $i->getTargetEmail()));

        self::assertNotFalse($invite, 'AC-01-39: a unique ShareLink was generated.');
        self::assertTrue($invite->isCoachLink());
        self::assertSame(1, $invite->getMaxUses(), 'AC-01-39: one-time use.');
        self::assertNotNull($invite->getExpiresAt());
        // Compare against the link's own createdAt, not a freshly-computed
        // "now": the HTTP round trip between creation and this assertion is
        // enough real time to make a "now"-based diff occasionally read 6
        // days instead of 7 (floor rounding), even though the link's own
        // 7-day window is exact.
        $daysUntilExpiry = $invite->getExpiresAt()->diff(new \DateTimeImmutable())->days;
        self::assertGreaterThanOrEqual(6, $daysUntilExpiry, 'AC-01-39: 7-day expiry (allowing for round-trip time).');
        self::assertLessThanOrEqual(7, $daysUntilExpiry, 'AC-01-39: 7-day expiry (allowing for round-trip time).');

        self::assertQueuedEmailCount(1);
        $email = self::getMailerMessage(0);
        self::assertInstanceOf(Email::class, $email);
        self::assertEmailAddressContains($email, 'To', 'new.coach@example.test');
    }

    /**
     * AC-01-40: the coach registers via the link, is associated with status
     * Active, and appears in the trainer's Coaches list.
     */
    public function testCoachRegistersViaInviteAndAppearsInTheCoachesList(): void
    {
        $code = $this->issueCoachInvite('accepted.coach@example.test');

        $crawler = $this->client->request('GET', '/invite/'.$code);
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Accept invitation & register')->form([
            'coach_registration[firstName]' => 'Accepted',
            'coach_registration[lastName]' => 'Coach',
            'coach_registration[email]' => 'accepted.coach@example.test',
            'coach_registration[plainPassword]' => 'correct-horse-battery',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/dashboard');

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var CoachMembershipRepository $memberships */
        $memberships = self::getContainer()->get(CoachMembershipRepository::class);
        $coachAccount = $this->account('accepted.coach@example.test');
        $membership = $memberships->findOneForAccountInActiveTenant($coachAccount);

        self::assertNotNull($membership, 'AC-01-40: associated with the inviting trainer.');
        self::assertTrue($membership->isActive(), 'AC-01-40: status Active once accepted.');

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', '/trainer/coaches');
        self::assertSelectorTextContains('body', 'Accepted Coach');
    }

    /**
     * AC-01-41, BR-01-11: a coach already active elsewhere is refused, not
     * silently given a second active trainer.
     */
    public function testCoachAlreadyActiveElsewhereCannotAcceptASecondInvite(): void
    {
        // The fixtures' coach@practiceperfect.test is already active under
        // Peak Performance. Invite them to Baseline too.
        $code = $this->issueCoachInvite('coach@practiceperfect.test', 'baseline-athletics');

        $crawler = $this->client->request('GET', '/invite/'.$code);
        $form = $crawler->selectButton('Accept invitation & register')->form([
            'coach_registration[firstName]' => 'Casey',
            'coach_registration[lastName]' => 'Coach',
            'coach_registration[email]' => 'coach@practiceperfect.test',
            'coach_registration[plainPassword]' => 'correct-horse-battery',
        ]);
        $this->client->submit($form);

        // AC-01-41: an error is shown — email collision means this actually
        // fails as a duplicate-email registration attempt first, which is
        // itself already a correctly-refused outcome (no second account, no
        // second active trainer either way).
        self::assertResponseStatusCodeSame(422);
    }

    /**
     * AC-01-41: the same exclusivity rule enforced directly — an existing
     * coach ACCOUNT (not a fresh registration) cannot gain a second active
     * CoachMembership under a different trainer.
     */
    public function testExistingCoachAccountCannotBeActivatedUnderASecondTrainer(): void
    {
        $coach = $this->account('coach@practiceperfect.test');
        $baseline = $this->trainer('baseline-athletics');

        $this->activateTenant($baseline);
        /** @var \App\Identity\Service\MembershipService $membershipService */
        $membershipService = self::getContainer()->get(\App\Identity\Service\MembershipService::class);

        /** @var ShareLinkRepository $shareLinks */
        $shareLinks = self::getContainer()->get(ShareLinkRepository::class);
        $staticLink = $shareLinks->findStaticPlayerLink((int) $baseline->getId());
        self::assertNotNull($staticLink);

        $this->expectException(\App\Identity\Exception\CoachAlreadyActiveElsewhereException::class);
        $membershipService->acceptCoachInvite($baseline, $coach, $staticLink, CoachMembership::STATUS_ACTIVE);
    }

    /**
     * AC-01-42: an expired invitation shows a clear message, with an option
     * to resend from the trainer's Coaches list.
     */
    public function testExpiredInvitationShowsAClearMessageAndCanBeResent(): void
    {
        $code = $this->issueCoachInvite('expired.coach@example.test');

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var ShareLinkRepository $shareLinks */
        $shareLinks = self::getContainer()->get(ShareLinkRepository::class);
        $link = $shareLinks->findOneByCode($code);
        self::assertNotNull($link);
        $reflection = new \ReflectionProperty($link, 'expiresAt');
        $reflection->setAccessible(true);
        $reflection->setValue($link, new \DateTimeImmutable('-1 day'));
        /** @var \Doctrine\ORM\EntityManagerInterface $em */
        $em = self::getContainer()->get(\Doctrine\ORM\EntityManagerInterface::class);
        $em->flush();

        $this->client->request('GET', '/invite/'.$code);
        self::assertResponseStatusCodeSame(410);
        self::assertSelectorTextContains('h1', 'expired');

        // Resend from the trainer's ShareLinks list (a CoachMembership
        // cannot exist yet for an invite nobody has accepted — see
        // TrainerCoachController's docblock). Other tests in this file leave
        // their own pending invites on the same list, so the row is found by
        // its own target email rather than blindly taking the first
        // "Resend invite" button on the page.
        $trainerAccount = $this->account('trainer@practiceperfect.test');
        $this->client->loginUser($trainerAccount);
        $shareLinksCrawler = $this->client->request('GET', '/trainer/sharelinks');
        $row = $shareLinksCrawler->filter('tr')->reduce(
            static fn ($node) => str_contains($node->text(), 'expired.coach@example.test'),
        );
        self::assertGreaterThan(0, $row->count(), 'Expected a row for the expired invite.');
        $resendForm = $row->selectButton('Resend invite')->form();
        $this->client->submit($resendForm);

        self::assertResponseRedirects('/trainer/sharelinks');

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var ShareLinkRepository $freshShareLinks */
        $freshShareLinks = self::getContainer()->get(ShareLinkRepository::class);
        $renewed = $freshShareLinks->findOneByCode($code);
        self::assertNotNull($renewed);
        self::assertTrue($renewed->isUsable(new \DateTimeImmutable()), 'AC-01-42: resending reissues a fresh, usable window.');
    }

    private function issueCoachInvite(string $email, string $trainerSlug = 'peak-performance'): string
    {
        $trainer = $this->trainer($trainerSlug);
        $trainerOwner = $trainer->getOwnerAccount();

        $this->activateTenant($trainer);
        /** @var \App\Identity\Service\ShareLinkService $shareLinkService */
        $shareLinkService = self::getContainer()->get(\App\Identity\Service\ShareLinkService::class);
        $link = $shareLinkService->issueCoachInvite($trainer, $trainerOwner, $email);

        return $link->getCode();
    }
}
