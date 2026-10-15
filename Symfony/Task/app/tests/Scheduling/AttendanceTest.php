<?php

declare(strict_types=1);

namespace App\Tests\Scheduling;

use App\Scheduling\Entity\AttendanceRecord;
use App\Scheduling\Entity\CoachAssignment;
use App\Scheduling\Entity\Rsvp;
use App\Scheduling\Repository\AttendanceRecordRepository;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-02.11 — Coach Tracks Attendance.
 */
final class AttendanceTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-02-38: after the event starts, "Take Attendance" becomes
     * available from "My Activities", showing the roster with Present as
     * the default per player who RSVP'd.
     */
    public function testTakeAttendanceAvailableAfterStartAndDefaultsToPresent(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $coachAccount = $this->createCoach($trainer, 'attendance-coach-1@practiceperfect.test');
        $coach = $this->coachMembershipFor($trainer, $coachAccount);
        $event = $this->createEvent($trainer, [
            'title' => 'Started Session For Attendance',
            'startsAt' => new \DateTimeImmutable('-2 hours'),
            'endsAt' => new \DateTimeImmutable('-1 hour'),
        ]);
        $this->createCoachAssignment($event, $coach, CoachAssignment::STATUS_CONFIRMED);
        $this->createRsvp($event, $this->patPlayer());

        $this->client->loginUser($coachAccount);
        $crawler = $this->client->request('GET', '/coach/activities');
        self::assertSelectorExists(sprintf('a[href="/coach/events/%d/attendance"]', $event->getId()), 'AC-02-38: "Take Attendance" is available once started.');

        $attendanceCrawler = $this->client->request('GET', sprintf('/coach/events/%d/attendance', $event->getId()));
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Pat');

        // Submit unchanged — the form's own pre-filled default is Present.
        $form = $attendanceCrawler->selectButton('Save attendance')->form();
        $this->client->submit($form);

        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var AttendanceRecordRepository $records */
        $records = self::getContainer()->get(AttendanceRecordRepository::class);
        $recorded = $records->findForEvent($event);
        self::assertCount(1, $recorded);
        self::assertSame(AttendanceRecord::STATUS_PRESENT, $recorded[0]->getStatus(), 'AC-02-38: Present is the default.');
    }

    /**
     * AC-02-39: the coach selects a status per player and saves — every
     * status is recorded correctly.
     */
    public function testCoachRecordsMixedAttendanceStatuses(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $coachAccount = $this->createCoach($trainer, 'attendance-coach-2@practiceperfect.test');
        $coach = $this->coachMembershipFor($trainer, $coachAccount);
        $alex = $this->alexPlayer();
        $this->ensureActivePlayerMembership($trainer, $alex);
        $event = $this->createEvent($trainer, [
            'title' => 'Mixed Attendance Session',
            'startsAt' => new \DateTimeImmutable('-2 hours'),
            'endsAt' => new \DateTimeImmutable('-1 hour'),
            'capacity' => 5,
        ]);
        $this->createCoachAssignment($event, $coach, CoachAssignment::STATUS_CONFIRMED);
        $this->createRsvp($event, $this->patPlayer());
        $this->createRsvp($event, $alex);

        $this->client->loginUser($coachAccount);
        $crawler = $this->client->request('GET', sprintf('/coach/events/%d/attendance', $event->getId()));

        $this->activateTenant($trainer);
        $patId = (string) $this->patPlayer()->getId();
        $alexId = (string) $alex->getId();

        $form = $crawler->selectButton('Save attendance')->form([
            'take_attendance[player_'.$patId.']' => AttendanceRecord::STATUS_ABSENT,
            'take_attendance[player_'.$alexId.']' => AttendanceRecord::STATUS_LATE,
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var AttendanceRecordRepository $records */
        $records = self::getContainer()->get(AttendanceRecordRepository::class);
        $byPlayer = [];
        foreach ($records->findForEvent($event) as $record) {
            $byPlayer[(int) $record->getPlayer()->getId()] = $record->getStatus();
        }
        self::assertSame(AttendanceRecord::STATUS_ABSENT, $byPlayer[(int) $patId] ?? null, 'AC-02-39: Pat marked Absent.');
        self::assertSame(AttendanceRecord::STATUS_LATE, $byPlayer[(int) $alexId] ?? null, 'AC-02-39: Alex marked Late.');
    }

    /**
     * AC-02-40: recorded attendance is visible to the trainer in the
     * event's own details.
     */
    public function testAttendanceVisibleToTrainerInEventDetails(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $coachAccount = $this->createCoach($trainer, 'attendance-coach-3@practiceperfect.test');
        $coach = $this->coachMembershipFor($trainer, $coachAccount);
        $event = $this->createEvent($trainer, [
            'title' => 'Attendance Visible To Trainer',
            'startsAt' => new \DateTimeImmutable('-2 hours'),
            'endsAt' => new \DateTimeImmutable('-1 hour'),
        ]);
        $this->createCoachAssignment($event, $coach, CoachAssignment::STATUS_CONFIRMED);
        $rsvp = $this->createRsvp($event, $this->patPlayer());
        $this->createAttendanceRecord($event, $rsvp, AttendanceRecord::STATUS_EXCUSED, $coach);

        $this->client->loginUser($trainer->getOwnerAccount());
        $this->client->request('GET', sprintf('/trainer/events/%d', $event->getId()));

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Pat');
        self::assertSelectorTextContains('body', AttendanceRecord::STATUS_EXCUSED);
    }

    /**
     * AC-02-41/BR-02-18: the coach may edit attendance the same day only —
     * denied outright for an event whose start was days ago. The trainer
     * can edit at any time (overrides the coach's own entry), including
     * for that same days-old event.
     */
    public function testCoachEditPermissionsAreSameDayOnlyTrainerAlwaysCan(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $coachAccount = $this->createCoach($trainer, 'attendance-coach-4@practiceperfect.test');
        $coach = $this->coachMembershipFor($trainer, $coachAccount);
        $event = $this->createEvent($trainer, [
            'title' => 'Days Old Session',
            'startsAt' => new \DateTimeImmutable('-3 days'),
            'endsAt' => new \DateTimeImmutable('-3 days +1 hour'),
        ]);
        $this->createCoachAssignment($event, $coach, CoachAssignment::STATUS_CONFIRMED);
        $this->createRsvp($event, $this->patPlayer());

        $this->client->loginUser($coachAccount);
        $this->client->request('GET', sprintf('/coach/events/%d/attendance', $event->getId()));
        self::assertResponseStatusCodeSame(403, 'AC-02-41: locked for the coach past the event\'s own calendar day.');

        $this->client->loginUser($trainer->getOwnerAccount());
        $this->client->request('GET', sprintf('/trainer/events/%d/attendance', $event->getId()));
        self::assertResponseIsSuccessful('AC-02-41: the trainer can edit at any time.');
    }

    /**
     * AC-02-42: attendance cannot be marked for an event that has not
     * started, or one not assigned to the acting coach; a player who
     * canceled their RSVP is not shown in the attendance list.
     */
    public function testAttendanceValidationNotStartedNotAssignedAndCanceledRsvpExcluded(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $coachAccount = $this->createCoach($trainer, 'attendance-coach-5@practiceperfect.test');
        $coach = $this->coachMembershipFor($trainer, $coachAccount);
        $otherCoachAccount = $this->createCoach($trainer, 'attendance-coach-6-unassigned@practiceperfect.test');

        $notStarted = $this->createEvent($trainer, ['title' => 'Not Started Yet Session']);
        $this->createCoachAssignment($notStarted, $coach, CoachAssignment::STATUS_CONFIRMED);

        $started = $this->createEvent($trainer, [
            'title' => 'Started But Not My Assignment',
            'startsAt' => new \DateTimeImmutable('-2 hours'),
            'endsAt' => new \DateTimeImmutable('-1 hour'),
            'capacity' => 5,
        ]);
        $this->createCoachAssignment($started, $coach, CoachAssignment::STATUS_CONFIRMED);
        $pat = $this->patPlayer();
        $this->createRsvp($started, $pat);
        $alex = $this->alexPlayer();
        $this->ensureActivePlayerMembership($trainer, $alex);
        $canceledRsvp = $this->createRsvp($started, $alex, Rsvp::METHOD_FREE, Rsvp::STATUS_CANCELED);

        // AC-02-42: not started yet.
        $this->client->loginUser($coachAccount);
        $this->client->request('GET', sprintf('/coach/events/%d/attendance', $notStarted->getId()));
        self::assertResponseStatusCodeSame(403, 'AC-02-42: cannot mark attendance before the event starts.');

        // AC-02-42: not assigned to this coach.
        $this->client->loginUser($otherCoachAccount);
        $this->client->request('GET', sprintf('/coach/events/%d/attendance', $started->getId()));
        self::assertResponseStatusCodeSame(403, 'AC-02-42: not this coach\'s event.');

        // AC-02-42: a canceled RSVP is excluded from the roster.
        $this->client->loginUser($coachAccount);
        $crawler = $this->client->request('GET', sprintf('/coach/events/%d/attendance', $started->getId()));
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Pat');
        self::assertSelectorTextNotContains('body', $alex->getFirstName());
        self::assertSame(Rsvp::STATUS_CANCELED, $canceledRsvp->getStatus());
    }
}
