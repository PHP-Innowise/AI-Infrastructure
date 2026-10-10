<?php

declare(strict_types=1);

namespace App\Tests\Identity;

use App\Identity\Repository\ParentChildLinkRepository;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-01.04 — Parent Manages Child-Trainer Associations.
 */
final class ChildTrainerAssociationTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-01-22: every child, with trainer associations (name, age,
     * associated trainers with dates).
     */
    public function testFamilyPageListsEveryChildWithTheirTrainerAssociations(): void
    {
        $parent = $this->account('player@practiceperfect.test');
        $this->client->loginUser($parent);

        $crawler = $this->client->request('GET', '/portal/family');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Alex');
        self::assertSelectorTextContains('body', 'Blake');
        self::assertSelectorTextContains('body', 'Peak Performance Basketball');
    }

    /**
     * AC-01-24: removing a child from a trainer soft-deletes the
     * association — the trainer no longer sees the child, but history (the
     * row itself) is preserved.
     */
    public function testParentRemovesAChildFromATrainer(): void
    {
        $parent = $this->account('player@practiceperfect.test');
        $this->client->loginUser($parent);

        $childId = $this->findChildIdByFirstName($parent, 'Alex');
        $trainerId = (int) $this->trainer('peak-performance')->getId();

        $confirmCrawler = $this->client->request('GET', sprintf('/portal/family/children/%d/trainers/%d/remove', $childId, $trainerId));
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('[role="alert"]', 'cancels');

        $form = $confirmCrawler->selectButton('Confirm removal')->form();
        $this->client->submit($form);

        self::assertResponseRedirects('/portal/family');

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var PlayerTrainerMembershipRepository $memberships */
        $memberships = self::getContainer()->get(PlayerTrainerMembershipRepository::class);
        /** @var \App\Identity\Repository\PlayerProfileRepository $playerProfiles */
        $playerProfiles = self::getContainer()->get(\App\Identity\Repository\PlayerProfileRepository::class);
        $child = $playerProfiles->find($childId);
        self::assertNotNull($child);

        $membership = $memberships->findOneByTrainerAndPlayer($this->trainer('peak-performance'), $child);
        self::assertNotNull($membership, 'AC-01-24: the row itself is preserved (soft-delete, not a hard delete).');
        self::assertFalse($membership->isActive(), 'AC-01-24: the trainer no longer sees the child in their roster.');
        self::assertNotNull($membership->getRemovedAt());
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
}
