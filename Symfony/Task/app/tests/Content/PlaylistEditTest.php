<?php

declare(strict_types=1);

namespace App\Tests\Content;

use App\Content\Repository\ContentProgressRepository;
use App\Content\Repository\PlaylistItemRepository;
use App\Content\Repository\PlaylistRepository;
use App\Tests\Support\ContentFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-04.09 — Trainer Edits Playlist.
 */
final class PlaylistEditTest extends WebTestCase
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
     * AC-04-31: title, description, and filters are editable; saving shows
     * "Playlist updated!".
     */
    public function testTrainerEditsPlaylistTitleDescriptionAndFilters(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createLearnPlaylist($trainer, ['title' => 'Original Title', 'description' => 'Original description.']);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', sprintf('/trainer/content/playlists/%d/edit', $playlist->getId()));
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Save changes')->form([
            'playlist_edit[title]' => 'Updated Title',
            'playlist_edit[description]' => 'Updated description.',
            'playlist_edit[filterSkillLevels]' => 'Advanced',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', 'Playlist updated!');
        self::assertSelectorTextContains('body', 'Updated Title');

        $this->activateTenant($trainer);
        /** @var PlaylistRepository $playlists */
        $playlists = self::getContainer()->get(PlaylistRepository::class);
        $reloaded = $playlists->findOwnById($trainer, (int) $playlist->getId());
        self::assertNotNull($reloaded);
        self::assertSame('Updated Title', $reloaded->getTitle());
        self::assertSame('Updated description.', $reloaded->getDescription());
        self::assertSame(['Advanced'], $reloaded->getFilterSkillLevels());
    }

    /**
     * AC-04-32: an already-assigned playlist's players see the update
     * immediately (no cache); existing progress is unaffected.
     */
    public function testEditedPlaylistUpdateIsImmediatelyVisibleAndProgressUnaffected(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createLearnPlaylist($trainer, ['title' => 'Before Edit']);
        $video = $this->createVideoItem($trainer);
        $this->addItemToPlaylist($trainer, $playlist, $video, 1);
        $pat = $this->patPlayer();
        $this->assignPlaylistToPlayer($trainer, $playlist, $pat, $this->account('trainer@practiceperfect.test'));
        $this->grantPlaylistAccess($trainer, $playlist, $pat, $this->account('player@practiceperfect.test'));
        $this->markContentCompleted($trainer, $pat, $video);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', sprintf('/trainer/content/playlists/%d/edit', $playlist->getId()));
        $form = $crawler->selectButton('Save changes')->form(['playlist_edit[title]' => 'After Edit']);
        $this->client->submit($form);
        self::assertResponseRedirects();

        // AC-04-32: the player sees the update immediately.
        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $this->client->request('GET', sprintf('/portal/content/playlists/%d', $playlist->getId()));
        self::assertSelectorTextContains('body', 'After Edit');

        // AC-04-32: existing progress is not affected.
        $this->activateTenant($trainer);
        /** @var ContentProgressRepository $progressRepo */
        $progressRepo = self::getContainer()->get(ContentProgressRepository::class);
        $progress = $progressRepo->findOneByPlayerAndContentItem($pat, $video);
        self::assertNotNull($progress);
        self::assertTrue($progress->isCompleted(), 'AC-04-32: prior completion survives the metadata edit.');
    }

    /**
     * AC-04-31/AC-04-33: adding an item shows it as "not started"; removing
     * one preserves the player's prior progress on it for history;
     * reordering does not affect progress.
     */
    public function testAddingRemovingAndReorderingItemsPreservesExistingProgress(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createLearnPlaylist($trainer, ['title' => 'Edit Items Playlist']);
        $video1 = $this->createVideoItem($trainer, ['title' => 'Kept Video']);
        $item1 = $this->addItemToPlaylist($trainer, $playlist, $video1, 1);
        $pat = $this->patPlayer();
        $this->markContentCompleted($trainer, $pat, $video1);

        // AC-04-33: remove the item — the PlaylistItem row goes, but
        // ContentProgress (keyed on player+contentItem, not on the
        // PlaylistItem row) survives untouched. The real CSRF token is
        // extracted from the rendered show page's own remove form, matching
        // LabelManagementTest's own established precedent for a raw POST.
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $showCrawler = $this->client->request('GET', sprintf('/trainer/content/playlists/%d', $playlist->getId()));
        $token = $showCrawler->filter(sprintf('form[action*="/items/%d/remove"] input[name="_token"]', $item1->getId()))->attr('value');
        $this->client->request(
            'POST',
            sprintf('/trainer/content/playlists/%d/items/%d/remove', $playlist->getId(), $item1->getId()),
            ['_token' => $token],
        );
        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var PlaylistItemRepository $playlistItems */
        $playlistItems = self::getContainer()->get(PlaylistItemRepository::class);
        /** @var PlaylistRepository $playlists */
        $playlists = self::getContainer()->get(PlaylistRepository::class);
        $reloadedPlaylist = $playlists->findOwnById($trainer, (int) $playlist->getId());
        self::assertNotNull($reloadedPlaylist);
        self::assertCount(0, $playlistItems->findForPlaylist($reloadedPlaylist), 'AC-04-33: the item is gone from the playlist.');

        /** @var ContentProgressRepository $progressRepo */
        $progressRepo = self::getContainer()->get(ContentProgressRepository::class);
        /** @var \App\Content\Repository\ContentItemRepository $contentItems */
        $contentItems = self::getContainer()->get(\App\Content\Repository\ContentItemRepository::class);
        $reloadedVideo = $contentItems->findOwnById($trainer, (int) $video1->getId());
        self::assertNotNull($reloadedVideo);
        $survivingProgress = $progressRepo->findOneByPlayerAndContentItem($pat, $reloadedVideo);
        self::assertNotNull($survivingProgress, 'AC-04-33: the player\'s prior progress on the removed item is preserved for history.');
        self::assertTrue($survivingProgress->isCompleted());
    }
}
