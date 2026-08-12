<?php

declare(strict_types=1);

namespace App\Tests\Content;

use App\Content\Entity\Playlist;
use App\Content\Service\ContentAssignmentResolver;
use App\Content\Service\ContentProgressService;
use App\Tests\Support\ContentFixtureHelpers;
use App\Tests\Support\CrmFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-04.08 — Player Tracks Progress.
 */
final class ProgressTrackingTest extends WebTestCase
{
    use FixtureHelpers;
    use ContentFixtureHelpers;
    use CrmFixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-04-27: overall stats — total playlists assigned, total items
     * completed, total watch time, completion rate. Asserted against
     * `ContentProgressService` directly, with a FRESH player
     * (`freshPlayerMembership()`) rather than the shared fixture "Pat" —
     * this whole suite shares one database with no per-test reset, and Pat
     * accumulates assignments from every other test file that also uses
     * her, which would make an EXACT aggregate count/percentage assertion
     * flaky by construction (see CrmFixtureHelpers::freshPlayerMembership()'s
     * own docblock on exactly this class of problem). The HTTP-level
     * structural check (labels present) still runs, separately, in
     * testProgressDashboardIsReachableOverHttpWithAllStatLabels().
     */
    public function testProgressDashboardComputesOverallStats(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer);
        $player = $membership->getPlayer();

        $playlist = $this->createLearnPlaylist($trainer, ['title' => 'Fresh Stats Playlist']);
        $video1 = $this->createVideoItem($trainer, ['title' => 'Fresh Video One']);
        $video2 = $this->createVideoItem($trainer, ['title' => 'Fresh Video Two']);
        $this->addItemToPlaylist($trainer, $playlist, $video1, 1);
        $this->addItemToPlaylist($trainer, $playlist, $video2, 2);
        $this->assignPlaylistToPlayer($trainer, $playlist, $player, $this->account('trainer@practiceperfect.test'));
        $this->markContentCompleted($trainer, $player, $video1);

        /** @var ContentAssignmentResolver $resolver */
        $resolver = self::getContainer()->get(ContentAssignmentResolver::class);
        /** @var ContentProgressService $progressService */
        $progressService = self::getContainer()->get(ContentProgressService::class);

        $assigned = $resolver->assignedPlaylistsForPlayer($trainer, $player, Playlist::PILLAR_LEARN);
        self::assertCount(1, $assigned, 'Exactly one playlist assigned to this freshly created, uncontaminated player.');

