<?php

declare(strict_types=1);

namespace App\Tests\Scheduling;

use App\Identity\Entity\SkillLevel;
use App\Scheduling\Entity\Event;
use App\Tests\Support\CrmFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * BR-02-5: a skill-restricted event is shown to the players who hold that
 * skill level. The whole of DEF-04, asserted where it actually hurt.
 *
 * Manual testing found the same player, the same event, and one difference:
 * a profile reading "intermediate" instead of "Intermediate" removed the
 * event from their calendar entirely. No error, no warning — the trainer saw
 * a filled-in profile and the player saw a shorter list of sessions. The
 * profile field was free text, the filter beside it was a closed list, and
 * eligibility compared the two with `in_array(..., strict)`.
 *
 * The forms are one vocabulary now, but a row written before that is still a
 * row, so these tests set the awkward spelling deliberately rather than
 * assuming the migration ran.
 */
final class SkillLevelEligibilityTest extends WebTestCase
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
     * The defect, as found: only the capitalisation differs.
     */
    public function testAProfileSpelledDifferentlyStillSeesTheEvent(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        $membership = $this->freshPlayerMembership($trainer, 'Case Mismatch');
        $this->setSkillLevel($membership->getId(), 'intermediate');

        $event = $this->createEvent($trainer, [
            'title' => 'Intermediate Only Clinic',
            'skillLevels' => [SkillLevel::INTERMEDIATE],
        ]);

        self::assertTrue(
            $this->isEligible($event, $membership->getId()),
            'A player whose profile says "intermediate" belongs in an event restricted to "Intermediate".',
        );
    }

    /**
     * The rule itself still holds — this is not a fix that makes everything
     * match everything.
     */
    public function testADifferentLevelIsStillExcluded(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        $membership = $this->freshPlayerMembership($trainer, 'Wrong Level');
        $this->setSkillLevel($membership->getId(), SkillLevel::BEGINNER);

        $event = $this->createEvent($trainer, [
            'title' => 'Advanced Only Clinic',
            'skillLevels' => [SkillLevel::ADVANCED],
        ]);

        self::assertFalse($this->isEligible($event, $membership->getId()));
    }

    public function testAPlayerWithNoLevelIsExcludedFromARestrictedEvent(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        $membership = $this->freshPlayerMembership($trainer, 'No Level');
        $this->setSkillLevel($membership->getId(), null);

        $event = $this->createEvent($trainer, [
            'title' => 'Elite Only Clinic',
            'skillLevels' => [SkillLevel::ELITE],
        ]);

        self::assertFalse(
            $this->isEligible($event, $membership->getId()),
            'An unrecorded level is not a wildcard — EventEligibilityChecker documents this choice.',
        );
    }

    /**
     * US-03.09 step 12, which the free-text field made unreachable: "Sarah
     * now appears in filtered lists for Beginners." Set through the real
     * form, read through the real filter.
     */
    public function testTheProfileFormAndTheSegmentationFilterAgree(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer, 'Filter Agrees');

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', sprintf('/trainer/players/%d', $membership->getId()));
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Save changes')->form();
        $form['player_crm_fields[skillLevel]'] = SkillLevel::ADVANCED;
        $this->client->submit($form);

        $crawler = $this->client->request('GET', '/trainer/players?skillLevel='.SkillLevel::ADVANCED);
        self::assertResponseIsSuccessful();
        self::assertStringContainsString(
            $membership->getPlayer()->getFirstName(),
            $crawler->filter('main')->text(),
            'A level chosen on the profile must be findable by the filter that offers the same word.',
        );
    }

    /**
     * The form can no longer produce a value the filter cannot match: the
     * only thing it offers is the shared list.
     */
    public function testTheProfileFormOffersExactlyTheSharedVocabulary(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer, 'Vocabulary');

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', sprintf('/trainer/players/%d', $membership->getId()));

        $options = $crawler->filter('#player_crm_fields_skillLevel option')->each(
            static fn ($node): string => (string) $node->attr('value'),
        );

        self::assertSame(array_merge([''], SkillLevel::ALL), $options, 'An empty "Not set", then the four.');
    }

    private function setSkillLevel(?int $membershipId, ?string $skillLevel): void
    {
        /** @var EntityManagerInterface $entityManager */
        $entityManager = self::getContainer()->get(EntityManagerInterface::class);

        // Written straight to the column on purpose: the point of these
        // tests is what happens to a row the current form could not produce.
        $entityManager->getConnection()->executeStatement(
            'UPDATE player_trainer_membership SET skill_level = ? WHERE id = ?',
            [$skillLevel, $membershipId],
        );
    }

    private function isEligible(Event $event, ?int $membershipId): bool
    {
        /** @var EntityManagerInterface $entityManager */
        $entityManager = self::getContainer()->get(EntityManagerInterface::class);

        // Cleared here, not in setSkillLevel(): the raw UPDATE above is
        // invisible to entities already in the identity map, and clearing
        // any earlier would detach the trainer the test still builds with.
        $entityManager->clear();

        /** @var \App\Identity\Entity\PlayerTrainerMembership $membership */
        $membership = $entityManager->find(\App\Identity\Entity\PlayerTrainerMembership::class, $membershipId);
        /** @var Event $freshEvent */
        $freshEvent = $entityManager->find(Event::class, $event->getId());

        /** @var \App\Scheduling\Service\EventEligibilityChecker $checker */
        $checker = self::getContainer()->get(\App\Scheduling\Service\EventEligibilityChecker::class);

        return $checker->isEligible($freshEvent, $membership);
    }
}
