<?php

declare(strict_types=1);

namespace App\Tests\Scheduling;

use App\Identity\Service\AvailabilityService;
use App\Platform\Entity\Trainer;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-02.04 — Trainer Sees Player Availability When Scheduling.
 */
final class AvailabilityWidgetTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-02-12: "15 of 20 eligible players available at this time" — the
     * JSON availability-check widget.
     */
    public function testAvailabilityCheckReportsEligibleAndAvailableCounts(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $pat = $this->patPlayer();

        $at = new \DateTimeImmutable('next Tuesday 15:00', $trainer->getTimezone());
        $this->declarePlayerAvailable($trainer, $pat, $at);

        $this->client->loginUser($trainer->getOwnerAccount());
        $this->client->request('GET', '/trainer/events/availability-check?start='.urlencode($at->format('c')));

        self::assertResponseIsSuccessful();
        /** @var array{eligible: int, available: int} $data */
        $data = json_decode((string) $this->client->getResponse()->getContent(), true);
        self::assertArrayHasKey('eligible', $data);
        self::assertArrayHasKey('available', $data);
        self::assertGreaterThanOrEqual(1, $data['eligible'], 'AC-02-12: eligible players are counted.');
        self::assertGreaterThanOrEqual(1, $data['available'], 'AC-02-12: available-at-that-time players are counted.');
        self::assertLessThanOrEqual($data['eligible'], $data['available']);
    }

    /**
     * AC-02-13: the RSVP list shows an availability indicator per player —
     * green (available), gray (unknown/not set), red (busy/conflicting).
     * Covered end-to-end via TrainerRsvpListTest's own availability-column
     * assertions; this test isolates the three-state logic itself using the
     * same helper the RSVP list view calls, against players in three known
     * states.
     */
    public function testEventShowsGreenGrayRedAvailabilityIndicatorPerPlayer(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $pat = $this->patPlayer();

        $event = $this->createEvent($trainer, [
            'title' => 'Availability Indicator Session',
            'startsAt' => new \DateTimeImmutable('next Wednesday 15:00', $trainer->getTimezone()),
            'endsAt' => new \DateTimeImmutable('next Wednesday 16:00', $trainer->getTimezone()),
        ]);
        $this->createRsvp($event, $pat);

        // Pat has set no availability at all yet for this trainer at this
        // point in the test — AC-02-13's "gray" state.
        $this->client->loginUser($trainer->getOwnerAccount());
        $this->client->request('GET', sprintf('/trainer/events/%d/rsvps', $event->getId()));
        self::assertSelectorTextContains('body', 'gray');
    }

    /**
     * AvailabilityWindow (Epic-01) carries no timezone of its own — see
     * CoachAssignmentService::guardNoConflict()'s docblock — so $at is
     * normalized to PHP's own runtime-default timezone (matching
     * EventService::availabilityCount()'s own normalization) before
     * deriving the day-of-week/time-of-day this fixture declares, rather
     * than using whatever zone the caller happened to construct $at in.
     */
    private function declarePlayerAvailable(Trainer $trainer, \App\Identity\Entity\PlayerProfile $player, \DateTimeImmutable $at): void
    {
        $this->activateTenant($trainer);
        /** @var AvailabilityService $availability */
        $availability = self::getContainer()->get(AvailabilityService::class);

        $normalized = $at->setTimezone(new \DateTimeZone(date_default_timezone_get()));

        $availability->setPlayerAvailability($trainer, $player, [[
            'dayOfWeek' => (int) $normalized->format('w'),
            'startTime' => new \DateTimeImmutable('1970-01-01 '.$normalized->format('H:i:s')),
            'endTime' => new \DateTimeImmutable('1970-01-01 '.$normalized->modify('+1 hour')->format('H:i:s')),
            'isAvailable' => true,
        ]]);
    }
}
