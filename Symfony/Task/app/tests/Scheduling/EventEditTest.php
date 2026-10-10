<?php

declare(strict_types=1);

namespace App\Tests\Scheduling;

use App\Scheduling\Entity\CoachAssignment;
use App\Scheduling\Entity\Event;
use App\Scheduling\Entity\Rsvp;
use App\Scheduling\Repository\EventRepository;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\Mime\Email;

/**
 * US-02.14 — Trainer Edits Event.
 */
final class EventEditTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-02-50/52: for a non-past event, every field is editable, and the
     * saved changes are reflected everywhere the event appears (its own
     * details page here; the Training Calendar and My Reservations read
     * from the same Event row, no separate projection to keep in sync).
     */
    public function testTrainerEditsEventFieldsAndTheyAppearEverywhere(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, ['title' => 'Original Title', 'location' => 'Court A', 'capacity' => 8]);

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', sprintf('/trainer/events/%d/edit', $event->getId()));
        $form = $crawler->selectButton('Save changes')->form([
            'event[title]' => 'Updated Title',
            'event[location]' => 'Court B',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects(sprintf('/trainer/events/%d', $event->getId()));

        $this->activateTenant($trainer);
        /** @var EventRepository $events */
        $events = self::getContainer()->get(EventRepository::class);
        $reloaded = $events->find($event->getId());
        self::assertNotNull($reloaded);
        self::assertSame('Updated Title', $reloaded->getTitle(), 'AC-02-50: the title was edited.');
        self::assertSame('Court B', $reloaded->getLocation());

        $this->client->request('GET', sprintf('/trainer/events/%d', $event->getId()));
        self::assertSelectorTextContains('body', 'Updated Title', 'AC-02-52: reflected on the event\'s own details page.');

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $this->client->request('GET', '/portal/calendar');
        self::assertSelectorTextContains('body', 'Updated Title', 'AC-02-52: reflected on the Training Calendar.');
    }

    /**
     * AC-02-51: a date/time or location change notifies every RSVP'd
     * player.
     */
    public function testLocationChangeNotifiesRsvpdPlayers(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, ['title' => 'Location Change Session', 'location' => 'Court A']);
        $this->createRsvp($event, $this->patPlayer());

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', sprintf('/trainer/events/%d/edit', $event->getId()));
        $form = $crawler->selectButton('Save changes')->form(['event[location]' => 'Court Z']);
        $this->client->submit($form);

        self::assertResponseRedirects();
        self::assertQueuedEmailCount(1, message: 'AC-02-51: the RSVP\'d player is notified of the location change.');
    }

    /**
     * AC-02-51: a price increase notifies RSVP'd players and states they
     * may cancel with a refund.
     */
    public function testPriceIncreaseNotifiesPlayersWithCancelOption(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, [
            'title' => 'Price Increase Session',
            'usdPricingEnabled' => true,
            'usdPriceMinorUnits' => 1000,
        ]);
        $this->createRsvp($event, $this->patPlayer(), Rsvp::METHOD_USD, Rsvp::STATUS_CONFIRMED);

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', sprintf('/trainer/events/%d/edit', $event->getId()));
        $form = $crawler->selectButton('Save changes')->form(['event[usdPrice]' => '20']);
        $this->client->submit($form);

        self::assertResponseRedirects();
        self::assertQueuedEmailCount(1);
        $email = self::getMailerMessage(0);
        self::assertInstanceOf(Email::class, $email);
        self::assertStringContainsString('cancel your RSVP for a full refund', (string) $email->getTextBody());
    }

    /**
     * AC-02-51: a coach change notifies both the previous and the new
     * coach.
     */
    public function testCoachChangeNotifiesBothOldAndNewCoach(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $coachA = $this->createCoach($trainer, 'edit-coach-a@practiceperfect.test');
        $coachB = $this->createCoach($trainer, 'edit-coach-b@practiceperfect.test');
        $membershipA = $this->coachMembershipFor($trainer, $coachA);
        $membershipB = $this->coachMembershipFor($trainer, $coachB);
        $event = $this->createEvent($trainer, ['title' => 'Coach Change Session']);
        $this->createCoachAssignment($event, $membershipA, CoachAssignment::STATUS_CONFIRMED);

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', sprintf('/trainer/events/%d/edit', $event->getId()));
        $form = $crawler->selectButton('Save changes')->form([
            'event[coach]' => (string) $membershipB->getId(),
            // CoachB has declared no availability at all — an empty set
            // always conflicts (CoachAvailabilityConflictChecker's own
            // docblock: "nothing on record that covers the proposed
            // slot") — an override reason is required to proceed, same as
            // CoachAssignmentTest's own conflicting-coach case.
            'event[coachOverrideReason]' => 'Reassigning for this test.',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();
        self::assertQueuedEmailCount(2, message: 'AC-02-51: both the old and new coach are notified.');
    }

    /**
     * AC-02-53: increasing capacity opens more spots; decreasing capacity
     * below the current RSVP count is blocked with a warning until
     * resolved.
     */
    public function testCapacityChangeRules(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, ['title' => 'Capacity Rules Session', 'capacity' => 5]);
        $this->createRsvp($event, $this->patPlayer());
        $alex = $this->alexPlayer();
        $this->ensureActivePlayerMembership($trainer, $alex);
        $this->createRsvp($event, $alex);

        $this->client->loginUser($trainer->getOwnerAccount());

        // Increase — always fine.
        $crawler = $this->client->request('GET', sprintf('/trainer/events/%d/edit', $event->getId()));
        $form = $crawler->selectButton('Save changes')->form(['event[capacity]' => '10']);
        $this->client->submit($form);
        self::assertResponseRedirects(sprintf('/trainer/events/%d', $event->getId()));

        // Decrease below the current (2) confirmed RSVPs — blocked.
        $crawler2 = $this->client->request('GET', sprintf('/trainer/events/%d/edit', $event->getId()));
        $form2 = $crawler2->selectButton('Save changes')->form(['event[capacity]' => '1']);
        $this->client->submit($form2);

        self::assertResponseIsUnprocessable();
        self::assertSelectorTextContains('body', 'exceed capacity');

        $this->activateTenant($trainer);
        /** @var EventRepository $events */
        $events = self::getContainer()->get(EventRepository::class);
        $reloaded = $events->find($event->getId());
        self::assertNotNull($reloaded);
        self::assertSame(10, $reloaded->getCapacity(), 'AC-02-53: the blocked save did not apply.');
    }

    /**
     * AC-02-54: events in the past cannot be edited, canceled events
     * cannot be edited, and date/time changes are validated (end after
     * start, not in the past).
     */
    public function testEditValidation(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $past = $this->createEvent($trainer, [
            'title' => 'Past Edit Target',
            'startsAt' => new \DateTimeImmutable('-2 hours'),
            'endsAt' => new \DateTimeImmutable('-1 hour'),
        ]);
        $canceled = $this->createEvent($trainer, ['title' => 'Canceled Edit Target']);
        $canceled->cancel('Testing.', $trainer->getOwnerAccount(), new \DateTimeImmutable());
        $this->entityManagerFlush();

        $editable = $this->createEvent($trainer, ['title' => 'End Before Start Target']);

        $this->client->loginUser($trainer->getOwnerAccount());

        $this->client->request('GET', sprintf('/trainer/events/%d/edit', $past->getId()));
        self::assertResponseRedirects(sprintf('/trainer/events/%d', $past->getId()), message: 'AC-02-54: a past event cannot be edited.');

        $this->client->request('GET', sprintf('/trainer/events/%d/edit', $canceled->getId()));
        self::assertResponseRedirects(sprintf('/trainer/events/%d', $canceled->getId()), message: 'AC-02-54: a canceled event cannot be edited.');

        $crawler = $this->client->request('GET', sprintf('/trainer/events/%d/edit', $editable->getId()));
        $form = $crawler->selectButton('Save changes')->form([
            'event[endsAt]' => (new \DateTimeImmutable('+1 day'))->format('Y-m-d\TH:i'),
            'event[startsAt]' => (new \DateTimeImmutable('+2 days'))->format('Y-m-d\TH:i'),
        ]);
        $this->client->submit($form);
        self::assertResponseIsUnprocessable('AC-02-54: end must be after start.');
    }

    private function entityManagerFlush(): void
    {
        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);
        $em->flush();
    }
}
