<?php

declare(strict_types=1);

namespace App\Tests\Content;

use App\Tests\Support\ContentFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-04.11 — Super Admin Views LPPP Analytics.
 */
final class SuperAdminContentAnalyticsTest extends WebTestCase
{
    use FixtureHelpers;
    use ContentFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-04-37: content stats (total playlists, total drills, public-vs-
     * private ratio, top 10 most-used public drills, top content
     * creators).
     */
    public function testSuperAdminSeesContentStatsTopDrillsAndTopCreators(): void
    {
        $trainerA = $this->trainer('peak-performance');
        $this->activateTenant($trainerA);
        $this->createLearnPlaylist($trainerA, ['title' => 'Analytics Playlist A']);
        $widelyUsedDrill = $this->createDrill($trainerA, ['title' => 'Analytics Popular Drill', 'isPublic' => true]);

        $trainerB = $this->trainer('baseline-athletics');
        $this->activateTenant($trainerB);
        $reusingPlaylist = $this->createPracticePlaylist($trainerB, ['title' => 'Reusing Playlist']);
        $this->addItemToPlaylist($trainerB, $reusingPlaylist, $widelyUsedDrill, 1);

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/super-admin/content/analytics');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Total playlists');
        self::assertSelectorTextContains('body', 'Total drills');
        self::assertSelectorTextContains('body', 'Public');
        self::assertSelectorTextContains('body', 'Private');
        self::assertSelectorTextContains('body', 'Analytics Popular Drill');
        self::assertSelectorTextContains('body', 'Peak Performance Basketball');

        $text = $crawler->filter('body')->text();
        self::assertStringContainsString('used by 1 trainer', $text, 'AC-04-37: usage count for the top drill.');
    }

    /**
     * AC-04-38: engagement stats — total player video views (all-time and
     * this week), average watch time per player, completion rate.
     */
    public function testSuperAdminSeesEngagementStats(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $video = $this->createVideoItem($trainer);
        /** @var \App\Identity\Repository\PlayerProfileRepository $playerProfiles */
        $playerProfiles = self::getContainer()->get(\App\Identity\Repository\PlayerProfileRepository::class);
        $player = $playerProfiles->findOneBy([]);
        self::assertNotNull($player, 'Fixtures should provide at least one player.');
        $this->markContentCompleted($trainer, $player, $video);

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $this->client->request('GET', '/super-admin/content/analytics');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Total views (all-time)');
        self::assertSelectorTextContains('body', 'Total views (this week)');
        self::assertSelectorTextContains('body', 'Average watch time per player');
        self::assertSelectorTextContains('body', 'Completion rate');
    }

    /**
     * AC-04-39: growth trends (content-creation, player-engagement, public-
     * content growth, per week).
     */
    public function testSuperAdminSeesGrowthTrends(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/super-admin/content/analytics');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Growth Trends');
        self::assertSelectorExists('table');
        self::assertGreaterThan(0, $crawler->filter('table tbody tr')->count(), 'AC-04-39: at least one weekly trend row.');
    }

    /**
     * AC-04-40: drill down into a specific trainer's stats, and export to
     * CSV.
     */
    public function testSuperAdminDrillsDownIntoOneTrainerAndExportsCsv(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->createLearnPlaylist($trainer, ['title' => 'Drilldown Playlist']);

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $this->client->request('GET', '/super-admin/content/analytics', ['trainer' => $trainer->getId()]);

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', sprintf('Trainer #%d drill-down', $trainer->getId()));

        // The response body is a StreamedResponse — TrainerRsvpListTest's
        // own precedent notes its callback content is not reliably
        // captured via the BrowserKit test client's getContent(); this
        // checks the export is reachable and correctly shaped as a
        // downloadable CSV attachment, matching that established pattern.
        $this->client->request('GET', '/super-admin/content/analytics/export');
        self::assertResponseIsSuccessful();
        self::assertStringStartsWith('text/csv', (string) $this->client->getResponse()->headers->get('Content-Type'));
        self::assertStringContainsString(
            'lppp-analytics.csv',
            (string) $this->client->getResponse()->headers->get('Content-Disposition'),
        );
    }

    /**
     * A trainer must never reach the Super Admin analytics console.
     */
    public function testTrainerCannotAccessSuperAdminAnalytics(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', '/super-admin/content/analytics');

        self::assertResponseStatusCodeSame(403);
    }
}
