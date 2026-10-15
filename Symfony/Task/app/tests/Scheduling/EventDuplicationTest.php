<?php

declare(strict_types=1);

namespace App\Tests\Scheduling;

use App\Scheduling\Entity\Event;
use App\Scheduling\Repository\EventDuplicationRecordRepository;
use App\Scheduling\Repository\EventRepository;
use App\Scheduling\Repository\RsvpRepository;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-02.05 — Trainer Duplicates Event.
 */
final class EventDuplicationTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-02-14: the duplicate modal is pre-filled with all original event
     * details.
     */
    public function testDuplicateFormIsPreFilledWithOriginalDetails(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $original = $this->createEvent($trainer, [
            'title' => 'Original Skills Session',
            'location' => 'Court 7',
            'capacity' => 6,
            'description' => 'Footwork and ball-handling drills.',
        ]);

        $this->client->loginUser($trainer->getOwnerAccount());
        $this->client->request('GET', sprintf('/trainer/events/%d/duplicate', $original->getId()));

        self::assertResponseIsSuccessful();
        self::assertInputValueSame('event[title]', 'Original Skills Session');
        self::assertInputValueSame('event[location]', 'Court 7');
        self::assertInputValueSame('event[capacity]', '6');
    }

    /**
     * AC-02-15: the date/time must be changed (required); other fields are
     * optional to change.
     */
    public function testDuplicateRequiresTheDateTimeToChange(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $original = $this->createEvent($trainer, ['title' => 'Unchanged Time Attempt']);

        $this->client->loginUser($trainer->getOwnerAccount());
        // Submit with NO changes at all — the pre-filled date/time round-trips unchanged.
        $crawler = $this->client->request('GET', sprintf('/trainer/events/%d/duplicate', $original->getId()));
        $form = $crawler->selectButton('Create duplicate')->form();
        $this->client->submit($form);

        self::assertResponseIsUnprocessable();
        self::assertSelectorTextContains('body', 'must be changed');
    }

    /**
     * AC-02-16: saving creates a new event with a new identifier; the
     * original is unchanged; RSVPs are not copied — the new event starts
     * with none.
     */
    public function testDuplicatingCreatesANewEventWithoutCopyingRsvps(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $original = $this->createEvent($trainer, ['title' => 'Has RSVPs Already', 'capacity' => 5]);
        $this->createRsvp($original, $this->patPlayer());

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', sprintf('/trainer/events/%d/duplicate', $original->getId()));
        $form = $crawler->selectButton('Create duplicate')->form([
            'event[startsAt]' => (new \DateTimeImmutable('+9 days'))->format('Y-m-d\TH:i'),
            'event[endsAt]' => (new \DateTimeImmutable('+9 days +1 hour'))->format('Y-m-d\TH:i'),
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();
        $location = $this->client->getResponse()->headers->get('Location');
        self::assertNotNull($location);
        self::assertMatchesRegularExpression('#/trainer/events/(\d+)#', $location);
        preg_match('#/trainer/events/(\d+)#', $location, $matches);
        $newEventId = (int) ($matches[1] ?? 0);
        self::assertNotSame($original->getId(), $newEventId, 'AC-02-16: a new identifier.');

        $this->activateTenant($trainer);
        /** @var EventRepository $events */
        $events = self::getContainer()->get(EventRepository::class);
        $originalReloaded = $events->find($original->getId());
        self::assertNotNull($originalReloaded);
        self::assertSame('Has RSVPs Already', $originalReloaded->getTitle(), 'AC-02-16: the original is untouched.');

        /** @var RsvpRepository $rsvps */
        $rsvps = self::getContainer()->get(RsvpRepository::class);
        $newEvent = $events->find($newEventId);
        self::assertNotNull($newEvent);
        self::assertCount(0, $rsvps->findForEvent($newEvent), 'AC-02-16: RSVPs are not copied.');
    }

    /**
     * BR-02-19: a reference to the original event is stored for analytics.
     */
    public function testDuplicationRecordReferencesTheOriginalEvent(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $original = $this->createEvent($trainer, ['title' => 'Analytics Source Event']);

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', sprintf('/trainer/events/%d/duplicate', $original->getId()));
        $form = $crawler->selectButton('Create duplicate')->form([
            'event[startsAt]' => (new \DateTimeImmutable('+10 days'))->format('Y-m-d\TH:i'),
            'event[endsAt]' => (new \DateTimeImmutable('+10 days +1 hour'))->format('Y-m-d\TH:i'),
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var EventDuplicationRecordRepository $records */
        $records = self::getContainer()->get(EventDuplicationRecordRepository::class);
        $found = $records->findByOriginalEvent($original);
        self::assertNotEmpty($found, 'BR-02-19: a duplication record references the original.');
    }

    /**
     * AC-02-17: the new duplicated event appears in the calendar for
     * eligible players.
     */
    public function testDuplicatedEventAppearsInThePlayerCalendar(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $original = $this->createEvent($trainer, ['title' => 'Duplicate Source For Calendar']);

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', sprintf('/trainer/events/%d/duplicate', $original->getId()));
        $form = $crawler->selectButton('Create duplicate')->form([
            'event[title]' => 'Duplicated Calendar Entry',
            'event[startsAt]' => (new \DateTimeImmutable('+11 days'))->format('Y-m-d\TH:i'),
            'event[endsAt]' => (new \DateTimeImmutable('+11 days +1 hour'))->format('Y-m-d\TH:i'),
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects();

        $this->client->request('GET', '/logout');
        $this->client->loginUser($this->account('player@practiceperfect.test'));
        // The shared fixture player may carry more than one active trainer
        // relationship by the time this test runs in the full suite — see
        // switchPlayerToTrainer()'s own docblock — so the trainer context
        // must be selected explicitly rather than relying on
        // TenantResolver's single-tenant fallback.
        $this->switchPlayerToTrainer($this->client, $trainer);
        $this->client->request('GET', '/portal/calendar');

        self::assertSelectorTextContains('body', 'Duplicated Calendar Entry');
    }
}
