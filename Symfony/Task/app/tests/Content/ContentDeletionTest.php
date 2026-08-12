<?php

declare(strict_types=1);

namespace App\Tests\Content;

use App\Content\Repository\ContentProgressRepository;
use App\Content\Repository\DrillRepository;
use App\Content\Repository\PlaylistItemRepository;
use App\Content\Repository\PlaylistRepository;
use App\Tests\Support\ContentFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-04.10 — Trainer Deletes Playlist or Drill.
 */
final class ContentDeletionTest extends WebTestCase
{
    use FixtureHelpers;
    use ContentFixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-04-34: deleting a playlist warns it will unassign it from all
     * players; on confirmation it is soft-deleted (AC-04-36), removed from
     * the assigned player's portal, and player progress history is
     * preserved for trainer analytics.
     */
    public function testTrainerDeletesPlaylistWithConfirmationAndSoftDelete(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createLearnPlaylist($trainer, ['title' => 'Deletable Playlist']);
        $video = $this->createVideoItem($trainer);
        $this->addItemToPlaylist($trainer, $playlist, $video, 1);
        $pat = $this->patPlayer();
        $this->assignPlaylistToPlayer($trainer, $playlist, $pat, $this->account('trainer@practiceperfect.test'));
        $this->grantPlaylistAccess($trainer, $playlist, $pat, $this->account('player@practiceperfect.test'));
        $this->markContentCompleted($trainer, $pat, $video);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', sprintf('/trainer/content/playlists/%d/delete', $playlist->getId()));
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'This will unassign it from all players');

        $form = $crawler->selectButton('Confirm delete')->form();
        $this->client->submit($form);

        self::assertResponseRedirects();
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', 'deleted');

        $this->activateTenant($trainer);
        /** @var PlaylistRepository $playlists */
        $playlists = self::getContainer()->get(PlaylistRepository::class);
        self::assertNull($playlists->findOwnById($trainer, (int) $playlist->getId()), 'AC-04-36: soft-deleted — excluded from ordinary lookups.');

        // AC-04-34: removed from the assigned player's portal.
        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $this->client->request('GET', '/portal/content', ['tab' => 'learn']);
        self::assertSelectorTextNotContains('body', 'Deletable Playlist');

        // AC-04-34: player progress HISTORY is preserved (the data survives
        // even though the playlist itself is gone from ordinary lookups —
        // ContentProgress has no FK to the playlist at all).
        $this->activateTenant($trainer);
        /** @var ContentProgressRepository $progressRepo */
        $progressRepo = self::getContainer()->get(ContentProgressRepository::class);
        $progress = $progressRepo->findOneByPlayerAndContentItem($pat, $video);
        self::assertNotNull($progress, 'AC-04-34: progress history is preserved for trainer analytics.');
        self::assertTrue($progress->isCompleted());
    }

    /**
     * AC-04-34, edge case: a playlist deletion by its OWNER breaks other
     * trainers' references to any content items it owned, but never
     * touches OTHER trainers' own playlists.
     */
    public function testDeletingAPlaylistDoesNotAffectOtherTrainersOwnPlaylists(): void
    {
        $trainerA = $this->trainer('peak-performance');
        $this->activateTenant($trainerA);
        $playlist = $this->createLearnPlaylist($trainerA, ['title' => 'Only Peak\'s Own Playlist']);

        $trainerB = $this->trainer('baseline-athletics');
        $this->activateTenant($trainerB);
        $otherPlaylist = $this->createLearnPlaylist($trainerB, ['title' => 'Baseline\'s Own Playlist']);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', sprintf('/trainer/content/playlists/%d/delete', $playlist->getId()));
        $form = $crawler->selectButton('Confirm delete')->form();
        $this->client->submit($form);
        self::assertResponseRedirects();

        $this->activateTenant($trainerB);
        /** @var PlaylistRepository $playlists */
        $playlists = self::getContainer()->get(PlaylistRepository::class);
        self::assertNotNull($playlists->findOwnById($trainerB, (int) $otherPlaylist->getId()), 'Another trainer\'s own playlist is untouched.');
    }

    /**
     * AC-04-35: deleting a drill used in playlists warns how many playlists
     * use it; on confirmation the drill is deleted (soft) and, if it was
     * used by ANOTHER trainer's playlist, that reference shows "Drill
     * unavailable".
     */
    public function testTrainerDeletesDrillWithUsageWarningAndOtherTrainersSeeUnavailable(): void
    {
        $trainerA = $this->trainer('peak-performance');
        $this->activateTenant($trainerA);
        $drill = $this->createDrill($trainerA, ['title' => 'Widely Used Drill', 'isPublic' => true]);
        $ownPlaylist = $this->createPracticePlaylist($trainerA, ['title' => 'Own Playlist Using It']);
        $this->addItemToPlaylist($trainerA, $ownPlaylist, $drill, 1);

        $trainerB = $this->trainer('baseline-athletics');
        $this->activateTenant($trainerB);
        $otherPlaylist = $this->createPracticePlaylist($trainerB, ['title' => 'Baseline Reuses The Drill']);
        $this->addItemToPlaylist($trainerB, $otherPlaylist, $drill, 1);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', sprintf('/trainer/content/drills/%d/delete', $drill->getId()));
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'This drill is used in 1 playlist'); // own tenant's own usage count

        $form = $crawler->selectButton('Confirm delete')->form();
        $this->client->submit($form);
        self::assertResponseRedirects();

        $this->activateTenant($trainerA);
        /** @var DrillRepository $drills */
        $drills = self::getContainer()->get(DrillRepository::class);
        self::assertNull($drills->findOwnById($trainerA, (int) $drill->getId()), 'AC-04-36: soft-deleted.');

        // AC-04-35: Baseline's reference now shows "Drill unavailable".
        $this->client->loginUser($this->account('trainer-b@practiceperfect.test'));
        $this->client->request('GET', sprintf('/trainer/content/playlists/%d', $otherPlaylist->getId()));
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Unavailable');
    }
}
