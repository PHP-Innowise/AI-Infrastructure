<?php

declare(strict_types=1);

namespace App\Tests\Content;

use App\Content\Entity\Playlist;
use App\Content\Repository\PlaylistItemRepository;
use App\Content\Repository\PlaylistRepository;
use App\Tests\Support\ContentFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-04.04 — Trainer Creates Practice Playlist (Workout).
 */
final class PracticePlaylistCreationTest extends WebTestCase
{
    use FixtureHelpers;
    use ContentFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-04-11: required title, optional description/filters, Private/
     * Public toggle — saving creates the playlist, which appears in the
     * Practice tab list.
     */
    public function testTrainerCreatesPracticePlaylistWithRequiredAndOptionalFields(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/trainer/content/practice/new');
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Create Practice Playlist')->form([
            'practice_playlist[title]' => 'Preseason Conditioning',
            'practice_playlist[description]' => 'A four-drill conditioning circuit.',
            'practice_playlist[filterPositions]' => 'Guard, Forward',
            'practice_playlist[isPublic]' => false,
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', 'Playlist created');

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var PlaylistRepository $playlists */
        $playlists = self::getContainer()->get(PlaylistRepository::class);
        $created = current(array_filter(
            $playlists->findAllForActiveTenant($this->trainer('peak-performance'), Playlist::PILLAR_PRACTICE),
            static fn (Playlist $p): bool => 'Preseason Conditioning' === $p->getTitle(),
        ));

        self::assertNotFalse($created, 'AC-04-11: the playlist appears in the Practice tab list.');
        self::assertSame(Playlist::PILLAR_PRACTICE, $created->getPillar());
        self::assertSame(['Guard', 'Forward'], $created->getFilterPositions());
    }

    /**
     * AC-04-11/12: drills added from the Drill Database (own drills), each
     * with optional trainer notes; AC-04-12: drills can be reordered and
     * removed.
     */
    public function testTrainerAddsOwnDrillsToPracticePlaylistThenReordersAndRemoves(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createPracticePlaylist($trainer, ['title' => 'Shooting Circuit']);
        $drillA = $this->createDrill($trainer, ['title' => 'Form Shooting']);
        $drillB = $this->createDrill($trainer, ['title' => 'Catch and Shoot']);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', sprintf('/trainer/content/playlists/%d/drills', $playlist->getId()));
        $form = $crawler->selectButton('Add to playlist')->form([
            'add_drill_to_playlist[drill]' => (string) $drillA->getId(),
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects();

        $crawler = $this->client->request('GET', sprintf('/trainer/content/playlists/%d/drills', $playlist->getId()));
        $form = $crawler->selectButton('Add to playlist')->form([
            'add_drill_to_playlist[drill]' => (string) $drillB->getId(),
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects();

        // Each $client->request() reboots the kernel, so the repository and
        // tenant context are re-fetched/re-activated after every round of
        // HTTP calls, matching EpicCompletionCriteriaTest's own established
        // pattern — a repository fetched before a request would otherwise
        // read through a stale EntityManager bound to the PREVIOUS boot.
        $this->activateTenant($trainer);
        /** @var PlaylistItemRepository $playlistItems */
        $playlistItems = self::getContainer()->get(PlaylistItemRepository::class);
        $items = $playlistItems->findForPlaylist($playlist);
        self::assertCount(2, $items, 'AC-04-11: both drills were added.');
        self::assertSame(1, $items[0]->getSequenceOrder());
        self::assertSame(2, $items[1]->getSequenceOrder());
        $firstItemId = $items[0]->getId();
        $secondItemId = $items[1]->getId();

        // AC-04-12: reorder — swap positions via the JSON endpoint.
        $this->client->request(
            'POST',
            sprintf('/trainer/content/playlists/%d/items/reorder', $playlist->getId()),
            [],
            [],
            ['CONTENT_TYPE' => 'application/json'],
            json_encode(['order' => [$secondItemId, $firstItemId]], \JSON_THROW_ON_ERROR),
        );
        self::assertResponseStatusCodeSame(204);

        $this->activateTenant($trainer);
        /** @var PlaylistItemRepository $playlistItemsAfterReorder */
        $playlistItemsAfterReorder = self::getContainer()->get(PlaylistItemRepository::class);
        $reordered = $playlistItemsAfterReorder->findForPlaylist($playlist);
        self::assertSame($secondItemId, $reordered[0]->getId(), 'AC-04-12: reordered — the second item now comes first.');
        self::assertSame($firstItemId, $reordered[1]->getId());

        // AC-04-12/31: removal.
        $this->client->request('GET', sprintf('/trainer/content/playlists/%d', $playlist->getId()));
        self::assertSelectorTextContains('body', 'Form Shooting');

        $csrfCrawler = $this->client->request('GET', sprintf('/trainer/content/playlists/%d', $playlist->getId()));
        $removeForm = $csrfCrawler->filter(sprintf('form[action*="/items/%d/remove"]', $firstItemId))->form();
        $this->client->submit($removeForm);
        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var PlaylistItemRepository $playlistItemsAfterRemoval */
        $playlistItemsAfterRemoval = self::getContainer()->get(PlaylistItemRepository::class);
        $afterRemoval = $playlistItemsAfterRemoval->findForPlaylist($playlist);
        self::assertCount(1, $afterRemoval, 'AC-04-12: the removed drill is gone.');
    }
}
