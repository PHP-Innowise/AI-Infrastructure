<?php

declare(strict_types=1);

namespace App\Tests\Identity;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountProfile;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\ParentChildLink;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Identity\Repository\ParentChildLinkRepository;
use App\Identity\Service\MembershipService;
use App\Identity\Voter\ChildProfileVoter;
use App\Tests\Support\FixtureHelpers;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\Mime\Email;
use Symfony\Component\PasswordHasher\Hasher\UserPasswordHasherInterface;
use Symfony\Component\Security\Core\Authorization\AuthorizationCheckerInterface;

/**
 * US-01.06 — Child Login with Constraints.
 */
final class ChildLoginConstraintsTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-01-29: a child can update their own basic profile info.
     */
    public function testChildCanUpdateBasicProfileInfo(): void
    {
        $child = $this->createChildWithOwnLogin('profile-child@example.test');
        $this->client->loginUser($child);

        $crawler = $this->client->request('GET', '/account/profile');
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Save changes')->form([
            'edit_profile[firstName]' => 'Chris',
            'edit_profile[lastName]' => 'Child',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();
    }

    /**
     * AC-01-30: a child cannot add new trainers (ShareLink registration is
     * blocked) — ChildProfileVoter denies CHILD_TRAINER_ADD outright.
     */
    public function testChildCannotAddATrainer(): void
    {
        $child = $this->createChildWithOwnLogin('blocked-add@example.test');
        $this->client->loginUser($child);

        /** @var ParentChildLinkRepository $links */
        $links = self::getContainer()->get(ParentChildLinkRepository::class);
        $link = $links->findByChildAccount($child);
        self::assertNotNull($link);
        $childPlayerId = $link->getChildPlayer()->getId();

        $this->client->request('GET', sprintf('/portal/family/children/%d/trainers/add', $childPlayerId));

        self::assertResponseStatusCodeSame(403);
    }

    /**
     * AC-01-30: a child cannot manage payment methods, purchase tokens, or
     * delete their account — none of these routes exist for a child to
     * reach in this codebase at all (no self-service account deletion route
     * exists for ANY role, and no payment/token routes exist yet — Epic-05).
     * What Epic-01 owns and this asserts: ChildProfileVoter denies every one
     * of its attributes for a child login, which is the mechanism AC-01-30's
     * "cannot... change trainer associations" runs on.
     */
    public function testChildProfileVoterDeniesEveryAttributeForAChildLogin(): void
    {
        $child = $this->createChildWithOwnLogin('denied-everywhere@example.test');

        /** @var ParentChildLinkRepository $links */
        $links = self::getContainer()->get(ParentChildLinkRepository::class);
        $link = $links->findByChildAccount($child);
        self::assertNotNull($link);

        // No HTTP request in between: loginUser() sets the token directly in
        // this same (not-yet-rebooted) container, so the authorization
        // checker below sees it without needing a round trip.
        $this->client->loginUser($child);
        /** @var AuthorizationCheckerInterface $authChecker */
        $authChecker = self::getContainer()->get(AuthorizationCheckerInterface::class);

        self::assertFalse($authChecker->isGranted(ChildProfileVoter::CHILD_PROFILE_CREATE));
        self::assertFalse($authChecker->isGranted(ChildProfileVoter::CHILD_TRAINER_ADD, $link->getChildPlayer()));
        self::assertFalse($authChecker->isGranted(ChildProfileVoter::CHILD_TRAINER_REMOVE, $link->getChildPlayer()));
        self::assertFalse($authChecker->isGranted(ChildProfileVoter::CHILD_TOKEN_APPROVAL_EDIT, $link->getChildPlayer()));
    }

    /**
     * AC-01-31: a logged-in child clicking a ShareLink is blocked and told
     * to ask their parent; the parent is emailed a "Review Registration" CTA;
     * no association happens.
     */
    public function testLoggedInChildClickingAShareLinkIsBlockedAndParentIsEmailed(): void
    {
        $child = $this->createChildWithOwnLogin('blocked-sharelink@example.test');
        $this->client->loginUser($child);

        $this->client->request('GET', '/join/join-baseline-athletics');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('h1', 'Ask your parent');

        self::assertQueuedEmailCount(1);
        $email = self::getMailerMessage(0);
        self::assertInstanceOf(Email::class, $email);
        self::assertEmailAddressContains($email, 'To', 'player@practiceperfect.test');
        self::assertEmailSubjectContains($email, 'wants to train with');

        // No association happened.
        /** @var ParentChildLinkRepository $links */
        $links = self::getContainer()->get(ParentChildLinkRepository::class);
        $link = $links->findByChildAccount($child);
        self::assertNotNull($link);
        $this->activateTenant($this->trainer('baseline-athletics'));
        /** @var \App\Identity\Repository\PlayerTrainerMembershipRepository $memberships */
        $memberships = self::getContainer()->get(\App\Identity\Repository\PlayerTrainerMembershipRepository::class);
        $membership = $memberships->findOneByTrainerAndPlayer($this->trainer('baseline-athletics'), $link->getChildPlayer());
        self::assertTrue(null === $membership || !$membership->isActive(), 'AC-01-31: no association until the parent completes it.');
    }

    /**
     * AC-01-32: a multi-trainer child's context selector lists only their
     * own trainer contexts — no parent data, no "Me" section.
     */
    public function testChildContextSelectorShowsOnlyTheirOwnTrainersNoParentData(): void
    {
        $child = $this->createChildWithOwnLogin('multi-trainer-child@example.test');

        // Associate the child's own player profile with a second trainer.
        /** @var ParentChildLinkRepository $links */
        $links = self::getContainer()->get(ParentChildLinkRepository::class);
        $link = $links->findByChildAccount($child);
        self::assertNotNull($link);

        $this->activateTenant($this->trainer('baseline-athletics'));
        /** @var MembershipService $membershipService */
        $membershipService = self::getContainer()->get(MembershipService::class);
        $membershipService->associatePlayer($this->trainer('baseline-athletics'), $link->getChildPlayer(), PlayerTrainerMembership::SOURCE_COACH_INVITE);

        $this->client->loginUser($child);
        $crawler = $this->client->request('GET', '/dashboard');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Peak Performance Basketball');
        self::assertSelectorTextContains('body', 'Baseline Athletics');
        // No parent identity ever appears on the child's own dashboard.
        self::assertStringNotContainsString('player@practiceperfect.test', (string) $crawler->filter('body')->text());
    }

    private function createChildWithOwnLogin(string $email): Account
    {
        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);
        /** @var UserPasswordHasherInterface $hasher */
        $hasher = self::getContainer()->get(UserPasswordHasherInterface::class);

        $parent = $this->account('player@practiceperfect.test');

        $childAccount = new Account($email, '', AccountRole::Player);
        $childAccount->changePasswordHash($hasher->hashPassword($childAccount, 'password'));
        $childAccount->verifyEmail();
        $em->persist($childAccount);
        $em->persist(new AccountProfile($childAccount, 'Test', 'Child', null));

        // A dedicated child profile per call — reusing the fixture's shared
        // "Alex" across several test methods would have one test's own-login
        // attachment collide with another's "starts with no login" assumption.
        $childPlayer = new \App\Identity\Entity\PlayerProfile('Test', new \DateTimeImmutable('2013-01-01'), $childAccount);
        $em->persist($childPlayer);
        $em->flush();

        $link = new ParentChildLink($parent, $childPlayer, $childAccount);
        $em->persist($link);
        $em->flush();

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var MembershipService $membershipService */
        $membershipService = self::getContainer()->get(MembershipService::class);
        $membershipService->associatePlayer($this->trainer('peak-performance'), $childPlayer, PlayerTrainerMembership::SOURCE_SHARELINK);

        return $childAccount;
    }
}
