<?php

declare(strict_types=1);

namespace App\Tests\Scheduling;

use App\Identity\Entity\Gender;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Scheduling\Entity\Event;
use App\Scheduling\Service\EventEligibilityChecker;
use App\Tests\Support\CrmFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * BR-02-5's gender axis — the sibling defect the skill-level fix found and
 * left alone on purpose, now looked at on its own.
 *
 * Every form that writes a gender has always offered `female|male|unspecified`
 * from a fixed list. The two places that read one did not agree: the Event
 * Builder's restriction was free text compared with `in_array(..., strict)`,
 * so an event restricted to "Female" matched no player at all, and the CRM
 * filter offered `other`, a value nothing in the product can write.
 *
 * Neither failed loudly. A trainer would see an empty roster and an empty
 * filter result and have no way to tell those apart from "nobody matches".
 */
final class GenderEligibilityTest extends WebTestCase
{
    use FixtureHelpers;
    use CrmFixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * The defect, as it stood: the trainer typed the word they saw on the
     * form's own label.
     */
    public function testAnEventRestrictedToTheLabelStillMatchesTheStoredValue(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        $membership = $this->freshPlayerMembership($trainer, 'Gender Case');
        $this->setGender($membership->getPlayer()->getId(), Gender::FEMALE);

        // "Female" is what the label says, and what a trainer typed into the
        // old free-text field.
        $event = $this->createEvent($trainer, ['title' => 'Girls Only Clinic', 'genders' => ['Female']]);

        self::assertTrue(
            $this->isEligible($event, $membership->getId()),
            'A player whose profile says "female" belongs in an event restricted to "Female".',
        );
    }

    public function testTheRestrictionStillExcludesTheOtherGenders(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        $membership = $this->freshPlayerMembership($trainer, 'Gender Excluded');
        $this->setGender($membership->getPlayer()->getId(), Gender::MALE);

        $event = $this->createEvent($trainer, ['title' => 'Girls Only Clinic 2', 'genders' => [Gender::FEMALE]]);

        self::assertFalse($this->isEligible($event, $membership->getId()));
    }

    public function testAPlayerWhoDidNotStateAGenderIsExcludedFromARestrictedEvent(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        $membership = $this->freshPlayerMembership($trainer, 'Gender Unset');
        $this->setGender($membership->getPlayer()->getId(), null);

        $event = $this->createEvent($trainer, ['title' => 'Restricted Clinic', 'genders' => [Gender::FEMALE]]);

        self::assertFalse(
            $this->isEligible($event, $membership->getId()),
            'An unrecorded value is not a wildcard — EventEligibilityChecker documents that choice.',
        );
    }

    /**
     * The Event Builder can no longer produce a restriction the profile
     * cannot satisfy: the only things it offers are the three stored values.
     */
    public function testTheEventFormOffersExactlyTheStoredVocabulary(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->client->loginUser($trainer->getOwnerAccount());

        $crawler = $this->client->request('GET', '/trainer/events/new');
        self::assertResponseIsSuccessful();

        $offered = $crawler->filter('input[name="event[genders][]"]')->each(
            static fn ($node): string => (string) $node->attr('value'),
        );

        self::assertSame(Gender::ALL, $offered);
    }

    /**
     * The other half: a filter option that no form can produce found nobody,
     * every time. Asserted as "the filter offers only values a profile can
     * hold", because the failure was never visible in a result — an empty
     * list looks the same as a genuinely empty segment.
     */
    public function testTheSegmentationFilterOffersOnlyValuesAProfileCanHold(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->client->loginUser($trainer->getOwnerAccount());

        $crawler = $this->client->request('GET', '/trainer/players');
        self::assertResponseIsSuccessful();

        $offered = $crawler->filter('select[name="gender"] option')->each(
            static fn ($node): string => (string) $node->attr('value'),
        );

        self::assertSame(array_merge([''], Gender::ALL), $offered, 'An empty "All", then the three a profile can actually store.');
        self::assertNotContains('other', $offered, 'No form in the product can write "other".');
    }

    /**
     * And end to end: a gender set on a profile is findable by the filter
     * offering the same word.
     */
    public function testTheFilterFindsAPlayerByTheirStoredGender(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        $membership = $this->freshPlayerMembership($trainer, 'Findable');
        $this->setGender($membership->getPlayer()->getId(), Gender::UNSPECIFIED);

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', '/trainer/players?gender='.Gender::UNSPECIFIED);

        self::assertResponseIsSuccessful();
        self::assertStringContainsString(
            $membership->getPlayer()->getFirstName(),
            $crawler->filter('main')->text(),
        );
    }

    private function setGender(?int $playerId, ?string $gender): void
    {
        /** @var EntityManagerInterface $entityManager */
        $entityManager = self::getContainer()->get(EntityManagerInterface::class);

        $entityManager->getConnection()->executeStatement(
            'UPDATE player_profile SET gender = ? WHERE id = ?',
            [$gender, $playerId],
        );
    }

    private function isEligible(Event $event, ?int $membershipId): bool
    {
        /** @var EntityManagerInterface $entityManager */
        $entityManager = self::getContainer()->get(EntityManagerInterface::class);

        // Cleared here, not in setGender(): the raw UPDATE is invisible to
        // entities already in the identity map, and clearing any earlier
        // would detach the trainer the test still builds with.
        $entityManager->clear();

        /** @var PlayerTrainerMembership $membership */
        $membership = $entityManager->find(PlayerTrainerMembership::class, $membershipId);
        /** @var Event $freshEvent */
        $freshEvent = $entityManager->find(Event::class, $event->getId());

        /** @var EventEligibilityChecker $checker */
        $checker = self::getContainer()->get(EventEligibilityChecker::class);

        return $checker->isEligible($freshEvent, $membership);
    }
}
