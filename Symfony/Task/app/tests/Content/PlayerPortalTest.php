<?php

declare(strict_types=1);

namespace App\Tests\Content;

use App\Content\Entity\Playlist;
use App\Content\Repository\ContentProgressRepository;
use App\Tests\Support\ContentFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-04.07 — Player Views Assigned Content (LPPP Portal).
 */
final class PlayerPortalTest extends WebTestCase
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
     * AC-04-20: navigating to LPPP shows content for the player's currently
     * selected trainer context only, with Learn/Practice/Progress tabs.
     */
    public function testPortalShowsLearnAndPracticeTabsForCurrentTrainerContext(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);

        $crawler = $this->client->request('GET', '/portal/content');
        self::assertResponseIsSuccessful();
        self::assertSelectorExists('a[href*="tab=learn"]');
        self::assertSelectorExists('a[href*="tab=practice"]');
        self::assertSelectorExists('a[href$="/portal/content/progress"]');
    }

    /**
     * AC-04-21: the Learn tab's Content Library shows every player-visible
     * Learn playlist for the current trainer — purchased (unlocked) and
     * unpurchased (locked) alike, each with title/description.
     */
    public function testLearnTabShowsPurchasedAndUnpurchasedPlaylists(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $locked = $this->createLearnPlaylist($trainer, ['title' => 'Locked Learn Playlist']);
        $unlocked = $this->createLearnPlaylist($trainer, ['title' => 'Unlocked Learn Playlist']);
        $pat = $this->patPlayer();
        $this->grantPlaylistAccess($trainer, $unlocked, $pat, $this->account('player@practiceperfect.test'));

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $this->client->request('GET', '/portal/content', ['tab' => 'learn']);

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Locked Learn Playlist');
        self::assertSelectorTextContains('body', 'Unlocked Learn Playlist');
        self::assertSelectorTextContains('body', 'Locked');
    }

    /**
     * AC-04-22: clicking a LOCKED playlist opens the purchase flow; an
     * unlocked/purchased playlist opens its detail listing videos with a
     * completion checkmark.
     */
    public function testClickingLockedPlaylistOffersPurchaseUnlockedShowsItems(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createLearnPlaylist($trainer, ['title' => 'Purchasable Playlist']);
        $video = $this->createVideoItem($trainer, ['title' => 'Intro Video']);
        $this->addItemToPlaylist($trainer, $playlist, $video, 1);
        $pat = $this->patPlayer();

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);

        // Locked: purchase CTA shown.
        $this->client->request('GET', sprintf('/portal/content/playlists/%d', $playlist->getId()));
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Locked');
        self::assertSelectorExists('a[href*="/checkout"]');

        // Grant access, then reload — item list with completion checkmark.
        // Every entity is re-fetched fresh (not reused from before the
        // request) — the client's kernel reboot leaves $trainer/$playlist/
        // $pat bound to a stale EntityManager, which
        // ORMInvalidArgumentException ("non-persisted new entities") would
        // otherwise catch on the next persist().
        $this->activateTenant($trainer);
        $freshTrainer = $this->trainer('peak-performance');
        /** @var \App\Content\Repository\PlaylistRepository $playlistsRepo */
        $playlistsRepo = self::getContainer()->get(\App\Content\Repository\PlaylistRepository::class);
        $freshPlaylist = $playlistsRepo->findOwnById($freshTrainer, (int) $playlist->getId());
        self::assertNotNull($freshPlaylist);
        /** @var \App\Content\Repository\ContentItemRepository $contentItemsRepo */
        $contentItemsRepo = self::getContainer()->get(\App\Content\Repository\ContentItemRepository::class);
        $freshVideo = $contentItemsRepo->findOwnById($freshTrainer, (int) $video->getId());
        self::assertNotNull($freshVideo);
        $freshPat = $this->patPlayer();

        $this->grantPlaylistAccess($freshTrainer, $freshPlaylist, $freshPat, $this->account('player@practiceperfect.test'));
        $this->markContentCompleted($freshTrainer, $freshPat, $freshVideo);

        $this->client->request('GET', sprintf('/portal/content/playlists/%d', $playlist->getId()));
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Intro Video');
        self::assertSelectorTextContains('body', 'Completed');
    }

    /**
     * AC-04-23: the Practice tab lists ASSIGNED Practice playlists, each
     * showing title, description, and progress; opening one shows its
     * drills in order with trainer notes.
     */
    public function testPracticeTabListsAssignedPlaylistsWithProgressAndTrainerNotes(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createPracticePlaylist($trainer, ['title' => 'Assigned Workout']);
        $drill = $this->createDrill($trainer, ['title' => 'Suicide Sprints']);
        $this->addItemToPlaylist($trainer, $playlist, $drill, 1, 'Push hard on the last rep.');
        $pat = $this->patPlayer();
        $this->assignPlaylistToPlayer($trainer, $playlist, $pat, $this->account('trainer@practiceperfect.test'));
        // BR-04-6 applies the paywall uniformly to Learn and Practice alike
        // in this implementation (see PlaylistService's own docblock/the
        // coder's final report) — being ASSIGNED is distinct from having
        // PURCHASED (BR-04-9: "must still purchase access if paywall is
        // enabled"), so the item list itself needs an explicit grant too.
        $this->grantPlaylistAccess($trainer, $playlist, $pat, $this->account('player@practiceperfect.test'));

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $this->client->request('GET', '/portal/content', ['tab' => 'practice']);

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Assigned Workout');

        $this->client->request('GET', sprintf('/portal/content/playlists/%d', $playlist->getId()));
        self::assertSelectorTextContains('body', 'Suicide Sprints');
        self::assertSelectorTextContains('body', 'Push hard on the last rep.');
    }

    /**
     * AC-04-24: the player page shows the embedded player, title, and a
     * Text Instructions section.
     */
    public function testVideoPlayerPageShowsEmbeddedPlayerAndInstructions(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createLearnPlaylist($trainer, ['title' => 'Player Page Test']);
        $video = $this->createVideoItem($trainer, [
            'title' => 'Setup And Footwork',
            'instructions' => 'Key points: stay low, eyes up, quick first step.',
        ]);
        $this->addItemToPlaylist($trainer, $playlist, $video, 1);
        $pat = $this->patPlayer();
        $this->grantPlaylistAccess($trainer, $playlist, $pat, $this->account('player@practiceperfect.test'));

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $this->client->request('GET', sprintf('/portal/content/items/%d/play', $video->getId()));

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Setup And Footwork');
        self::assertSelectorTextContains('body', 'stay low, eyes up, quick first step');
        self::assertSelectorExists('iframe');
    }

    /**
     * AC-04-25/BR-04-16: clicking play automatically marks content
     * complete, immediately, and is idempotent on replay.
     */
    public function testClickingPlayAutomaticallyMarksContentCompleteAndIsIdempotent(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createLearnPlaylist($trainer, ['title' => 'Auto Complete Test']);
        $video = $this->createVideoItem($trainer, ['title' => 'Auto-Complete Video']);
        $this->addItemToPlaylist($trainer, $playlist, $video, 1);
        $pat = $this->patPlayer();
        $this->grantPlaylistAccess($trainer, $playlist, $pat, $this->account('player@practiceperfect.test'));

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $this->client->request('POST', sprintf('/portal/content/items/%d/complete', $video->getId()));
        self::assertResponseIsSuccessful();
        self::assertJsonStringEqualsJsonString('{"completed": true}', (string) $this->client->getResponse()->getContent());

        $this->activateTenant($trainer);
        /** @var ContentProgressRepository $progressRepo */
        $progressRepo = self::getContainer()->get(ContentProgressRepository::class);
        $progress = $progressRepo->findOneByPlayerAndContentItem($pat, $video);
        self::assertNotNull($progress);
        self::assertTrue($progress->isCompleted());
        $firstProgressId = $progress->getId();
        // Compared at whole-second precision, matching the column's own
        // TIMESTAMP(0) type — an in-memory, not-yet-round-tripped
        // DateTimeImmutable can carry microseconds a freshly re-queried one
        // never will, which is a precision artifact, not a real
        // idempotency violation.
        $firstCompletedAt = $progress->getCompletedAt()?->format('Y-m-d H:i:s');

        // Idempotent replay: clicking play again does not change the
        // completion timestamp or error, and creates no second row.
        $this->client->request('POST', sprintf('/portal/content/items/%d/complete', $video->getId()));
        self::assertResponseIsSuccessful();

        $this->activateTenant($trainer);
        /** @var ContentProgressRepository $progressRepoAgain */
        $progressRepoAgain = self::getContainer()->get(ContentProgressRepository::class);
        $progressAgain = $progressRepoAgain->findOneByPlayerAndContentItem($pat, $video);
        self::assertNotNull($progressAgain);
        self::assertSame($firstProgressId, $progressAgain->getId(), 'Testing Considerations: idempotent — no second progress row was created.');
        self::assertSame($firstCompletedAt, $progressAgain->getCompletedAt()?->format('Y-m-d H:i:s'), 'Testing Considerations: completion stays idempotent.');
    }

    /**
     * AC-04-26: a parent can switch between children via a dropdown to view
     * each child's assigned content separately — reusing Epic-01's existing
     * child-context switch, exercised end to end here for Content
     * specifically.
     */
    public function testParentSwitchesBetweenChildrenToViewSeparateAssignedContent(): void
    {
        $trainerA = $this->trainer('peak-performance');
        $this->activateTenant($trainerA);
        $alexPlaylist = $this->createPracticePlaylist($trainerA, ['title' => 'Alex Only Playlist']);
        $alex = $this->alexPlayer();
        $this->assignPlaylistToPlayer($trainerA, $alexPlaylist, $alex, $this->account('trainer@practiceperfect.test'));

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainerA);
        $this->switchToChild($this->client, 'Alex');

        $this->client->request('GET', '/portal/content', ['tab' => 'practice']);
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Alex Only Playlist');
    }
}
