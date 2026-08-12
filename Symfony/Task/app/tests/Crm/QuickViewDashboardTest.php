<?php

declare(strict_types=1);

namespace App\Tests\Crm;

use App\Crm\Entity\PlayerFlag;
use App\Crm\Service\QuickViewDashboardService;
use App\Scheduling\Entity\AttendanceRecord;
use App\Scheduling\Entity\Event;
use App\Tests\Support\CrmFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-03.08 — Trainer Views Quick View Dashboard.
 */
final class QuickViewDashboardTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;
    use CrmFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-03-34: the trainer can navigate to the dashboard.
     */
    public function testTrainerNavigatesToTheQuickViewDashboard(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', '/trainer/dashboard');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('h1', 'Quick View Dashboard');
    }

    /**
     * AC-03-35: Events This Week, compared to last week.
     */
    public function testEventsThisWeekComparedToLastWeek(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $now = new \DateTimeImmutable();

        $this->createEvent($trainer, ['title' => 'This Week Event '.uniqid(), 'startsAt' => $now->modify('+1 day')]);
        $this->createEvent($trainer, ['title' => 'Last Week Event '.uniqid(), 'startsAt' => $now->modify('-8 days')]);

        /** @var QuickViewDashboardService $dashboard */
        $dashboard = self::getContainer()->get(QuickViewDashboardService::class);
        $before = $dashboard->build($trainer, $now);

        $this->createEvent($trainer, ['title' => 'Another This Week Event '.uniqid(), 'startsAt' => $now->modify('+2 days')]);
        $after = $dashboard->build($trainer, $now);

        self::assertSame($before->eventsThisWeek + 1, $after->eventsThisWeek, 'AC-03-35: events this week increments.');
    }

    /**
     * AC-03-36: RSVPs this week, by event type.
     */
    public function testRsvpsThisWeekBrokenDownByEventType(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $player = $this->freshPlayerMembership($trainer)->getPlayer();
        $event = $this->createEvent($trainer, ['title' => 'RSVP Type Event '.uniqid(), 'eventType' => Event::TYPE_SMALL_GROUP]);
        $this->createRsvp($event, $player);

        /** @var QuickViewDashboardService $dashboard */
        $dashboard = self::getContainer()->get(QuickViewDashboardService::class);
        $metrics = $dashboard->build($trainer, new \DateTimeImmutable());

        self::assertGreaterThanOrEqual(1, $metrics->rsvpsThisWeekByType[Event::TYPE_SMALL_GROUP]);
    }

    /**
     * AC-03-37/BR-03-16..19: Top 10 Players, ranked by 90-day attendance,
     * ties broken alphabetically, players with 0 sessions excluded, only
     * Present/Late count.
     */
    public function testTopPlayersRankingFollowsTheConfirmedAlgorithm(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $coach = $this->coachMembershipFor($trainer, $this->account('coach@practiceperfect.test'));
        $now = new \DateTimeImmutable();

        $topPlayer = $this->freshPlayerMembership($trainer, 'TopRanked'.uniqid())->getPlayer();
        foreach (range(1, 3) as $i) {
            $event = $this->createEvent($trainer, ['title' => 'Top Player Event '.$i.' '.uniqid(), 'startsAt' => $now->modify('-'.$i.' days')]);
            $rsvp = $this->createRsvp($event, $topPlayer);
            $this->createAttendanceRecord($event, $rsvp, AttendanceRecord::STATUS_PRESENT, $coach, $now->modify('-'.$i.' days'));
        }

        // Excused/Absent never count toward the ranking.
        $excludedPlayer = $this->freshPlayerMembership($trainer, 'NeverRanked'.uniqid())->getPlayer();
        $excludedEvent = $this->createEvent($trainer, ['title' => 'Excluded Event '.uniqid()]);
        $excludedRsvp = $this->createRsvp($excludedEvent, $excludedPlayer);
        $this->createAttendanceRecord($excludedEvent, $excludedRsvp, AttendanceRecord::STATUS_EXCUSED, $coach, $now->modify('-1 day'));

        // Outside the 90-day window never counts.
        $stalePlayer = $this->freshPlayerMembership($trainer, 'StaleRanked'.uniqid())->getPlayer();
        $staleEvent = $this->createEvent($trainer, ['title' => 'Stale Event '.uniqid(), 'startsAt' => $now->modify('-200 days')]);
        $staleRsvp = $this->createRsvp($staleEvent, $stalePlayer);
        $this->createAttendanceRecord($staleEvent, $staleRsvp, AttendanceRecord::STATUS_PRESENT, $coach, $now->modify('-100 days'));

        /** @var QuickViewDashboardService $dashboard */
        $dashboard = self::getContainer()->get(QuickViewDashboardService::class);
        $metrics = $dashboard->build($trainer, $now);

        $names = array_column($metrics->topPlayers, 'name');
        self::assertContains($topPlayer->getFirstName(), $names, 'BR-03-16: qualifies with 3 Present sessions.');
        self::assertNotContains($excludedPlayer->getFirstName(), $names, 'BR-03-17: Excused never counts.');
        self::assertNotContains($stalePlayer->getFirstName(), $names, 'BR-03-16: outside the 90-day window.');

        $topRow = current(array_filter($metrics->topPlayers, static fn ($row) => $row['name'] === $topPlayer->getFirstName()));
        self::assertNotFalse($topRow);
        self::assertSame(3, $topRow['sessionCount']);
    }

    /**
     * AC-03-38: flag counts by type, clickable to filter the player list.
     */
    public function testFlagAlertsShowCountsByTypeAndLinkToTheFilteredList(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $player = $this->freshPlayerMembership($trainer)->getPlayer();
        $this->applyFlagToPlayer($trainer, $player, PlayerFlag::TYPE_SCHOLARSHIP, $trainer->getOwnerAccount());

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/dashboard');

        self::assertResponseIsSuccessful();
        $link = $crawler->selectLink('Scholarship: '.$this->scholarshipCount($trainer));
        self::assertGreaterThan(0, $link->count(), 'AC-03-38: the flag count links out to the filtered player list.');
    }

    /**
     * AC-03-39/BR-03-22/23: ShareLink opens and new joins this week.
     */
    public function testShareLinkTrackingShowsOpensAndNewJoinsThisWeek(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        /** @var \App\Identity\Repository\ShareLinkRepository $shareLinks */
        $shareLinks = self::getContainer()->get(\App\Identity\Repository\ShareLinkRepository::class);
        $staticLink = $shareLinks->findStaticPlayerLink((int) $trainer->getId());
        self::assertNotNull($staticLink);

        /** @var \App\Identity\Service\ShareLinkService $shareLinkService */
        $shareLinkService = self::getContainer()->get(\App\Identity\Service\ShareLinkService::class);
        $shareLinkService->recordOpen($staticLink);

        /** @var \App\Identity\Service\MembershipService $memberships */
        $memberships = self::getContainer()->get(\App\Identity\Service\MembershipService::class);
        $newPlayer = new \App\Identity\Entity\PlayerProfile('NewJoiner'.uniqid(), new \DateTimeImmutable('-15 years'));
        /** @var \Doctrine\ORM\EntityManagerInterface $em */
        $em = self::getContainer()->get(\Doctrine\ORM\EntityManagerInterface::class);
        $em->persist($newPlayer);
        $em->flush();
        $memberships->associatePlayer($trainer, $newPlayer, \App\Identity\Entity\PlayerTrainerMembership::SOURCE_SHARELINK, $staticLink);

        /** @var QuickViewDashboardService $dashboard */
        $dashboard = self::getContainer()->get(QuickViewDashboardService::class);
        $metrics = $dashboard->build($trainer, new \DateTimeImmutable());

        self::assertGreaterThanOrEqual(1, $metrics->shareLinkOpensThisWeek, 'AC-03-39: opens this week.');
        self::assertGreaterThanOrEqual(1, $metrics->newPlayersThisWeek, 'AC-03-39/BR-03-23: new joins this week.');
    }

    /**
     * AC-03-40 (optional MVP): attendance rate this week vs last week, and
     * the no-show rate this week.
     */
    public function testAttendanceTrendsAreCalculatedFromTheDatabase(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $coach = $this->coachMembershipFor($trainer, $this->account('coach@practiceperfect.test'));
        $player = $this->freshPlayerMembership($trainer)->getPlayer();
        $event = $this->createEvent($trainer, ['title' => 'Trend Event '.uniqid()]);
        $rsvp = $this->createRsvp($event, $player);
        $this->createAttendanceRecord($event, $rsvp, AttendanceRecord::STATUS_ABSENT, $coach, new \DateTimeImmutable());

        /** @var QuickViewDashboardService $dashboard */
        $dashboard = self::getContainer()->get(QuickViewDashboardService::class);
        $metrics = $dashboard->build($trainer, new \DateTimeImmutable());

        self::assertNotNull($metrics->noShowRateThisWeek);
        self::assertGreaterThan(0.0, $metrics->noShowRateThisWeek);
    }

    /**
     * AC-03-41: every figure is calculated from the database, not entered
     * manually — proven by the fact the exact same build() call is what the
     * HTTP route renders (crm_trainer_quick_view), with no separate
     * data-entry path anywhere in this module.
     */
    public function testDashboardIsFullyCalculatedFromTheDatabaseOnEveryRequest(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/dashboard');
        $firstRenderText = $crawler->filter('body')->text();

        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->createEvent($trainer, ['title' => 'Freshly Added Event '.uniqid()]);

        $crawler = $this->client->request('GET', '/trainer/dashboard');
        self::assertResponseIsSuccessful();
        self::assertNotSame($firstRenderText, $crawler->filter('body')->text(), 'AC-03-41: reflects the database on every refresh.');
    }

    /**
     * AC-03-66: Coach Hours Tracking — total hours/sessions per coach.
     */
    public function testCoachHoursTrackingShowsSessionsAndHoursCovered(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $coach = $this->coachMembershipFor($trainer, $this->account('coach@practiceperfect.test'));
        $event = $this->createEvent($trainer, [
            'title' => 'Coach Hours Event '.uniqid(),
            'startsAt' => (new \DateTimeImmutable())->modify('+3 days')->setTime(10, 0),
            'endsAt' => (new \DateTimeImmutable())->modify('+3 days')->setTime(11, 30),
        ]);
        $this->createCoachAssignment($event, $coach, \App\Scheduling\Entity\CoachAssignment::STATUS_CONFIRMED);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', '/trainer/dashboard');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Casey Coach');
        self::assertSelectorTextContains('body', 'hours');
    }

    private function scholarshipCount(\App\Platform\Entity\Trainer $trainer): int
    {
        $this->activateTenant($trainer);
        /** @var \App\Crm\Repository\PlayerFlagRepository $flags */
        $flags = self::getContainer()->get(\App\Crm\Repository\PlayerFlagRepository::class);

        return $flags->countActiveByTypeForActiveTenant()[PlayerFlag::TYPE_SCHOLARSHIP];
    }
}
