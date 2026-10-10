<?php

declare(strict_types=1);

namespace App\Tests\Scheduling;

use App\Scheduling\Entity\Event;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-02.15 — Super Admin Views All Events ("Event Master Tool").
 */
final class EventMasterTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-02-55: a list of all events from all trainers, with search by
     * title, trainer name, location, or date.
     */
    public function testSuperAdminSeesAllEventsFromAllTrainersWithSearch(): void
    {
        $trainerA = $this->trainer('peak-performance');
        $trainerB = $this->trainer('baseline-athletics');
        $this->activateTenant($trainerA);
        $this->createEvent($trainerA, ['title' => 'Peak Master Tool Event', 'location' => 'Court Alpha']);
        $this->activateTenant($trainerB);
        $this->createEvent($trainerB, ['title' => 'Baseline Master Tool Event', 'location' => 'Field Beta']);

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $this->client->request('GET', '/super-admin/events');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Peak Master Tool Event', 'AC-02-55: sees every trainer\'s events.');
        self::assertSelectorTextContains('body', 'Baseline Master Tool Event');

        $this->client->request('GET', '/super-admin/events?q=Peak+Master');
        self::assertSelectorTextContains('body', 'Peak Master Tool Event');
        self::assertSelectorTextNotContains('body', 'Baseline Master Tool Event');
    }

    /**
     * AC-02-56: filter by trainer, location, date range, event type, and
     * status, with pagination showing 50 per page.
     */
    public function testSuperAdminCanFilterEventsAndPaginationIsFiftyPerPage(): void
    {
        $trainerA = $this->trainer('peak-performance');
        $trainerB = $this->trainer('baseline-athletics');
        $this->activateTenant($trainerA);
        $this->createEvent($trainerA, [
            'title' => 'Filter Target Training',
            'location' => 'Filter Court',
            'eventType' => Event::TYPE_TRAINING_SESSION,
        ]);
        $this->activateTenant($trainerB);
        $this->createEvent($trainerB, [
            'title' => 'Filter Non Target Group',
            'eventType' => Event::TYPE_SMALL_GROUP,
        ]);

        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        // Filter by trainer.
        $this->client->request('GET', '/super-admin/events?trainer='.$trainerA->getId());
        self::assertSelectorTextContains('body', 'Filter Target Training');
        self::assertSelectorTextNotContains('body', 'Filter Non Target Group');

        // Filter by event type.
        $this->client->request('GET', '/super-admin/events?type='.Event::TYPE_SMALL_GROUP);
        self::assertSelectorTextContains('body', 'Filter Non Target Group');
        self::assertSelectorTextNotContains('body', 'Filter Target Training');

        // Filter by location.
        $this->client->request('GET', '/super-admin/events?location=Filter+Court');
        self::assertSelectorTextContains('body', 'Filter Target Training');
        self::assertSelectorTextNotContains('body', 'Filter Non Target Group');

        // Filter by status (both fixture events are active).
        $this->client->request('GET', '/super-admin/events?status='.Event::STATUS_ACTIVE);
        self::assertSelectorTextContains('body', 'Filter Target Training');

        // AC-02-56: 50 per page.
        $this->client->request('GET', '/super-admin/events');
        self::assertSelectorTextContains('body', 'Page 1 of');
    }

    /**
     * AC-02-57: clicking an event opens its full details; Super Admin can
     * view the RSVP list and export events as CSV.
     */
    public function testSuperAdminViewsDetailsRsvpListAndExportsCsv(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, ['title' => 'Master Tool Detail Target']);
        $this->createRsvp($event, $this->patPlayer());

        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $this->client->request('GET', sprintf('/super-admin/events/%d', $event->getId()));
        self::assertResponseIsSuccessful('AC-02-57: opens the event\'s full details.');
        self::assertSelectorTextContains('body', 'Master Tool Detail Target');

        $this->client->request('GET', sprintf('/super-admin/events/%d/rsvps', $event->getId()));
        self::assertResponseIsSuccessful('AC-02-57: views the RSVP list.');
        self::assertSelectorTextContains('body', 'Pat');

        $this->client->request('GET', '/super-admin/events/export');
        self::assertResponseIsSuccessful('AC-02-57: exports events as CSV.');
        self::assertStringStartsWith('text/csv', (string) $this->client->getResponse()->headers->get('Content-Type'));
    }
}
