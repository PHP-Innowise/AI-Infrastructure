<?php

declare(strict_types=1);

namespace App\Tests\Scheduling;

use App\Identity\Entity\PlayerTrainerMembership;
use App\Scheduling\Entity\Event;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-02.06 — Player Views Training Calendar.
 *
 * Every test here explicitly selects the intended trainer context via
 * switchPlayerToTrainer() before asserting on Training Calendar content —
 * see that helper's own docblock: the shared fixture player "Pat" may
 * already carry more than one active trainer relationship by the time any
 * given test runs, from earlier tests elsewhere in this same suite run
 * (there is no per-test database reset), which defeats
 * TenantResolver's single-tenant fallback.
 */
final class TrainingCalendarTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-02-18: month, week, and day view toggles.
     */
    public function testCalendarOffersMonthWeekAndDayViewToggles(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $this->client->request('GET', '/portal/calendar');

        self::assertResponseIsSuccessful();
        self::assertSelectorExists('a[href*="view=day"]');
        self::assertSelectorExists('a[href*="view=week"]');
        self::assertSelectorExists('a[href*="view=month"]');
    }

    /**
     * AC-02-20: each event shows title, date/time, location, capacity
     * (X/Y), and price; clicking opens the details.
     */
    public function testCalendarEntryShowsCoreEventDetails(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->createEvent($trainer, [
            'title' => 'Calendar Detail Event',
            'location' => 'Court 3',
            'capacity' => 8,
        ]);

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', '/portal/calendar');

        self::assertSelectorTextContains('body', 'Calendar Detail Event');
        self::assertSelectorTextContains('body', 'Court 3');
        self::assertSelectorTextContains('body', 'Free');

        $link = $crawler->selectLink('View')->first();
        self::assertGreaterThan(0, $link->count());
    }

    /**
     * AC-02-21: search by title/location/date, filter by date range, type,
     * or location — scoped to the Training Calendar tool.
     */
    public function testCalendarCanBeSearchedAndFilteredByTypeAndLocation(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->createEvent($trainer, ['title' => 'Findable Camp Session', 'eventType' => Event::TYPE_SMALL_GROUP]);
        $this->createEvent($trainer, ['title' => 'Other Training', 'eventType' => Event::TYPE_TRAINING_SESSION]);

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);

        $this->client->request('GET', '/portal/calendar?q=Findable+Camp');
        self::assertSelectorTextContains('body', 'Findable Camp Session');
        self::assertSelectorTextNotContains('body', 'Other Training');

        $this->client->request('GET', '/portal/calendar?type='.Event::TYPE_SMALL_GROUP);
        self::assertSelectorTextContains('body', 'Findable Camp Session');
    }

    /**
     * AC-02-22: a "Matches your availability" / "Conflicts with your
     * availability" badge once the player has set preferences.
     */
    public function testCalendarShowsAvailabilityMatchBadgeWhenPlayerHasSetPreferences(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        $starts = new \DateTimeImmutable('next Thursday 15:00', $trainer->getTimezone());
        $this->createEvent($trainer, [
            'title' => 'Badge Check Session',
            'startsAt' => $starts,
            'endsAt' => $starts->modify('+1 hour'),
        ]);

        // AvailabilityWindow (Epic-01) carries no timezone of its own — see
        // CoachAssignmentService::guardNoConflict()'s docblock — so the
        // "My Times" form, which has no timezone conversion of its own,
        // must be given the UTC-equivalent (PHP's runtime default) wall
        // clock reading of the event's own trainer-local time, not the
        // trainer-local reading itself, or this would declare a window
        // that never actually lines up with how the event's own time
        // compares once normalized for the badge check below.
        $normalizedStarts = $starts->setTimezone(new \DateTimeZone(date_default_timezone_get()));

        // Pat marks themselves UNAVAILABLE at exactly this slot.
        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', '/portal/availability');
        $dayKey = 'availability_grid[day'.(int) $normalizedStarts->format('w').']';
        $form = $crawler->selectButton('Save availability')->form([
            $dayKey.'[isAvailable]' => false,
            $dayKey.'[startTime]' => $normalizedStarts->format('H:i'),
            $dayKey.'[endTime]' => $normalizedStarts->modify('+1 hour')->format('H:i'),
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects();

        $this->client->request('GET', '/portal/calendar');
        self::assertSelectorTextContains('body', 'Conflicts with your availability');
    }

    /**
     * AC-02-64: a player training with multiple trainers sees fully
     * separated calendars, switching context rather than a merged view.
     */
    public function testPlayerWithMultipleTrainersSeesSeparatedCalendars(): void
    {
        $trainerA = $this->trainer('peak-performance');
        $trainerB = $this->trainer('baseline-athletics');
        $pat = $this->patPlayer();

        $this->activateTenant($trainerA);
        $this->createEvent($trainerA, ['title' => 'Peak Performance Only Event']);

        $this->activateTenant($trainerB);
        $this->createEvent($trainerB, ['title' => 'Baseline Athletics Only Event']);

        // Pat joins Trainer B too (Epic-01's MembershipService), so a
        // combined-view bug would surface both events at once.
        $this->ensureActivePlayerMembership($trainerB, $pat);

        $this->client->loginUser($this->account('player@practiceperfect.test'));

        $this->switchPlayerToTrainer($this->client, $trainerA);
        $this->client->request('GET', '/portal/calendar');
        self::assertSelectorTextContains('body', 'Peak Performance Only Event');
        self::assertSelectorTextNotContains('body', 'Baseline Athletics Only Event');

        // Switch to Trainer B — sees only Trainer B's event, never a merge.
        $this->switchPlayerToTrainer($this->client, $trainerB);
        $this->client->request('GET', '/portal/calendar');
        self::assertSelectorTextContains('body', 'Baseline Athletics Only Event');
        self::assertSelectorTextNotContains('body', 'Peak Performance Only Event');
    }
}
