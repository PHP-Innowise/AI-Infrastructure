<?php

declare(strict_types=1);

namespace App\Tests\Crm;

use App\Identity\Entity\PlayerProfile;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Identity\Service\MembershipService;
use App\Scheduling\Entity\AttendanceRecord;
use App\Tests\Support\CrmFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-03.01 (list), US-03.02 (search), US-03.06 (segmentation/filters).
 */
final class PlayerSegmentationTest extends WebTestCase
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
     * AC-03-1: trainer navigates to "CRM Tools"/"Players" and sees the list.
     * AC-03-3: name, age, gender, skill level, attendance rate, last
     * activity are all shown per row. AC-03-4: clicking opens the detail.
     */
    public function testTrainerViewsThePlayerListWithSummaryColumns(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer, 'ListedPlayer'.uniqid());
        $membership->setSkillLevel('Advanced');
        $this->flush();
        $playerName = $membership->getPlayer()->getFirstName();

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        // Narrowed by search (AC-03-5's 50/page cutoff would otherwise hide
        // this player once enough other tests in this run have added more).
        $this->client->request('GET', '/trainer/players?q='.$playerName);

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', $playerName);
        self::assertSelectorTextContains('body', 'Advanced');

        $this->client->clickLink($playerName);
        self::assertResponseIsSuccessful();
        self::assertRouteSame('crm_trainer_player_show');
    }

    /**
     * AC-03-2/BR-03-1: the roster is populated from all three association
     * sources — ShareLink, event registration, and coach invitation.
     */
    public function testThePlayerListIncludesPlayersFromAllThreeAssociationSources(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        /** @var MembershipService $memberships */
        $memberships = self::getContainer()->get(MembershipService::class);

        // Shared tag so a single narrow search finds only these three, no
        // matter how many other players earlier tests in this whole-suite
        // run (no per-test reset) already added past the 50/page cutoff.
        $tag = 'AssocSrc'.uniqid();

        $viaShareLink = $this->newPlayer('ViaShareLink'.$tag);
        $memberships->associatePlayer($trainer, $viaShareLink, PlayerTrainerMembership::SOURCE_SHARELINK);

        $viaEvent = $this->newPlayer('ViaEvent'.$tag);
        $memberships->associatePlayer($trainer, $viaEvent, PlayerTrainerMembership::SOURCE_EVENT_REGISTRATION);

        $viaCoach = $this->newPlayer('ViaCoach'.$tag);
        $memberships->associatePlayer($trainer, $viaCoach, PlayerTrainerMembership::SOURCE_COACH_INVITE);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/players?q='.$tag);
        $text = $crawler->filter('body')->text();

        self::assertStringContainsString($viaShareLink->getFirstName(), $text);
        self::assertStringContainsString($viaEvent->getFirstName(), $text);
        self::assertStringContainsString($viaCoach->getFirstName(), $text);
    }

    /**
     * AC-03-6: sortable by Name, Last Activity, or Attendance Rate.
     */
    public function testThePlayerListCanBeSortedByName(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        // A shared, otherwise-nonexistent tag narrows the search to just
        // these two, so the 50/page cutoff (AC-03-5) can never hide either
        // one regardless of how many other players already exist.
        $tag = 'SortTag'.uniqid();
        $this->freshPlayerMembership($trainer, 'AAAFirst'.$tag);
        $this->freshPlayerMembership($trainer, 'ZZZLast'.$tag);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/players?sort=name_asc&q='.$tag);
        self::assertResponseIsSuccessful();

        $bodyText = $crawler->filter('body')->text();
        $posA = mb_strpos($bodyText, 'AAAFirst'.$tag);
        $posZ = mb_strpos($bodyText, 'ZZZLast'.$tag);
        self::assertNotFalse($posA);
        self::assertNotFalse($posZ);
        self::assertLessThan($posZ, $posA, 'AC-03-6: name_asc sorts A before Z.');
    }

    /**
     * AC-03-8/AC-03-10: search matches partial player name; scoped to this
     * tool only. AC-03-9: no match shows the exact copy.
     */
    public function testSearchMatchesPartialPlayerNameAndReportsNoMatches(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $unique = 'Johnny'.uniqid();
        $this->freshPlayerMembership($trainer, $unique);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        // Partial match, e.g. "John" finds "Johnny...".
        $this->client->request('GET', '/trainer/players?q='.substr($unique, 0, 6));
        self::assertSelectorTextContains('body', $unique);

        $this->client->request('GET', '/trainer/players?q=NoSuchPlayerNameAtAll12345');
        self::assertSelectorTextContains('body', 'No players found. Try different search terms.');
    }

    /**
     * AC-03-22/BR-03-13: multi-criteria filters combine with AND logic —
     * skill level AND gender must both match.
     */
    public function testFiltersCombineWithAndLogic(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        $matches = $this->freshPlayerMembership($trainer, 'MatchesBoth'.uniqid());
        $matches->setSkillLevel('Elite');
        $this->setGender($matches->getPlayer(), 'female');

        $skillOnly = $this->freshPlayerMembership($trainer, 'SkillOnly'.uniqid());
        $skillOnly->setSkillLevel('Elite');
        $this->setGender($skillOnly->getPlayer(), 'male');
        $this->flush();

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/players?skillLevel=Elite&gender=female');

        self::assertSelectorTextContains('body', $matches->getPlayer()->getFirstName());
        self::assertStringNotContainsString($skillOnly->getPlayer()->getFirstName(), $crawler->filter('body')->text(), 'BR-03-13: AND logic excludes a partial match.');
    }

    /**
     * AC-03-24: Registration Date (date range) and Last Activity bands —
     * Active (within 30 days), Inactive (30-90 days), Churned (>90 days).
     */
    public function testRegistrationDateRangeAndLastActivityBandFilters(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        $recentlyRegistered = $this->freshPlayerMembership($trainer, 'RecentReg'.uniqid());
        $oldRegistered = $this->freshPlayerMembership($trainer, 'OldReg'.uniqid());
        $reflection = new \ReflectionProperty($oldRegistered, 'joinedAt');
        $reflection->setAccessible(true);
        $reflection->setValue($oldRegistered, new \DateTimeImmutable('-200 days'));
        $this->flush();

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        // Registration date range: only the recent one falls within the
        // last 7 days.
        $from = (new \DateTimeImmutable('-7 days'))->format('Y-m-d');
        $crawler = $this->client->request('GET', '/trainer/players?registeredFrom='.$from);
        $text = $crawler->filter('body')->text();
        self::assertStringContainsString($recentlyRegistered->getPlayer()->getFirstName(), $text);
        self::assertStringNotContainsString($oldRegistered->getPlayer()->getFirstName(), $text, 'AC-03-24: registration date range excludes the old registration.');

        // Last Activity band: "Churned" (>90 days) matches only the
        // backdated player, since last_activity_at floors at joined_at.
        $crawler = $this->client->request('GET', '/trainer/players?lastActivityBand=churned');
        $text = $crawler->filter('body')->text();
        self::assertStringContainsString($oldRegistered->getPlayer()->getFirstName(), $text, 'AC-03-24: Churned band (>90 days).');
        self::assertStringNotContainsString($recentlyRegistered->getPlayer()->getFirstName(), $text);
    }

    /**
     * AC-03-25, AC-03-26, BR-03-15: a match count, active filters shown as
     * removable badges with a "Clear all filters" option, and the exact
     * copy for zero matches.
     */
    public function testZeroMatchingFiltersShowsTheExactCopy(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/players?skillLevel=NoSuchSkillLevelXYZ');

        self::assertSelectorTextContains('body', 'No players match your filters. Try adjusting criteria.');
        // AC-03-25: a match count is always shown, zero included.
        self::assertSelectorTextContains('body', '0 players match your filters');
        // AC-03-26: a "Clear all filters" control is always available.
        self::assertSelectorExists('a[href="/trainer/players"]');
        unset($crawler);
    }

    /**
     * AC-03-23: attendance filters — attended > X, attended within last Y
     * days, attendance rate > Z%, no-shows > N. BR-03-14: "No-shows > N"
     * counts only Absent-status attendance_record rows — Excused does not
     * count as a no-show.
     */
    public function testNoShowFilterCountsOnlyAbsentStatus(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $coach = $this->coachMembershipFor($trainer, $this->account('coach@practiceperfect.test'));

        $absentPlayer = $this->freshPlayerMembership($trainer, 'AbsentPlayer'.uniqid());
        $event1 = $this->createEvent($trainer, ['title' => 'NoShow Test Event 1 '.uniqid()]);
        $rsvp1 = $this->createRsvp($event1, $absentPlayer->getPlayer());
        $this->createAttendanceRecord($event1, $rsvp1, AttendanceRecord::STATUS_ABSENT, $coach);

        $excusedPlayer = $this->freshPlayerMembership($trainer, 'ExcusedPlayer'.uniqid());
        $event2 = $this->createEvent($trainer, ['title' => 'NoShow Test Event 2 '.uniqid()]);
        $rsvp2 = $this->createRsvp($event2, $excusedPlayer->getPlayer());
        $this->createAttendanceRecord($event2, $rsvp2, AttendanceRecord::STATUS_EXCUSED, $coach);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/players?noShowsGreaterThan=0');
        $text = $crawler->filter('body')->text();

        self::assertStringContainsString($absentPlayer->getPlayer()->getFirstName(), $text, 'BR-03-14: an Absent record counts as a no-show.');
        self::assertStringNotContainsString($excusedPlayer->getPlayer()->getFirstName(), $text, 'BR-03-14: an Excused record does not count as a no-show.');
    }

    /**
     * AC-03-67 (epic-level, no dedicated search field per US-03.02 — see the
     * coder's final report): "search players by attendance" is satisfied by
     * BR-03-14's attendance FILTERS, proven here via "attended more than X."
     */
    public function testAttendedMoreThanFilterFindsQualifyingPlayers(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $coach = $this->coachMembershipFor($trainer, $this->account('coach@practiceperfect.test'));

        $frequentPlayer = $this->freshPlayerMembership($trainer, 'FrequentPlayer'.uniqid());
        for ($i = 0; $i < 3; ++$i) {
            $event = $this->createEvent($trainer, ['title' => 'Attended Event '.$i.' '.uniqid()]);
            $rsvp = $this->createRsvp($event, $frequentPlayer->getPlayer());
            $this->createAttendanceRecord($event, $rsvp, AttendanceRecord::STATUS_PRESENT, $coach);
        }

        $rarePlayer = $this->freshPlayerMembership($trainer, 'RarePlayer'.uniqid());
        $event = $this->createEvent($trainer, ['title' => 'Single Event '.uniqid()]);
        $rsvp = $this->createRsvp($event, $rarePlayer->getPlayer());
        $this->createAttendanceRecord($event, $rsvp, AttendanceRecord::STATUS_PRESENT, $coach);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/players?attendedMoreThan=2');
        $text = $crawler->filter('body')->text();

        self::assertStringContainsString($frequentPlayer->getPlayer()->getFirstName(), $text);
        self::assertStringNotContainsString($rarePlayer->getPlayer()->getFirstName(), $text);
    }

    /**
     * The task's own explicit requirement: native SQL is permitted for this
     * repository ONLY because RLS covers it, and this must be proven, not
     * assumed — a foreign trainer's players must never appear, no matter
     * which trainer's search is run.
     */
    public function testSegmentationNeverLeaksAForeignTrainersPlayersAcrossTenants(): void
    {
        $trainerA = $this->trainer('peak-performance');
        $this->activateTenant($trainerA);
        $onlyOnA = 'OnlyOnTrainerA'.uniqid();
        $this->freshPlayerMembership($trainerA, $onlyOnA);

        $trainerB = $this->trainer('baseline-athletics');
        $this->activateTenant($trainerB);
        $onlyOnB = 'OnlyOnTrainerB'.uniqid();
        $this->freshPlayerMembership($trainerB, $onlyOnB);

        // Trainer B's own session must never see Trainer A's player.
        $this->client->loginUser($this->account('trainer-b@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/players');
        self::assertResponseIsSuccessful();
        $text = $crawler->filter('body')->text();
        self::assertStringContainsString($onlyOnB, $text, 'Trainer B sees their own player.');
        self::assertStringNotContainsString($onlyOnA, $text, 'Trainer B must never see Trainer A\'s player — RLS + the explicit trainer_id predicate.');

        // And the reverse direction.
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/players');
        $text = $crawler->filter('body')->text();
        self::assertStringContainsString($onlyOnA, $text, 'Trainer A sees their own player.');
        self::assertStringNotContainsString($onlyOnB, $text, 'Trainer A must never see Trainer B\'s player.');

        // Directly against the repository too, with Trainer B's own id but
        // no active RLS session for B (Trainer A's tenant still active) —
        // the explicit trainer_id predicate this repository binds must be
        // the one actually deciding the row set, not an accidental reliance
        // on whichever tenant the session happens to be in.
        /** @var \App\Crm\Repository\PlayerSegmentationRepository $segmentation */
        $segmentation = self::getContainer()->get(\App\Crm\Repository\PlayerSegmentationRepository::class);
        $result = $segmentation->search((int) $trainerB->getId(), new \App\Crm\Dto\SegmentCriteria());
        $names = array_map(static fn ($item) => $item->name, $result['items']);
        self::assertNotContains($onlyOnA, $names, 'The explicit trainer_id predicate must govern the query, not ambient session state alone.');
    }

    private function newPlayer(string $firstName): PlayerProfile
    {
        $player = new PlayerProfile($firstName.' '.uniqid(), new \DateTimeImmutable('-15 years'));
        $this->em()->persist($player);
        $this->em()->flush();

        return $player;
    }

    private function setGender(PlayerProfile $player, string $gender): void
    {
        $reflection = new \ReflectionProperty($player, 'gender');
        $reflection->setAccessible(true);
        $reflection->setValue($player, $gender);
    }

    private function flush(): void
    {
        $this->em()->flush();
    }

    private function em(): EntityManagerInterface
    {
        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);

        return $em;
    }
}
