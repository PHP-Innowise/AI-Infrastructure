<?php

declare(strict_types=1);

namespace App\Tests\Scheduling;

use App\Scheduling\Entity\Event;
use App\Scheduling\Repository\EventRepository;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-02.01 — Trainer Creates Training Event.
 */
final class EventCreationTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-02-1: required fields (title, event type, start/end, location,
     * capacity) plus optional fields (description, eligibility, pricing,
     * coach). AC-02-2: saving creates the event, shows a success message,
     * and it appears in the trainer's own Event Builder list.
     */
    public function testTrainerCreatesATrainingEventWithRequiredAndOptionalFields(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/trainer/events/new');
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Create event')->form([
            'event[title]' => 'Skills Camp',
            'event[eventType]' => Event::TYPE_SMALL_GROUP,
            'event[startsAt]' => (new \DateTimeImmutable('+3 days'))->format('Y-m-d\TH:i'),
            'event[endsAt]' => (new \DateTimeImmutable('+3 days +2 hours'))->format('Y-m-d\TH:i'),
            'event[location]' => 'Main Gym',
            'event[capacity]' => '15',
            'event[description]' => 'A skills-building camp for intermediate players.',
            'event[minAge]' => '8',
            'event[maxAge]' => '14',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', 'Event created successfully');

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var EventRepository $events */
        $events = self::getContainer()->get(EventRepository::class);
        $created = current(array_filter($events->findAllForActiveTenant(), static fn (Event $e): bool => 'Skills Camp' === $e->getTitle()));

        self::assertNotFalse($created, 'AC-02-2: the event appears in the trainer\'s own Event Builder list.');
        self::assertSame(Event::TYPE_SMALL_GROUP, $created->getEventType());
        self::assertSame('Main Gym', $created->getLocation());
        self::assertSame(15, $created->getCapacity());
        self::assertSame(8, $created->getMinAge());
        self::assertSame(14, $created->getMaxAge());
    }

    /**
     * AC-02-3/BR-02-1: title required, max 100 characters.
     */
    public function testEventTitleIsRequiredAndCappedAt100Characters(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/trainer/events/new');
        $form = $crawler->selectButton('Create event')->form([
            'event[title]' => str_repeat('a', 101),
            'event[eventType]' => Event::TYPE_TRAINING_SESSION,
            'event[startsAt]' => (new \DateTimeImmutable('+3 days'))->format('Y-m-d\TH:i'),
            'event[endsAt]' => (new \DateTimeImmutable('+3 days +1 hour'))->format('Y-m-d\TH:i'),
            'event[location]' => 'Main Gym',
            'event[capacity]' => '10',
        ]);
        $this->client->submit($form);

        self::assertResponseIsUnprocessable();
        self::assertSelectorExists('form');
    }

    /**
     * AC-02-3/BR-02-2: a start date/time in the past is rejected.
     */
    public function testEventStartDateTimeCannotBeInThePast(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/trainer/events/new');
        $form = $crawler->selectButton('Create event')->form([
            'event[title]' => 'Yesterday Session',
            'event[eventType]' => Event::TYPE_TRAINING_SESSION,
            'event[startsAt]' => (new \DateTimeImmutable('-1 day'))->format('Y-m-d\TH:i'),
            'event[endsAt]' => (new \DateTimeImmutable('+1 hour'))->format('Y-m-d\TH:i'),
            'event[location]' => 'Main Gym',
            'event[capacity]' => '10',
        ]);
        $this->client->submit($form);

        self::assertResponseIsUnprocessable();
        self::assertSelectorTextContains('body', 'cannot be in the past');

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var EventRepository $events */
        $events = self::getContainer()->get(EventRepository::class);
        self::assertEmpty(array_filter($events->findAllForActiveTenant(), static fn (Event $e): bool => 'Yesterday Session' === $e->getTitle()));
    }

    /**
     * AC-02-3/BR-02-2: end time must be after start time.
     */
    public function testEventEndTimeMustBeAfterStartTime(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $start = new \DateTimeImmutable('+3 days');
        $crawler = $this->client->request('GET', '/trainer/events/new');
        $form = $crawler->selectButton('Create event')->form([
            'event[title]' => 'Backwards Event',
            'event[eventType]' => Event::TYPE_TRAINING_SESSION,
            'event[startsAt]' => $start->format('Y-m-d\TH:i'),
            'event[endsAt]' => $start->modify('-30 minutes')->format('Y-m-d\TH:i'),
            'event[location]' => 'Main Gym',
            'event[capacity]' => '10',
        ]);
        $this->client->submit($form);

        self::assertResponseIsUnprocessable();
        self::assertSelectorTextContains('body', 'must be after');
    }

    /**
     * AC-02-3/BR-02-4: capacity must be > 0.
     */
    public function testEventCapacityMustBePositive(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/trainer/events/new');
        $form = $crawler->selectButton('Create event')->form([
            'event[title]' => 'Zero Capacity',
            'event[eventType]' => Event::TYPE_TRAINING_SESSION,
            'event[startsAt]' => (new \DateTimeImmutable('+3 days'))->format('Y-m-d\TH:i'),
            'event[endsAt]' => (new \DateTimeImmutable('+3 days +1 hour'))->format('Y-m-d\TH:i'),
            'event[location]' => 'Main Gym',
            'event[capacity]' => '0',
        ]);
        $this->client->submit($form);

        self::assertResponseIsUnprocessable();
    }

    /**
     * AC-02-3/BR-02-3: a paid event requires a price/token amount.
     */
    public function testAPaidEventRequiresAPositiveAmount(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/trainer/events/new');
        $form = $crawler->selectButton('Create event')->form([
            'event[title]' => 'Paid Session No Amount',
            'event[eventType]' => Event::TYPE_TRAINING_SESSION,
            'event[startsAt]' => (new \DateTimeImmutable('+3 days'))->format('Y-m-d\TH:i'),
            'event[endsAt]' => (new \DateTimeImmutable('+3 days +1 hour'))->format('Y-m-d\TH:i'),
            'event[location]' => 'Main Gym',
            'event[capacity]' => '10',
            'event[usdPricingEnabled]' => '1',
            'event[usdPrice]' => '0',
        ]);
        $this->client->submit($form);

        self::assertResponseIsUnprocessable();
        self::assertSelectorTextContains('body', 'amount greater than zero');
    }
}
