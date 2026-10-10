<?php

declare(strict_types=1);

namespace App\Tests\Content;

use App\Content\Entity\Playlist;
use App\Tests\Support\ContentFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * Scope note (not a dedicated user story) — Coach-Only Content.
 *
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md AC-04-41
 */
final class CoachOnlyContentTest extends WebTestCase
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
     * AC-04-41: a trainer can create a playlist visible only to coaches via
     * a visibility toggle; a coach can view it; a player cannot, even
     * within the SAME trainer's own tenant.
     */
    public function testCoachOnlyPlaylistIsVisibleToCoachesNotPlayers(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createLearnPlaylist($trainer, [
            'title' => 'Coach Certification Material',
            'isPublic' => false,
            'audience' => Playlist::AUDIENCE_COACHES_ONLY,
        ]);

        self::assertTrue($playlist->isCoachesOnly());

        // The trainer's own coach can view it.
        $this->client->loginUser($this->account('coach@practiceperfect.test'));
        $this->client->request('GET', sprintf('/coach/content/playlists/%d', $playlist->getId()));
        self::assertResponseIsSuccessful('AC-04-41: a coach may view coach-only content.');
        self::assertSelectorTextContains('body', 'Coach Certification Material');

        // A player, even in the same trainer's own tenant, is denied.
        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $this->client->request('GET', sprintf('/portal/content/playlists/%d', $playlist->getId()));
        self::assertResponseStatusCodeSame(403, 'AC-04-41: coach-only content is never shown to players.');

        // It never appears in the player's own Learn Library either.
        $this->client->request('GET', '/portal/content', ['tab' => 'learn']);
        self::assertResponseIsSuccessful();
        self::assertSelectorTextNotContains('body', 'Coach Certification Material');
    }

    /**
     * AC-04-41/A9: a coach-only playlist can never ALSO be public — the
     * fourth combination the CHECK constraint (and the entity guard)
     * forbids.
     */
    public function testCoachOnlyPlaylistCannotAlsoBePublic(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createLearnPlaylist($trainer, ['title' => 'Attempted Public Coach Only']);

        $this->expectException(\InvalidArgumentException::class);
        $playlist->setVisibility(true, Playlist::AUDIENCE_COACHES_ONLY, new \DateTimeImmutable());
    }

    /**
     * AC-04-41, over HTTP: the visibility form itself rejects the same
     * invalid combination as an ordinary validation error, not a 500.
     */
    public function testVisibilityFormRejectsPublicCoachesOnlyCombination(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createLearnPlaylist($trainer, ['title' => 'Form Validation Playlist']);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', sprintf('/trainer/content/playlists/%d/visibility', $playlist->getId()));
        $form = $crawler->selectButton('Save visibility')->form([
            'playlist_visibility[publication]' => '1',
            'playlist_visibility[audience]' => Playlist::AUDIENCE_COACHES_ONLY,
        ]);
        $this->client->submit($form);

        self::assertResponseIsUnprocessable();
    }
}