        $stats = $progressService->overallStats($trainer, $player, $assigned);
        self::assertSame(1, $stats['playlistsAssigned']);
        self::assertSame(1, $stats['itemsCompleted']);
        self::assertSame(0.5, $stats['completionRate'], 'AC-04-27: completed/total assigned = 1/2 = 50%.');
    }

    /**
     * AC-04-27: the dashboard renders all four overall-stat labels over
     * HTTP.
     */
    public function testProgressDashboardIsReachableOverHttpWithAllStatLabels(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $this->client->request('GET', '/portal/content/progress');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Playlists assigned');
        self::assertSelectorTextContains('body', 'Items completed');
        self::assertSelectorTextContains('body', 'Watch time');
        self::assertSelectorTextContains('body', 'Completion rate');
    }

    /**
     * AC-04-28: recent activity (recently completed, with pillar and
     * completion date) and incomplete assignments sorted by nearest due
     * date first.
     */
    public function testProgressDashboardShowsRecentActivityAndIncompleteAssignmentsSortedByDueDate(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $pat = $this->patPlayer();
        $actor = $this->account('trainer@practiceperfect.test');

        $completedPlaylist = $this->createLearnPlaylist($trainer, ['title' => 'Completed Long Ago']);
        $completedVideo = $this->createVideoItem($trainer, ['title' => 'Already Watched']);
        $this->addItemToPlaylist($trainer, $completedPlaylist, $completedVideo, 1);
        $this->assignPlaylistToPlayer($trainer, $completedPlaylist, $pat, $actor);
        $this->markContentCompleted($trainer, $pat, $completedVideo);

        $nearDuePlaylist = $this->createLearnPlaylist($trainer, ['title' => 'Due Soon']);
        $nearDueVideo = $this->createVideoItem($trainer, ['title' => 'Not Yet Watched Soon']);
        $this->addItemToPlaylist($trainer, $nearDuePlaylist, $nearDueVideo, 1);
        $this->assignPlaylistToPlayer($trainer, $nearDuePlaylist, $pat, $actor, new \DateTimeImmutable('+2 days'));

        $farDuePlaylist = $this->createLearnPlaylist($trainer, ['title' => 'Due Later']);
        $farDueVideo = $this->createVideoItem($trainer, ['title' => 'Not Yet Watched Later']);
        $this->addItemToPlaylist($trainer, $farDuePlaylist, $farDueVideo, 1);
        $this->assignPlaylistToPlayer($trainer, $farDuePlaylist, $pat, $actor, new \DateTimeImmutable('+10 days'));

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', '/portal/content/progress');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Already Watched');

        $incompleteText = $crawler->filter('body')->text();
        $duePos = strpos($incompleteText, 'Due Soon');
        $laterPos = strpos($incompleteText, 'Due Later');
        self::assertNotFalse($duePos);
        self::assertNotFalse($laterPos);
        self::assertLessThan($laterPos, $duePos, 'AC-04-28: sorted by nearest due date first.');
    }

    /**
     * AC-04-29: for each assigned playlist, a progress bar and count
     * ("3 of 5 items completed").
     */
    public function testProgressDashboardShowsPerPlaylistProgressCount(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createLearnPlaylist($trainer, ['title' => 'Per Playlist Progress']);
        $video1 = $this->createVideoItem($trainer);
        $video2 = $this->createVideoItem($trainer);
        $video3 = $this->createVideoItem($trainer);
        $this->addItemToPlaylist($trainer, $playlist, $video1, 1);
        $this->addItemToPlaylist($trainer, $playlist, $video2, 2);
        $this->addItemToPlaylist($trainer, $playlist, $video3, 3);
        $pat = $this->patPlayer();
        $this->assignPlaylistToPlayer($trainer, $playlist, $pat, $this->account('trainer@practiceperfect.test'));
        $this->markContentCompleted($trainer, $pat, $video1);
        $this->markContentCompleted($trainer, $pat, $video2);

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $this->client->request('GET', '/portal/content/progress');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', '2 of 3 items completed');
    }

    /**
     * AC-04-30: a trainer can view a player's progress from the CRM player
     * detail — assigned playlists, completion status, last activity.
     */
    public function testTrainerViewsPlayerProgressFromCrmPlayerDetail(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createLearnPlaylist($trainer, ['title' => 'CRM Visible Progress']);
        $video = $this->createVideoItem($trainer, ['title' => 'CRM Visible Video']);
        $this->addItemToPlaylist($trainer, $playlist, $video, 1);
        $pat = $this->patPlayer();
        $this->assignPlaylistToPlayer($trainer, $playlist, $pat, $this->account('trainer@practiceperfect.test'));
        $this->markContentCompleted($trainer, $pat, $video);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $membership = $this->membershipForPat($trainer, $pat);
        $this->client->request('GET', sprintf('/trainer/players/%d', $membership->getId()));

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'CRM Visible Progress');
        self::assertSelectorTextContains('body', '1 of 1 items completed');
    }

    private function membershipForPat(\App\Platform\Entity\Trainer $trainer, \App\Identity\Entity\PlayerProfile $pat): \App\Identity\Entity\PlayerTrainerMembership
    {
        /** @var \App\Identity\Repository\PlayerTrainerMembershipRepository $repository */
        $repository = self::getContainer()->get(\App\Identity\Repository\PlayerTrainerMembershipRepository::class);
        $membership = $repository->findOneByTrainerAndPlayer($trainer, $pat);
        self::assertNotNull($membership);

        return $membership;
    }
}
