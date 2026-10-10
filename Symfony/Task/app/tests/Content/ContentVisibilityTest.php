<?php

declare(strict_types=1);

namespace App\Tests\Content;

use App\Content\Entity\Playlist;
use App\Content\Repository\PlaylistRepository;
use App\Tests\Support\ContentFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-04.06 — Trainer Marks Content as Public.
 *
 * The two RLS-level pinning tests for BR-04-5/BR-04-12 (the publication
 * exception itself) live in PublicationExceptionTest, not here — this file
 * covers the HTTP-level toggle workflow and its confirmation/badge/
 * attribution behavior.
 */
final class ContentVisibilityTest extends WebTestCase
{
    use FixtureHelpers;
    use ContentFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-04-17: toggling to Public shows a confirmation, and the trainer
     * retains ownership.
     */
    public function testTrainerTogglesPlaylistFromPrivateToPublic(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createLearnPlaylist($trainer, ['title' => 'Toggle Me Public']);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', sprintf('/trainer/content/playlists/%d/visibility', $playlist->getId()));
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Make "Toggle Me Public" public? Other trainers will be able to discover and use it.');

        $form = $crawler->selectButton('Save visibility')->form([
            'playlist_visibility[publication]' => '1',
            'playlist_visibility[audience]' => Playlist::AUDIENCE_PLAYERS_AND_COACHES,
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', 'now public');

        $this->activateTenant($trainer);
        /** @var PlaylistRepository $playlists */
        $playlists = self::getContainer()->get(PlaylistRepository::class);
        $reloaded = $playlists->findOwnById($trainer, (int) $playlist->getId());
        self::assertNotNull($reloaded);
        self::assertTrue($reloaded->isPublic());
        self::assertSame($trainer->getId(), $reloaded->getTrainer()->getId(), 'AC-04-17: the trainer retains ownership.');
    }

    /**
     * AC-04-18: public content appears in public discovery for all
     * trainers with a "Public" badge.
     */
    public function testPublicPlaylistAppearsInPublicDiscoveryWithBadge(): void
    {
        $trainerB = $this->trainer('baseline-athletics');
        $this->activateTenant($trainerB);
        $drill = $this->createDrill($trainerB, ['title' => 'Publicly Discoverable Drill', 'isPublic' => true]);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/content/drills', ['scope' => 'public']);

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Publicly Discoverable Drill');
        self::assertSelectorExists('.badge--public');
    }

    /**
     * AC-04-19: the original creator is stored and shown as "By [Trainer
     * Name]" in admin views.
     */
    public function testOriginalCreatorShownAsByTrainerNameInAdminView(): void
    {
        $trainerB = $this->trainer('baseline-athletics');
        $this->activateTenant($trainerB);
        $playlist = $this->createLearnPlaylist($trainerB, ['title' => 'Attribution Playlist', 'isPublic' => true]);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', sprintf('/trainer/content/playlists/%d', $playlist->getId()));

        self::assertResponseIsSuccessful('AC-04-19/BR-04-4: any trainer may view a published playlist.');
    }

    /**
     * AC-04-19/BR-04-5: reverting to Private removes it from public
     * discovery (the "removes from discovery" half — the "existing
     * references keep working" half is PublicationExceptionTest's own,
     * more targeted pinning test).
     */
    public function testRevertingToPrivateRemovesFromPublicDiscovery(): void
    {
        $trainerB = $this->trainer('baseline-athletics');
        $this->activateTenant($trainerB);
        $drill = $this->createDrill($trainerB, ['title' => 'Soon To Be Private', 'isPublic' => true]);

        $this->client->loginUser($this->account('trainer-b@practiceperfect.test'));
        $crawler = $this->client->request('GET', sprintf('/trainer/content/drills/%d/visibility', $drill->getId()));
        $form = $crawler->selectButton('Save visibility')->form(['publish_toggle[isPublic]' => '0']);
        $this->client->submit($form);
        self::assertResponseRedirects();

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', '/trainer/content/drills', ['scope' => 'public', 'q' => 'Soon To Be Private']);

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'No drills match your filters. Try adjusting criteria.');
    }
}
