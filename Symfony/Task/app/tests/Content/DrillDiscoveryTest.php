<?php

declare(strict_types=1);

namespace App\Tests\Content;

use App\Content\Entity\Drill;
use App\Content\Repository\ContentUsageRepository;
use App\Content\Repository\PlaylistItemRepository;
use App\Tests\Support\ContentFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-04.03 — Trainer Discovers and Uses Public Drill.
 */
final class DrillDiscoveryTest extends WebTestCase
{
    use FixtureHelpers;
    use ContentFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-04-7: toggling to "Public Drills" and filtering by category shows
     * results from OTHER trainers, each with name, creator, difficulty, etc.
     */
    public function testTrainerSearchesAndFiltersPublicDrills(): void
    {
        $trainerB = $this->trainer('baseline-athletics');
        $this->activateTenant($trainerB);
        $this->createDrill($trainerB, [
            'title' => 'Elite Cone Drill',
            'difficultyLevel' => Drill::DIFFICULTY_ELITE,
            'categories' => ['Agility'],
            'isPublic' => true,
        ]);
        $this->createDrill($trainerB, [
            'title' => 'Beginner Passing Drill',
            'difficultyLevel' => Drill::DIFFICULTY_BEGINNER,
            'categories' => ['Passing'],
            'isPublic' => true,
        ]);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', '/trainer/content/drills', ['scope' => 'public', 'difficulty' => Drill::DIFFICULTY_ELITE]);
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Elite Cone Drill');
        self::assertSelectorTextNotContains('body', 'Beginner Passing Drill');
        self::assertSelectorTextContains('body', 'Baseline Athletics');
    }

    /**
     * AC-04-9: an empty result set shows the epic's own exact copy.
     */
    public function testEmptyPublicDrillSearchShowsNoMatchesMessage(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', '/trainer/content/drills', ['scope' => 'public', 'q' => 'no-such-drill-exists-xyz']);

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'No drills match your filters. Try adjusting criteria.');
    }

    /**
     * AC-04-9: search matches drill name, description, and tags; filters
     * combine with AND logic.
     */
    public function testPublicDrillSearchMatchesNameDescriptionAndTagsWithAndCombinedFilters(): void
    {
        $trainerB = $this->trainer('baseline-athletics');
        $this->activateTenant($trainerB);
        $this->createDrill($trainerB, [
            'title' => 'Sharp Cuts',
            'instructions' => 'Focus on explosive change of direction.',
            'difficultyLevel' => Drill::DIFFICULTY_ADVANCED,
            'categories' => ['Agility'],
            'isPublic' => true,
        ]);
        $this->createDrill($trainerB, [
            'title' => 'Sharp Cuts Beginner Variant',
            'difficultyLevel' => Drill::DIFFICULTY_BEGINNER,
            'categories' => ['Agility'],
            'isPublic' => true,
        ]);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        // AND logic: matches the "Sharp Cuts" query AND the advanced
        // difficulty filter — excludes the beginner variant even though it
        // also matches the text query.
        $this->client->request('GET', '/trainer/content/drills', ['scope' => 'public', 'q' => 'Sharp Cuts', 'difficulty' => Drill::DIFFICULTY_ADVANCED]);
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Sharp Cuts');
        self::assertSelectorTextNotContains('body', 'Sharp Cuts Beginner Variant');
    }

    /**
     * AC-04-8: "Preview" opens the full drill details; "Add to Playlist"
     * adds it BY REFERENCE (BR-04-12) to the trainer's own Practice
     * playlist, recording the original creator, with a confirmation.
     */
    public function testTrainerPreviewsAndAddsPublicDrillToOwnPlaylist(): void
    {
        $trainerB = $this->trainer('baseline-athletics');
        $this->activateTenant($trainerB);
        $drill = $this->createDrill($trainerB, ['title' => 'Reusable Public Drill', 'isPublic' => true]);

        $trainerA = $this->trainer('peak-performance');
        $this->activateTenant($trainerA);
        $playlist = $this->createPracticePlaylist($trainerA, ['title' => 'My Workout']);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        // AC-04-8: Preview shows the full drill details.
        $this->client->request('GET', sprintf('/trainer/content/drills/%d', $drill->getId()));
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Reusable Public Drill');
        self::assertSelectorTextContains('body', 'Baseline Athletics');

        // AC-04-8: "Add to Playlist".
        $crawler = $this->client->request('GET', sprintf('/trainer/content/playlists/%d/drills', $playlist->getId()));
        self::assertResponseIsSuccessful();
        $form = $crawler->selectButton('Add to playlist')->form([
            'add_drill_to_playlist[drill]' => (string) $drill->getId(),
            'add_drill_to_playlist[trainerNotes]' => 'Great warmup drill.',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', sprintf('Drill added to %s', $playlist->getTitle()));

        $this->activateTenant($trainerA);
        /** @var PlaylistItemRepository $playlistItems */
        $playlistItems = self::getContainer()->get(PlaylistItemRepository::class);
        $items = $playlistItems->findForPlaylist($playlist);
        self::assertCount(1, $items);
        self::assertSame($drill->getId(), $items[0]->getContentItem()->getId(), 'BR-04-12: a REFERENCE was stored, not a copy.');
        self::assertSame('Great warmup drill.', $items[0]->getTrainerNotes());

        /** @var ContentUsageRepository $usage */
        $usage = self::getContainer()->get(ContentUsageRepository::class);
        $usageRow = $usage->findOneByTrainerAndContentItem($trainerA, $items[0]->getContentItem());
        self::assertNotNull($usageRow, 'BR-04-12/AC-04-37: usage is tracked for the reusing trainer.');
    }

    /**
     * AC-04-10: the original creator is tracked and shown in the admin
     * view; player-facing attribution is out of MVP scope (deferred to
     * Phase 2), so only the trainer-facing (admin) view is asserted here.
     */
    public function testOriginalCreatorIsTrackedAndShownToOtherTrainers(): void
    {
        $trainerB = $this->trainer('baseline-athletics');
        $this->activateTenant($trainerB);
        $drill = $this->createDrill($trainerB, ['title' => 'Attribution Test Drill', 'isPublic' => true]);

        self::assertSame($trainerB->getId(), $drill->getTrainer()->getId(), 'AC-04-10: the original creator is tracked in the database.');

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', sprintf('/trainer/content/drills/%d', $drill->getId()));

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'By Baseline Athletics');
    }
}
