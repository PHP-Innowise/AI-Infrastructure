<?php

declare(strict_types=1);

namespace App\Tests\Administration;

use App\Identity\Entity\CoachMembership;
use App\Identity\Service\AvailabilityService;
use App\Platform\Entity\Trainer;
use App\Platform\Repository\AuditLogEntryRepository;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-07.07 — Super Admin Overrides Scheduling Conflict.
 *
 * Uses a freshly created coach (`createCoach()`), never the shared fixture
 * coach ("Casey Coach") — that helper's own docblock explains why:
 * assignment/attendance history other tests set on the fixture coach would
 * make "is this conflict genuinely caused by the assignment below" an
 * unreliable question.
 */
final class EventMasterConflictOverrideTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-07-27: a coach already assigned to another event at the same
     * time — no warning is shown and the event saves without requiring
     * confirmation. AC-07-28: the override is recorded in the audit log.
     */
    public function testAssigningADoubleBookedCoachSavesWithNoWarningAndIsAudited(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $now = new \DateTimeImmutable();
        $coachAccount = $this->createCoach($trainer, 'conflict-coach@example.test');
        $coach = $this->coachMembershipFor($trainer, $coachAccount);

        $busyEvent = $this->createEvent($trainer, [
            'title' => 'Already Booked Session',
            'startsAt' => $now->modify('+10 days')->setTime(9, 0),
            'endsAt' => $now->modify('+10 days')->setTime(10, 0),
        ]);
        $this->createCoachAssignment($busyEvent, $coach);

        $conflictingEvent = $this->createEvent($trainer, [
            'title' => 'Overlapping New Session',
            'startsAt' => $now->modify('+10 days')->setTime(9, 30),
            'endsAt' => $now->modify('+10 days')->setTime(10, 30),
        ]);

        $admin = $this->account('admin@practiceperfect.test');
        $this->client->loginUser($admin);

        $editCrawler = $this->client->request('GET', sprintf('/super-admin/events/%d/edit', $conflictingEvent->getId()));
        self::assertResponseIsSuccessful();

        $form = $editCrawler->selectButton('Save (Super Admin)')->form();
        $form['event[coach]'] = (string) $coach->getId();
        $this->client->submit($form);

        // AC-07-27: no warning, no form-error round-trip — a clean redirect
        // straight through, exactly like any conflict-free save.
        self::assertResponseRedirects(
            sprintf('/super-admin/events/%d', $conflictingEvent->getId()),
            message: 'AC-07-27: the double-booked assignment saves without requiring confirmation.',
        );
        $this->client->followRedirect();
        self::assertSelectorTextContains('.flash--success', 'Event updated (Super Admin)');

        $showText = (string) $this->client->getResponse()->getContent();
        self::assertStringNotContainsString('Continue anyway?', $showText, 'AC-07-27: the conflict warning text never appears.');

        /** @var AuditLogEntryRepository $auditLog */
        $auditLog = self::getContainer()->get(AuditLogEntryRepository::class);
        $entries = $auditLog->search('event.coach_conflict_override', (int) $trainer->getId());
        $match = current(array_filter(
            $entries,
            static fn ($e) => (int) ($e->getSubjectId() ?? 0) === (int) $conflictingEvent->getId(),
        ));

        self::assertNotFalse($match, 'AC-07-28: the overridden conflict is recorded in the audit log.');
        self::assertSame($admin->getId(), $match->getActorAccount()?->getId(), 'AC-07-28: attributed to the Super Admin.');
        self::assertStringContainsString('Super Admin', (string) ($match->getDetails()['reason'] ?? ''));
    }

    /**
     * The converse: assigning a coach with NO conflict never manufactures
     * an override audit entry — proving the override log is genuinely
     * conditional on a real conflict, not written unconditionally whenever
     * Super Admin assigns a coach via Event Master. An empty availability
     * set always conflicts on its own (CoachAvailabilityConflictChecker's
     * own "there is nothing on record that covers the proposed slot"
     * rule — see CoachAssignmentTest's own precedent for this exact
     * caveat), so this test's coach needs a declared window wide enough to
     * genuinely cover the event, not merely a freshly created one.
     */
    public function testAssigningAFreeCoachWithNoConflictWritesNoOverrideEntry(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $now = new \DateTimeImmutable();
        $coachAccount = $this->createCoach($trainer, 'free-coach@example.test');
        $coach = $this->coachMembershipFor($trainer, $coachAccount);
        $this->declareCoachAvailableAllWeekDaytime($trainer, $coach);

        $event = $this->createEvent($trainer, [
            'title' => 'No Conflict Session',
            'startsAt' => $now->modify('+11 days')->setTime(14, 0),
            'endsAt' => $now->modify('+11 days')->setTime(15, 0),
        ]);

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $editCrawler = $this->client->request('GET', sprintf('/super-admin/events/%d/edit', $event->getId()));
        $form = $editCrawler->selectButton('Save (Super Admin)')->form();
        $form['event[coach]'] = (string) $coach->getId();
        $this->client->submit($form);

        self::assertResponseRedirects();

        /** @var AuditLogEntryRepository $auditLog */
        $auditLog = self::getContainer()->get(AuditLogEntryRepository::class);
        $entries = $auditLog->search('event.coach_conflict_override', (int) $trainer->getId());
        $match = current(array_filter(
            $entries,
            static fn ($e) => (int) ($e->getSubjectId() ?? 0) === (int) $event->getId(),
        ));

        self::assertFalse($match, 'No conflict existed, so no override entry is written for this event.');
    }

    /**
     * Mirrors `CoachAssignmentTest`'s own private helper of the same name
     * exactly — not shared via `SchedulingFixtureHelpers`, so duplicated
     * here rather than widening that trait for a single caller.
     */
    private function declareCoachAvailableAllWeekDaytime(Trainer $trainer, CoachMembership $coach): void
    {
        $this->activateTenant($trainer);
        /** @var AvailabilityService $availability */
        $availability = self::getContainer()->get(AvailabilityService::class);

        $slots = [];
        foreach (range(0, 6) as $day) {
            $slots[] = [
                'dayOfWeek' => $day,
                'startTime' => new \DateTimeImmutable('1970-01-01 06:00:00'),
                'endTime' => new \DateTimeImmutable('1970-01-01 22:00:00'),
                'isAvailable' => true,
            ];
        }

        $availability->setCoachAvailability($trainer, $coach, $slots);
    }
}
