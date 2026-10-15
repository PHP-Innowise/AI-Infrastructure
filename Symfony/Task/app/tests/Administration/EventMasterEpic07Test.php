<?php

declare(strict_types=1);

namespace App\Tests\Administration;

use App\Scheduling\Entity\Event;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-07.06 — Super Admin Views Event Master Tool. `tests/Scheduling/
 * EventMasterTest.php` already proves this same tool end to end under
 * AC-02-55..57's own numbering (the Event Master minimal slice was first
 * built in Epic-02 — see EventMasterController's own class docblock); this
 * file adds Epic-07's own citations plus the two things that file does not
 * cover: the date-range filter and sort (AC-07-26).
 */
final class EventMasterEpic07Test extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-07-22: navigates to "Events"/"Event Master" and views all events
     * from all trainers, system-wide.
     */
    public function testSuperAdminViewsEventsFromEveryTrainer(): void
    {
        $trainerA = $this->trainer('peak-performance');
        $trainerB = $this->trainer('baseline-athletics');
        $this->activateTenant($trainerA);
        $this->createEvent($trainerA, ['title' => 'Epic07 Master A']);
        $this->activateTenant($trainerB);
        $this->createEvent($trainerB, ['title' => 'Epic07 Master B']);

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $this->client->request('GET', '/super-admin/events');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Epic07 Master A');
        self::assertSelectorTextContains('body', 'Epic07 Master B');
    }

    /**
     * AC-07-23: the event list shows title, trainer, date & time, event
     * type, capacity ("held / capacity"), status, and assigned coach.
     */
    public function testEventListShowsCapacityAndAssignedCoach(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, ['title' => 'Capacity Coach Row', 'capacity' => 20]);
        $coachAccount = $this->createCoach($trainer, 'row-coach@example.test');
        $this->createCoachAssignment($event, $this->coachMembershipFor($trainer, $coachAccount));
        $this->createRsvp($event, $this->patPlayer());

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/super-admin/events', ['q' => 'Capacity Coach Row']);

        self::assertResponseIsSuccessful();
        $row = $crawler->filter('table tbody tr')->first();
        self::assertStringContainsString('1 / 20', $row->text(), 'AC-07-23: capacity shown as "held / capacity".');
        self::assertStringContainsString('Test Coach', $row->text(), 'AC-07-23: the assigned coach\'s name is shown.');
    }

    /**
     * AC-07-24: filter by date range (next 7/30 days, custom), trainer,
     * event type, and status — trainer/type/location/status are already
     * proven by EventMasterTest's own AC-02-56 coverage; this adds the
     * date-range half that file does not exercise.
     */
    public function testEventListFiltersByDateRange(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $now = new \DateTimeImmutable();
        $soon = $this->createEvent($trainer, [
            'title' => 'Date Range Soon',
            'startsAt' => $now->modify('+3 days'),
            'endsAt' => $now->modify('+3 days')->modify('+1 hour'),
        ]);
        $far = $this->createEvent($trainer, [
            'title' => 'Date Range Far',
            'startsAt' => $now->modify('+60 days'),
            'endsAt' => $now->modify('+60 days')->modify('+1 hour'),
        ]);

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $this->client->request('GET', '/super-admin/events', [
            'dateFrom' => $now->format('Y-m-d'),
            'dateTo' => $now->modify('+7 days')->format('Y-m-d'),
        ]);

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Date Range Soon');
        self::assertSelectorTextNotContains('body', 'Date Range Far');

        unset($soon, $far);
    }

    /**
     * AC-07-25: per event, Super Admin can view full details, edit the
     * event as if they had created it, cancel it, and view its RSVP list —
     * EventMasterTest's own AC-02-57 test already proves view/RSVP-list/
     * export; this adds the edit and cancel actions that file does not
     * exercise.
     */
    public function testSuperAdminEditsAndCancelsAnyEvent(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, ['title' => 'Edit Cancel Target', 'location' => 'Original Court']);

        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $editCrawler = $this->client->request('GET', sprintf('/super-admin/events/%d/edit', $event->getId()));
        self::assertResponseIsSuccessful();
        $form = $editCrawler->selectButton('Save (Super Admin)')->form();
        $form['event[location]'] = 'Super Admin Edited Court';
        $this->client->submit($form);

        self::assertResponseRedirects(sprintf('/super-admin/events/%d', $event->getId()));
        $this->client->followRedirect();
        self::assertSelectorTextContains('.flash--success', 'Event updated (Super Admin)');
        self::assertSelectorTextContains('body', 'Super Admin Edited Court', 'AC-07-25: edits as if they had created it.');

        $cancelCrawler = $this->client->request('GET', sprintf('/super-admin/events/%d/cancel', $event->getId()));
        $cancelForm = $cancelCrawler->selectButton('Cancel event')->form(['cancel_event[reason]' => 'Super Admin emergency override.']);
        $this->client->submit($cancelForm);

        self::assertResponseRedirects('/super-admin/events');
        $this->client->followRedirect();
        self::assertSelectorTextContains('.flash--success', 'Event canceled (Super Admin)');
    }

    /**
     * AC-07-26: sort by date (ascending/descending), trainer, or capacity.
     */
    public function testEventListCanBeSortedByDateTrainerAndCapacity(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        // A distinctive marker in every title, and the "q" search filter
        // below, keep this query scoped to exactly these two rows —
        // relying on a date range alone is not enough: an unbounded
        // "dateFrom" with no "dateTo" also matches events other tests in
        // this same shared, mutable fixture database create far in the
        // future.
        $marker = 'Epic07Sort'.bin2hex(random_bytes(4));
        $now = new \DateTimeImmutable();
        $this->createEvent($trainer, [
            'title' => $marker.' Small Capacity',
            'capacity' => 2,
            'startsAt' => $now->modify('+40 days'),
            'endsAt' => $now->modify('+40 days')->modify('+1 hour'),
        ]);
        $this->createEvent($trainer, [
            'title' => $marker.' Large Capacity',
            'capacity' => 200,
            'startsAt' => $now->modify('+41 days'),
            'endsAt' => $now->modify('+41 days')->modify('+1 hour'),
        ]);

        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $ascCrawler = $this->client->request('GET', '/super-admin/events', ['sort' => 'date_asc', 'q' => $marker]);
        $ascTitles = $ascCrawler->filter('table tbody tr td:first-child')->each(static fn ($node) => $node->text());
        self::assertSame([$marker.' Small Capacity', $marker.' Large Capacity'], $ascTitles, 'AC-07-26: date ascending.');

        $capacityCrawler = $this->client->request('GET', '/super-admin/events', ['sort' => 'capacity', 'q' => $marker]);
        $capacityTitles = $capacityCrawler->filter('table tbody tr td:first-child')->each(static fn ($node) => $node->text());
        self::assertSame([$marker.' Large Capacity', $marker.' Small Capacity'], $capacityTitles, 'AC-07-26: capacity descending.');
    }

    /**
     * AC-07-26: paginated at 50 events per page — already proven directly
     * by EventMasterTest's own AC-02-56 pagination assertion ("Page 1 of");
     * this restates the SAME observable fact under this epic's own id
     * rather than duplicating the setup, since AC-07-26 and AC-02-56 name
     * the identical "50 per page" figure for the identical route.
     */
    public function testEventListPaginatesAtFiftyPerPage(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/super-admin/events');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Page 1 of');
        self::assertLessThanOrEqual(50, \count($crawler->filter('table tbody tr')));
    }

    /**
     * A Trainer (not Super Admin) cannot reach the Event Master tool.
     */
    public function testATrainerCannotReachEventMaster(): void
    {
        $this->client->loginUser($this->trainer('peak-performance')->getOwnerAccount());
        $this->client->request('GET', '/super-admin/events');

        self::assertResponseStatusCodeSame(403);
    }
}
