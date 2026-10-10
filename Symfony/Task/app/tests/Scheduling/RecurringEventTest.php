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
 * "Recurring Events - Bulk Auto-Creation" — In Scope (MVP), Event Creation
 * & Management (Trainer). AC-02-61.
 *
 * Distinct from Duplicate Event (US-02.05) — see
 * specs/requirements-analyst-epic-02-event-management-spec.md's own note on
 * AC-02-61 for the scope-conflict-with-the-plan this resolves the same way
 * every other such conflict in this epic is resolved ("the specs follow the
 * epic").
 */
final class RecurringEventTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-02-61: a recurring pattern bulk-generates independent event
     * instances — e.g. "Every Tuesday," here for 3 occurrences.
     */
    public function testRecurringPatternBulkGeneratesIndependentWeeklyEvents(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $firstStart = new \DateTimeImmutable('next Tuesday 15:00', $trainer->getTimezone());
        $repeatUntil = $firstStart->modify('+14 days');

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', '/trainer/events/new-recurring');
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Create recurring events')->form([
            'recurring_event[title]' => 'Recurring Tuesday Drills',
            'recurring_event[startsAt]' => $firstStart->format('Y-m-d\TH:i'),
            'recurring_event[endsAt]' => $firstStart->modify('+1 hour')->format('Y-m-d\TH:i'),
            'recurring_event[location]' => 'Court Recurring',
            'recurring_event[capacity]' => '8',
            'recurring_event[repeatUntil]' => $repeatUntil->format('Y-m-d'),
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/trainer/events');

        $this->activateTenant($trainer);
        /** @var EventRepository $events */
        $events = self::getContainer()->get(EventRepository::class);
        $generated = array_values(array_filter(
            $events->findAllForActiveTenant(),
            static fn (Event $e): bool => 'Recurring Tuesday Drills' === $e->getTitle(),
        ));

        self::assertCount(3, $generated, 'AC-02-61: week 0, 1, and 2 — 3 independent event instances.');

        $starts = array_map(static fn (Event $e): string => $e->getStartsAt()->setTimezone($trainer->getTimezone())->format('Y-m-d'), $generated);
        sort($starts);
        self::assertSame(
            [
                $firstStart->format('Y-m-d'),
                $firstStart->modify('+7 days')->format('Y-m-d'),
                $firstStart->modify('+14 days')->format('Y-m-d'),
            ],
            $starts,
            'AC-02-61: exactly one week apart, same day and time.',
        );

        // Every generated row is a genuinely distinct Event id — never a
        // shared/linked series row.
        self::assertCount(3, array_unique(array_map(static fn (Event $e): int => (int) $e->getId(), $generated)));
    }

    /**
     * AC-02-61: "each generated event is independent — it can have a
     * different coach and can be edited or canceled separately from the
     * others."
     */
    public function testEachGeneratedEventCanBeEditedOrCanceledIndependently(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $firstStart = new \DateTimeImmutable('next Wednesday 10:00', $trainer->getTimezone());
        $repeatUntil = $firstStart->modify('+7 days');

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', '/trainer/events/new-recurring');
        $form = $crawler->selectButton('Create recurring events')->form([
            'recurring_event[title]' => 'Independent Series Session',
            'recurring_event[startsAt]' => $firstStart->format('Y-m-d\TH:i'),
            'recurring_event[endsAt]' => $firstStart->modify('+1 hour')->format('Y-m-d\TH:i'),
            'recurring_event[location]' => 'Court Independent',
            'recurring_event[capacity]' => '8',
            'recurring_event[repeatUntil]' => $repeatUntil->format('Y-m-d'),
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects('/trainer/events');

        $this->activateTenant($trainer);
        /** @var EventRepository $events */
        $events = self::getContainer()->get(EventRepository::class);
        $generated = array_values(array_filter(
            $events->findAllForActiveTenant(),
            static fn (Event $e): bool => 'Independent Series Session' === $e->getTitle(),
        ));
        self::assertCount(2, $generated);
        usort($generated, static fn (Event $a, Event $b): int => $a->getStartsAt() <=> $b->getStartsAt());
        [$occurrence1, $occurrence2] = $generated;

        // Edit only the first occurrence.
        $editCrawler = $this->client->request('GET', sprintf('/trainer/events/%d/edit', $occurrence1->getId()));
        $editForm = $editCrawler->selectButton('Save changes')->form(['event[title]' => 'Only First Occurrence Renamed']);
        $this->client->submit($editForm);
        self::assertResponseRedirects();

        // Cancel only the second occurrence.
        $cancelCrawler = $this->client->request('GET', sprintf('/trainer/events/%d/cancel', $occurrence2->getId()));
        $cancelForm = $cancelCrawler->selectButton('Cancel event')->form(['cancel_event[reason]' => 'Independent cancellation test.']);
        $this->client->submit($cancelForm);
        self::assertResponseRedirects();

        // Re-fetched fresh: KernelBrowser reboots the kernel (a new
        // EntityManager) on every request by default, and $events was
        // first fetched several requests ago — see
        // TrainerRsvpListTest::testTrainerCanManuallyAddRemoveAndExportRsvps's
        // own note on the exact same pitfall.
        $this->activateTenant($trainer);
        /** @var EventRepository $freshEvents */
        $freshEvents = self::getContainer()->get(EventRepository::class);
        $reloaded1 = $freshEvents->find($occurrence1->getId());
        $reloaded2 = $freshEvents->find($occurrence2->getId());
        self::assertNotNull($reloaded1);
        self::assertNotNull($reloaded2);
        self::assertSame('Only First Occurrence Renamed', $reloaded1->getTitle(), 'AC-02-61: editing one occurrence never touches the other.');
        self::assertFalse($reloaded1->isCanceled());
        self::assertTrue($reloaded2->isCanceled(), 'AC-02-61: canceling one occurrence never touches the other.');
        self::assertSame('Independent Series Session', $reloaded2->getTitle(), 'AC-02-61: the canceled occurrence keeps its own original title — the rename never propagated.');
    }

    /**
     * The "repeat until" date must be on or after the first occurrence.
     */
    public function testRepeatUntilMustNotPrecedeTheFirstOccurrence(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $firstStart = new \DateTimeImmutable('next Thursday 09:00', $trainer->getTimezone());

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', '/trainer/events/new-recurring');
        $form = $crawler->selectButton('Create recurring events')->form([
            'recurring_event[title]' => 'Invalid Repeat Until Session',
            'recurring_event[startsAt]' => $firstStart->format('Y-m-d\TH:i'),
            'recurring_event[endsAt]' => $firstStart->modify('+1 hour')->format('Y-m-d\TH:i'),
            'recurring_event[location]' => 'Court Invalid',
            'recurring_event[capacity]' => '8',
            'recurring_event[repeatUntil]' => $firstStart->modify('-7 days')->format('Y-m-d'),
        ]);
        $this->client->submit($form);

        self::assertResponseIsUnprocessable();

        $this->activateTenant($trainer);
        /** @var EventRepository $events */
        $events = self::getContainer()->get(EventRepository::class);
        self::assertEmpty(array_filter(
            $events->findAllForActiveTenant(),
            static fn (Event $e): bool => 'Invalid Repeat Until Session' === $e->getTitle(),
        ), 'No events are created when the pattern itself is invalid.');
    }
}
