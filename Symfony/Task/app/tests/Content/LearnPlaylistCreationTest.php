<?php

declare(strict_types=1);

namespace App\Tests\Content;

use App\Content\Entity\ContentItem;
use App\Content\Entity\Playlist;
use App\Identity\Entity\SkillLevel;
use App\Content\Repository\PlaylistItemRepository;
use App\Content\Repository\PlaylistRepository;
use App\Content\Service\PlaylistService;
use App\Content\Dto\VideoItemInput;
use App\Tests\Support\ContentFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-04.01 — Trainer Creates Learn Playlist.
 */
final class LearnPlaylistCreationTest extends WebTestCase
{
    use FixtureHelpers;
    use ContentFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-04-1/AC-04-2: required title (max 100), optional description and
     * customization filters, Private/Public toggle, one embedded video with
     * its own title/instructions/tags/duration — saving creates the
     * playlist and it appears in the Learn tab list with a confirmation.
     */
    public function testTrainerCreatesLearnPlaylistWithRequiredAndOptionalFields(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/trainer/content/learn/new');
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Create Learn Playlist')->form([
            'learn_playlist[title]' => 'Ball Handling Fundamentals',
            'learn_playlist[description]' => 'Foundational dribbling skills.',
            'learn_playlist[isPublic]' => false,
            'learn_playlist[audience]' => Playlist::AUDIENCE_PLAYERS_AND_COACHES,
            'learn_playlist[videos][0][youtubeUrl]' => 'https://www.youtube.com/watch?v=dQw4w9WgXcQ',
            'learn_playlist[videos][0][title]' => 'Two-Ball Dribbling Drill',
            'learn_playlist[videos][0][instructions]' => 'Set up two cones. Dribble between them.',
            'learn_playlist[videos][0][tags]' => 'dribbling, footwork',
            'learn_playlist[videos][0][durationSeconds]' => '180',
        ]);

        // Skill levels are checkboxes now, not a comma-separated text field
        // (see SkillLevel): DomCrawler cannot assign to a compound field, so
        // the selection is merged into the submitted values directly.
        $values = $form->getPhpValues();
        $values['learn_playlist']['filterSkillLevels'] = [SkillLevel::BEGINNER, SkillLevel::INTERMEDIATE];
        $this->client->request($form->getMethod(), $form->getUri(), $values);

        self::assertResponseRedirects();
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', 'Playlist created');

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var PlaylistRepository $playlists */
        $playlists = self::getContainer()->get(PlaylistRepository::class);
        $created = current(array_filter(
            $playlists->findAllForActiveTenant($this->trainer('peak-performance'), Playlist::PILLAR_LEARN),
            static fn (Playlist $p): bool => 'Ball Handling Fundamentals' === $p->getTitle(),
        ));

        self::assertNotFalse($created, 'AC-04-1/2: the playlist appears in the Learn tab list.');
        self::assertSame(Playlist::PILLAR_LEARN, $created->getPillar());
        self::assertFalse($created->isPublic(), 'AC-04-1: Private is the default.');
        self::assertSame(['Beginner', 'Intermediate'], $created->getFilterSkillLevels());

        /** @var PlaylistItemRepository $playlistItems */
        $playlistItems = self::getContainer()->get(PlaylistItemRepository::class);
        $items = $playlistItems->findForPlaylist($created);
        self::assertCount(1, $items, 'AC-04-2: the video was added to the playlist.');
        self::assertSame('Two-Ball Dribbling Drill', $items[0]->getContentItem()->getTitle());
        self::assertSame(180, $items[0]->getContentItem()->getDurationSeconds());
        self::assertSame(['dribbling', 'footwork'], $items[0]->getContentItem()->getTags());
    }

    /**
     * AC-04-3: a non-empty title is required.
     */
    public function testLearnPlaylistTitleIsRequired(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/trainer/content/learn/new');
        $form = $crawler->selectButton('Create Learn Playlist')->form([
            'learn_playlist[title]' => '',
            'learn_playlist[videos][0][youtubeUrl]' => 'https://www.youtube.com/watch?v=dQw4w9WgXcQ',
            'learn_playlist[videos][0][title]' => 'A video',
        ]);
        $this->client->submit($form);

        self::assertResponseIsUnprocessable();
        self::assertSelectorExists('form');
    }

    /**
     * AC-04-3: an invalid-format YouTube URL is rejected.
     */
    public function testLearnPlaylistVideoRequiresValidYoutubeUrlFormat(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/trainer/content/learn/new');
        $form = $crawler->selectButton('Create Learn Playlist')->form([
            'learn_playlist[title]' => 'Bad URL Playlist',
            'learn_playlist[videos][0][youtubeUrl]' => 'https://not-youtube.example.com/video',
            'learn_playlist[videos][0][title]' => 'A video',
        ]);
        $this->client->submit($form);

        self::assertResponseIsUnprocessable();
        self::assertSelectorExists('form');

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var PlaylistRepository $playlists */
        $playlists = self::getContainer()->get(PlaylistRepository::class);
        self::assertEmpty(array_filter(
            $playlists->findAllForActiveTenant($this->trainer('peak-performance'), Playlist::PILLAR_LEARN),
            static fn (Playlist $p): bool => 'Bad URL Playlist' === $p->getTitle(),
        ), 'The rejected playlist was never persisted.');
    }

    /**
     * AC-04-3: Learn playlist creation requires at least one video —
     * asserted directly against PlaylistService (the exact collaborator the
     * controller calls), since simulating a CollectionType submission with
     * ZERO rows through DomCrawler's Form API cannot represent "no rows
     * present" the way a real browser drag-removing the only row can.
     */
    public function testLearnPlaylistRequiresAtLeastOneVideo(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        /** @var PlaylistService $playlistService */
        $playlistService = self::getContainer()->get(PlaylistService::class);

        $this->expectException(\InvalidArgumentException::class);
        $this->expectExceptionMessage('at least one video');

        $playlistService->createLearnPlaylist($trainer, 'Empty Playlist', null, null, null, null, false, Playlist::AUDIENCE_PLAYERS_AND_COACHES, []);
    }

    /**
     * AC-04-3, "Validation": the same video may appear more than once, at
     * different positions.
     */
    public function testSameVideoCanAppearMoreThanOnceInAPlaylist(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        /** @var PlaylistService $playlistService */
        $playlistService = self::getContainer()->get(PlaylistService::class);

        $video = new VideoItemInput('https://www.youtube.com/watch?v=dQw4w9WgXcQ', 'Repeated Video', null, null, null);
        $playlist = $playlistService->createLearnPlaylist($trainer, 'Repeats Allowed', null, null, null, null, false, Playlist::AUDIENCE_PLAYERS_AND_COACHES, [$video, $video]);

        /** @var PlaylistItemRepository $playlistItems */
        $playlistItems = self::getContainer()->get(PlaylistItemRepository::class);
        $items = $playlistItems->findForPlaylist($playlist);

        self::assertCount(2, $items, 'AC-04-3: the same video was added at two different positions.');
        self::assertNotSame($items[0]->getContentItem()->getId(), $items[1]->getContentItem()->getId(), 'Each occurrence is its own ContentItem row (no dedup).');
        self::assertSame($items[0]->getContentItem()->getYoutubeUrl(), $items[1]->getContentItem()->getYoutubeUrl());
        self::assertSame(1, $items[0]->getSequenceOrder());
        self::assertSame(2, $items[1]->getSequenceOrder());
    }
}
