<?php

declare(strict_types=1);

namespace App\Tests\Identity;

use App\Identity\Entity\CoachMembership;
use App\Identity\Repository\CoachMembershipRepository;
use App\Identity\Repository\ParentChildLinkRepository;
use App\Identity\Repository\PlayerProfileRepository;
use App\Identity\Service\AvailabilityService;
use App\Identity\Service\CoachAvailabilityConflictChecker;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-01.09 — Player/Parent Sets Availability; US-01.10 — Coach Sets My Times.
 */
final class AvailabilityTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-01-43: a grid of days/time slots, toggle available/not-available or
     * a custom range, save with confirmation.
     */
    public function testPlayerSetsAvailabilityAndSeesASaveConfirmation(): void
    {
        $player = $this->account('player@practiceperfect.test');
        $this->client->loginUser($player);

        $crawler = $this->client->request('GET', '/portal/availability');
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Save availability')->form([
            'availability_grid[day2][isAvailable]' => true,
            'availability_grid[day2][startTime]' => '15:00',
            'availability_grid[day2][endTime]' => '17:00',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/portal/availability');
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', 'Availability saved');

        /** @var PlayerProfileRepository $playerProfiles */
        $playerProfiles = self::getContainer()->get(PlayerProfileRepository::class);
        $selfPlayer = $playerProfiles->findOneForSelfAccount($this->account('player@practiceperfect.test'));
        self::assertNotNull($selfPlayer);

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var AvailabilityService $availabilityService */
        $availabilityService = self::getContainer()->get(AvailabilityService::class);
        $windows = $availabilityService->getPlayerAvailability($selfPlayer);

        self::assertCount(1, $windows);
        self::assertSame(2, $windows[0]->getDayOfWeek());
        self::assertSame('15:00:00', $windows[0]->getStartTime()->format('H:i:s'));
    }

    /**
     * AC-01-44: a parent sets separate availability per child via the
     * profile switcher (identity_portal_context_child_switch).
     */
    public function testParentSetsSeparateAvailabilityPerChild(): void
    {
        $parent = $this->account('player@practiceperfect.test');
        $this->client->loginUser($parent);

        // Switch to Alex, then save availability for Alex specifically.
        $familyCrawler = $this->client->request('GET', '/portal/family');
        $switchForm = $familyCrawler->selectButton('Switch to Alex')->form();
        $this->client->submit($switchForm);

        $crawler = $this->client->request('GET', '/portal/availability');
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('p', 'Alex');

        $form = $crawler->selectButton('Save availability')->form([
            'availability_grid[day3][isAvailable]' => true,
            'availability_grid[day3][startTime]' => '16:00',
            'availability_grid[day3][endTime]' => '18:00',
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects();

        /** @var ParentChildLinkRepository $links */
        $links = self::getContainer()->get(ParentChildLinkRepository::class);
        $alexLink = current(array_filter(
            $links->findByParent($this->account('player@practiceperfect.test')),
            static fn ($l) => 'Alex' === $l->getChildPlayer()->getFirstName(),
        ));
        self::assertNotFalse($alexLink);

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var AvailabilityService $availabilityService */
        $availabilityService = self::getContainer()->get(AvailabilityService::class);
        $alexWindows = $availabilityService->getPlayerAvailability($alexLink->getChildPlayer());

        self::assertCount(1, $alexWindows, "AC-01-44: Alex's own availability was saved, separate from the parent's.");
        self::assertSame(3, $alexWindows[0]->getDayOfWeek());

        /** @var PlayerProfileRepository $playerProfiles */
        $playerProfiles = self::getContainer()->get(PlayerProfileRepository::class);
        $selfPlayer = $playerProfiles->findOneForSelfAccount($this->account('player@practiceperfect.test'));
        self::assertNotNull($selfPlayer);
        $parentWindows = $availabilityService->getPlayerAvailability($selfPlayer);
        // Not necessarily empty — another test in this suite may have saved
        // the parent's own availability too — but AC-01-44's own guarantee
        // is that it holds no day-3 window this test itself never wrote.
        $parentDays = array_map(static fn ($w) => $w->getDayOfWeek(), $parentWindows);
        self::assertNotContains(3, $parentDays, "AC-01-44: the parent's own availability is untouched by the child's save.");
    }

    /**
     * AC-01-45: trainers can filter players by availability at a selected
     * day/time, and see a per-player "Best Times" summary.
     */
    public function testTrainerCanFilterPlayersByAvailabilityAndSeeBestTimes(): void
    {
        $player = $this->account('player@practiceperfect.test');
        $this->client->loginUser($player);
        $crawler = $this->client->request('GET', '/portal/availability');
        $form = $crawler->selectButton('Save availability')->form([
            'availability_grid[day4][isAvailable]' => true,
            'availability_grid[day4][startTime]' => '10:00',
            'availability_grid[day4][endTime]' => '12:00',
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects();

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var PlayerProfileRepository $playerProfiles */
        $playerProfiles = self::getContainer()->get(PlayerProfileRepository::class);
        $selfPlayer = $playerProfiles->findOneForSelfAccount($this->account('player@practiceperfect.test'));
        self::assertNotNull($selfPlayer);

        /** @var AvailabilityService $availabilityService */
        $availabilityService = self::getContainer()->get(AvailabilityService::class);

        $available = $availabilityService->playerIdsAvailableAt(4, new \DateTimeImmutable('1970-01-01 11:00:00'));
        self::assertContains($selfPlayer->getId(), $available, 'AC-01-45: filter players by availability at a selected day/time.');

        $notAvailable = $availabilityService->playerIdsAvailableAt(4, new \DateTimeImmutable('1970-01-01 20:00:00'));
        self::assertNotContains($selfPlayer->getId(), $notAvailable);

        $summary = $availabilityService->bestTimesSummary($selfPlayer);
        self::assertArrayHasKey(4, $summary, 'AC-01-45: a per-player Best Times summary.');
    }

    /**
     * AC-01-46: a coach's recurring weekly schedule, multiple slots per day.
     */
    public function testCoachSetsAWeeklyScheduleWithMultipleSlotsPerDay(): void
    {
        $coach = $this->account('coach@practiceperfect.test');
        $this->client->loginUser($coach);

        $crawler = $this->client->request('GET', '/coach/availability');
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Save my times')->form([
            'coach_availability[day1_slot0][isAvailable]' => true,
            'coach_availability[day1_slot0][startTime]' => '09:00',
            'coach_availability[day1_slot0][endTime]' => '11:00',
            'coach_availability[day1_slot1][isAvailable]' => true,
            'coach_availability[day1_slot1][startTime]' => '15:00',
            'coach_availability[day1_slot1][endTime]' => '17:00',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/coach/availability');

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var CoachMembershipRepository $coachMemberships */
        $coachMemberships = self::getContainer()->get(CoachMembershipRepository::class);
        $membership = $coachMemberships->findOneForAccountInActiveTenant($this->account('coach@practiceperfect.test'));
        self::assertNotNull($membership);

        /** @var AvailabilityService $availabilityService */
        $availabilityService = self::getContainer()->get(AvailabilityService::class);
        $windows = $availabilityService->getCoachAvailability($membership);

        self::assertCount(2, $windows, 'AC-01-46: multiple time slots allowed per day.');
        foreach ($windows as $window) {
            self::assertSame(1, $window->getDayOfWeek());
        }
    }

    /**
     * AC-01-47: the system detects when a proposed slot conflicts with the
     * coach's stated availability, warranting a trainer warning + override
     * reason. See CoachAvailabilityConflictChecker's own docblock for why
     * the trigger (assigning a coach to an Event) is Epic-02's job, not
     * built here — Event and CoachAssignment do not exist yet, and
     * coach_availability_override is itself scheduled for Epic-02's own
     * migration batch per the schema's migration ordering.
     */
    public function testCoachAvailabilityConflictIsDetectedAgainstDeclaredWindows(): void
    {
        $coach = $this->account('coach@practiceperfect.test');
        $this->client->loginUser($coach);
        $crawler = $this->client->request('GET', '/coach/availability');
        $form = $crawler->selectButton('Save my times')->form([
            'coach_availability[day1_slot0][isAvailable]' => true,
            'coach_availability[day1_slot0][startTime]' => '09:00',
            'coach_availability[day1_slot0][endTime]' => '11:00',
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects();

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var CoachMembershipRepository $coachMemberships */
        $coachMemberships = self::getContainer()->get(CoachMembershipRepository::class);
        $membership = $coachMemberships->findOneForAccountInActiveTenant($this->account('coach@practiceperfect.test'));
        self::assertNotNull($membership);

        /** @var AvailabilityService $availabilityService */
        $availabilityService = self::getContainer()->get(AvailabilityService::class);
        $windows = $availabilityService->getCoachAvailability($membership);

        $checker = new CoachAvailabilityConflictChecker();

        // Inside the declared window: no conflict.
        self::assertFalse($checker->conflictsWith(
            $windows,
            1,
            new \DateTimeImmutable('1970-01-01 09:30:00'),
            new \DateTimeImmutable('1970-01-01 10:30:00'),
        ), 'A proposed slot inside the declared window must not warn.');

        // Outside the declared window: AC-01-47's conflict.
        self::assertTrue($checker->conflictsWith(
            $windows,
            1,
            new \DateTimeImmutable('1970-01-01 14:00:00'),
            new \DateTimeImmutable('1970-01-01 15:00:00'),
        ), 'AC-01-47: a proposed slot outside declared availability must warn the trainer.');

        // No availability declared for this day at all: also a conflict.
        self::assertTrue($checker->conflictsWith($windows, 2, new \DateTimeImmutable('1970-01-01 09:30:00'), new \DateTimeImmutable('1970-01-01 10:30:00')));
    }
}
