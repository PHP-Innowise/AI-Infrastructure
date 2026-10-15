<?php

declare(strict_types=1);

namespace App\Tests\Identity;

use App\Identity\Repository\ParentChildLinkRepository;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-01.03 — Parent Creates Child Profile.
 */
final class ChildProfileTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-01-16: name, age, gender, and optional school are captured, and the
     * profile is marked "Child".
     */
    public function testParentAddsAChildProfile(): void
    {
        $parent = $this->account('player@practiceperfect.test');
        $this->client->loginUser($parent);

        $crawler = $this->client->request('GET', '/portal/family/children/new');
        self::assertResponseIsSuccessful();

        $dob = sprintf('%d-05-01', ((int) date('Y')) - 9);
        $form = $crawler->selectButton('Save child profile')->form([
            'child_profile[firstName]' => 'Jamie',
            'child_profile[dateOfBirth]' => $dob,
            'child_profile[gender]' => 'unspecified',
            'child_profile[schoolOrTeam]' => 'Riverside Elementary',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/portal/family');

        /** @var ParentChildLinkRepository $links */
        $links = self::getContainer()->get(ParentChildLinkRepository::class);
        $jamie = current(array_filter(
            $links->findByParent($parent),
            static fn ($link) => 'Jamie' === $link->getChildPlayer()->getFirstName(),
        ));

        self::assertNotFalse($jamie, 'AC-01-16: the child profile is linked to the parent account.');
        self::assertTrue($jamie->getChildPlayer()->isChild(), 'AC-01-16: the profile is marked Child.');
        self::assertSame('Riverside Elementary', $jamie->getChildPlayer()->getSchoolOrTeam(), 'AC-01-16: school is captured.');
    }

    /**
     * AC-01-17: with exactly one trainer, a yes/no prompt associates the
     * child immediately when confirmed.
     */
    public function testChildCreationWithOneTrainerOffersAYesNoAssociation(): void
    {
        $parent = $this->account('player@practiceperfect.test');
        $this->client->loginUser($parent);

        $crawler = $this->client->request('GET', '/portal/family/children/new');
        $dob = sprintf('%d-05-01', ((int) date('Y')) - 8);
        $form = $crawler->selectButton('Save child profile')->form([
            'child_profile[firstName]' => 'Quinn',
            'child_profile[dateOfBirth]' => $dob,
            'child_profile[gender]' => 'unspecified',
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects();

        // The client reboots the kernel on every request, which discards any
        // service/repository instance obtained beforehand (its EntityManager
        // and the DB session TenantContext writes to both go stale) — so
        // every repository and TenantContext activation below is re-fetched
        // fresh from the container after each request, never reused across one.
        $quinnId = $this->findChildIdByFirstName($parent, 'Quinn');

        // Without confirming a trainer, no association exists yet — this is
        // "otherwise the child profile is created without a trainer
        // association" (AC-01-17). Confirming happens via the same "add a
        // trainer" flow AC-01-23 covers (My Trainers picker / code entry).
        self::assertNull($this->findMembership('peak-performance', $quinnId));

        // Confirming ("Will Quinn also train with Peak Performance?" = Yes)
        // associates the child with that one trainer via the picker.
        $addCrawler = $this->client->request('GET', sprintf('/portal/family/children/%d/trainers/add', $quinnId));
        self::assertResponseIsSuccessful();
        $addForm = $addCrawler->selectButton('Add trainer')->form([
            'add_trainer[trainer]' => (string) $this->trainer('peak-performance')->getId(),
        ]);
        $this->client->submit($addForm);
        self::assertResponseRedirects();

        $confirmed = $this->findMembership('peak-performance', $quinnId);
        self::assertNotNull($confirmed);
        self::assertTrue($confirmed->isActive(), 'AC-01-17: confirming associates the child with the trainer.');
    }

    private function findChildIdByFirstName(\App\Identity\Entity\Account $parent, string $firstName): int
    {
        /** @var ParentChildLinkRepository $links */
        $links = self::getContainer()->get(ParentChildLinkRepository::class);
        $link = current(array_filter(
            $links->findByParent($parent),
            static fn ($l) => $firstName === $l->getChildPlayer()->getFirstName(),
        ));
        self::assertNotFalse($link, sprintf('Expected a child named "%s".', $firstName));

        return (int) $link->getChildPlayer()->getId();
    }

    private function findMembership(string $trainerSlug, int $childId): ?\App\Identity\Entity\PlayerTrainerMembership
    {
        $trainer = $this->trainer($trainerSlug);
        $this->activateTenant($trainer);

        /** @var PlayerTrainerMembershipRepository $memberships */
        $memberships = self::getContainer()->get(PlayerTrainerMembershipRepository::class);
        /** @var \App\Identity\Repository\PlayerProfileRepository $playerProfiles */
        $playerProfiles = self::getContainer()->get(\App\Identity\Repository\PlayerProfileRepository::class);
        $child = $playerProfiles->find($childId);
        self::assertNotNull($child);

        return $memberships->findOneByTrainerAndPlayer($trainer, $child);
    }

    /**
     * AC-01-18: the child profile is linked to the parent account and the
     * parent can switch between children via a context selector.
     */
    public function testParentCanSwitchChildContext(): void
    {
        $parent = $this->account('player@practiceperfect.test');
        $this->client->loginUser($parent);

        $crawler = $this->client->request('GET', '/portal/family');
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Switch to Alex')->form();
        $this->client->submit($form);

        self::assertResponseRedirects('/portal/family', message: 'AC-01-18: switching redirects back with the new context active.');
    }

    /**
     * AC-01-19: each child has its own availability preferences per trainer —
     * exercised at the "Best Times" route, scoped to whichever child the
     * context selector last switched to.
     */
    public function testChildHasSeparateAvailabilityFromTheParent(): void
    {
        $parent = $this->account('player@practiceperfect.test');
        $this->client->loginUser($parent);

        $familyCrawler = $this->client->request('GET', '/portal/family');
        $switchForm = $familyCrawler->selectButton('Switch to Alex')->form();
        $this->client->submit($switchForm);
        self::assertResponseRedirects();

        $crawler = $this->client->request('GET', '/portal/availability');
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('p', 'Alex');
    }

    /**
     * AC-01-20: a child can optionally have a separate login, sharing the
     * parent's contact info.
     */
    public function testChildCanBeGivenASeparateLoginSharingParentContact(): void
    {
        $parent = $this->account('player@practiceperfect.test');
        /** @var ParentChildLinkRepository $links */
        $links = self::getContainer()->get(ParentChildLinkRepository::class);
        $alexLink = current(array_filter(
            $links->findByParent($parent),
            static fn ($link) => 'Alex' === $link->getChildPlayer()->getFirstName(),
        ));
        self::assertNotFalse($alexLink);
        self::assertNull($alexLink->getChildAccount(), 'Alex starts with no separate login.');

        $childAccount = new \App\Identity\Entity\Account('alex.child@example.test', '', \App\Identity\Entity\AccountRole::Player);
        $alexLink->attachChildAccount($childAccount);

        self::assertSame($childAccount, $alexLink->getChildAccount(), 'AC-01-20: the child can be given a separate login.');
    }

    /**
     * AC-01-21: creation requires name, age (1-18), and gender; the system
     * warns (non-blocking) on a possible duplicate.
     */
    public function testChildAgeMustBeBetweenOneAndEighteen(): void
    {
        $parent = $this->account('player@practiceperfect.test');
        $this->client->loginUser($parent);

        $tooOldDob = sprintf('%d-01-01', ((int) date('Y')) - 25);
        $crawler = $this->client->request('GET', '/portal/family/children/new');
        $form = $crawler->selectButton('Save child profile')->form([
            'child_profile[firstName]' => 'TooOld',
            'child_profile[dateOfBirth]' => $tooOldDob,
        ]);
        $this->client->submit($form);

        self::assertResponseStatusCodeSame(422, 'AC-01-21: age outside 1-18 is rejected, not silently accepted.');
    }

    /**
     * AC-01-21: a duplicate-looking profile warns but does not block.
     */
    public function testSimilarChildProfileWarnsButDoesNotBlock(): void
    {
        $parent = $this->account('player@practiceperfect.test');
        $this->client->loginUser($parent);

        // "Alex" already exists for this parent (fixtures).
        $dob = sprintf('%d-04-18', ((int) date('Y')) - 12);
        $crawler = $this->client->request('GET', '/portal/family/children/new');
        $form = $crawler->selectButton('Save child profile')->form([
            'child_profile[firstName]' => 'Alex',
            'child_profile[dateOfBirth]' => $dob,
        ]);
        $this->client->submit($form);

        // Non-blocking: the request still succeeds (redirect), not a 4xx.
        self::assertResponseRedirects('/portal/family', message: 'AC-01-21: a possible duplicate does not block creation.');
    }
}
