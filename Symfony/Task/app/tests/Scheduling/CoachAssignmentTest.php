<?php

declare(strict_types=1);

namespace App\Tests\Scheduling;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountProfile;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\CoachMembership;
use App\Identity\Service\AvailabilityService;
use App\Platform\Entity\Trainer;
use App\Scheduling\Entity\CoachAssignment;
use App\Scheduling\Repository\CoachAssignmentRepository;
use App\Scheduling\Repository\CoachAvailabilityOverrideRepository;
use App\Scheduling\Repository\EventRepository;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\PasswordHasher\Hasher\UserPasswordHasherInterface;

/**
 * US-02.03 — Trainer Assigns Coach with Availability Check.
 * US-02.10 — Coach Confirms Event Assignment.
 */
final class CoachAssignmentTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-02-8: the coach dropdown includes the trainer themselves and every
     * coach the trainer has added.
     */
    public function testCoachDropdownIncludesTheTrainerAndAddedCoaches(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/trainer/events/new');
        self::assertResponseIsSuccessful();

        $options = $crawler->filter('select[name="event[coach]"] option')->each(static fn ($node) => $node->text());

        self::assertContains('Tina Trainer', $options, 'AC-02-8: the trainer can assign themselves.');
        self::assertContains('Casey Coach', $options, 'AC-02-8: an added coach appears.');
    }

    /**
     * AC-02-10: no conflict — the coach is assigned directly, no override
     * required.
     */
    public function testCoachIsAssignedDirectlyWhenNoConflict(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        // A dedicated coach, not the shared "Casey Coach" fixture:
        // declaring broad availability here must not leak into (and
        // silently satisfy) a different test's availability-conflict
        // assertion against the shared fixture coach — this positive
        // ("assigned directly") case and the negative (conflict) case each
        // need their own, non-interfering coach.
        $coachAccount = $this->createSecondCoach($trainer, 'no-conflict-coach@practiceperfect.test');
        $coach = $this->coachMembershipFor($trainer, $coachAccount);

        // An empty availability set always conflicts
        // (CoachAvailabilityConflictChecker's own docblock: "there is
        // nothing on record that covers the proposed slot") — this test is
        // specifically the NO-conflict path, so the coach needs a declared
        // window that actually covers the event.
        $event = $this->createEvent($trainer, [
            'title' => 'No Conflict Session',
            'startsAt' => new \DateTimeImmutable('next Monday 10:00'),
            'endsAt' => new \DateTimeImmutable('next Monday 11:00'),
        ]);
        $this->declareCoachAvailableAllWeekDaytime($trainer, $coach);

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', sprintf('/trainer/events/%d/edit', $event->getId()));
        $form = $crawler->selectButton('Save changes')->form([
            'event[coach]' => (string) $coach->getId(),
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var CoachAssignmentRepository $assignments */
        $assignments = self::getContainer()->get(CoachAssignmentRepository::class);
        $assignment = $assignments->findOneByEventAndCoach($event, $coach);
        self::assertNotNull($assignment);
        self::assertTrue($assignment->isPending(), 'AC-02-65/BR-02-13: Pending until the coach confirms.');
    }

    /**
     * AC-02-9/BR-02-13/15: an availability or double-booking conflict shows
     * a warning and requires an override reason; the override is logged
     * (who, when, event, coach, reason).
     */
    public function testAssigningAConflictingCoachRequiresAnOverrideReasonAndLogsIt(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $coach = $this->coachMembershipFor($trainer, $this->account('coach@practiceperfect.test'));

        // An existing confirmed assignment for the coach...
        $existing = $this->createEvent($trainer, [
            'title' => 'Existing Assignment',
            'startsAt' => new \DateTimeImmutable('+4 days 15:00'),
            'endsAt' => new \DateTimeImmutable('+4 days 16:00'),
        ]);
        $this->createCoachAssignment($existing, $coach, CoachAssignment::STATUS_CONFIRMED);

        // ...and a NEW event that overlaps it (BR-02-15 double-booking).
        $conflicting = $this->createEvent($trainer, [
            'title' => 'Overlapping Session',
            'startsAt' => new \DateTimeImmutable('+4 days 15:30'),
            'endsAt' => new \DateTimeImmutable('+4 days 16:30'),
        ]);

        $this->client->loginUser($trainer->getOwnerAccount());

        // Without a reason: rejected with a warning.
        $crawler = $this->client->request('GET', sprintf('/trainer/events/%d/edit', $conflicting->getId()));
        $form = $crawler->selectButton('Save changes')->form([
            'event[coach]' => (string) $coach->getId(),
        ]);
        $this->client->submit($form);

        self::assertResponseIsUnprocessable();
        self::assertSelectorTextContains('body', 'Continue anyway?');

        // With a reason: accepted, and the override is logged.
        $crawler = $this->client->request('GET', sprintf('/trainer/events/%d/edit', $conflicting->getId()));
        $form = $crawler->selectButton('Save changes')->form([
            'event[coach]' => (string) $coach->getId(),
            'event[coachOverrideReason]' => 'Coach agreed to cover both sessions back-to-back.',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var CoachAvailabilityOverrideRepository $overrides */
        $overrides = self::getContainer()->get(CoachAvailabilityOverrideRepository::class);
        $found = $overrides->findForEvent($conflicting);
        self::assertNotEmpty($found, 'AC-02-9: the override is logged.');
        self::assertSame('Coach agreed to cover both sessions back-to-back.', $found[0]->getReason());
        self::assertSame($trainer->getOwnerAccount()->getId(), $found[0]->getOverriddenByAccount()->getId(), 'AC-02-9: who overrode is logged.');
    }

    /**
     * AC-02-11: the assigned coach sees the event in "Events to Confirm".
     */
    public function testAssignedCoachSeesTheEventInEventsToConfirm(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $coach = $this->coachMembershipFor($trainer, $this->account('coach@practiceperfect.test'));
        $event = $this->createEvent($trainer, ['title' => 'Fresh Assignment Unique 1']);
        $this->createCoachAssignment($event, $coach, CoachAssignment::STATUS_PENDING);

        $this->client->loginUser($this->account('coach@practiceperfect.test'));
        $this->client->request('GET', '/coach/activities');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Fresh Assignment Unique 1');
    }

    /**
     * AC-02-34: pending assignments show title, date/time, location, and
     * number of players RSVP'd.
     */
    public function testActivitiesPageShowsAssignmentDetails(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $coach = $this->coachMembershipFor($trainer, $this->account('coach@practiceperfect.test'));
        $event = $this->createEvent($trainer, ['title' => 'Detail Check Session Unique 2', 'location' => 'Court 9']);
        $this->createCoachAssignment($event, $coach, CoachAssignment::STATUS_PENDING);

        $this->client->loginUser($this->account('coach@practiceperfect.test'));
        $this->client->request('GET', '/coach/activities');

        self::assertSelectorTextContains('body', 'Detail Check Session Unique 2');
        self::assertSelectorTextContains('body', 'Court 9');
    }

    /**
     * AC-02-35: confirming moves the session to Assigned Sessions, status
     * Confirmed. Scoped to the specific row by its unique title — other
     * tests in this same run may leave their own pending assignments
     * visible on this shared-database list.
     */
    public function testCoachConfirmsAnAssignment(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $coach = $this->coachMembershipFor($trainer, $this->account('coach@practiceperfect.test'));
        $event = $this->createEvent($trainer, ['title' => 'To Be Confirmed Unique 3']);
        $assignment = $this->createCoachAssignment($event, $coach, CoachAssignment::STATUS_PENDING);

        $this->client->loginUser($this->account('coach@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/coach/activities');
        $row = $crawler->filter('tr:contains("To Be Confirmed Unique 3")');
        self::assertGreaterThan(0, $row->count());
        $form = $row->selectButton('Confirm')->form();
        $this->client->submit($form);

        self::assertResponseRedirects('/coach/activities');

        $this->activateTenant($trainer);
        /** @var CoachAssignmentRepository $assignments */
        $assignments = self::getContainer()->get(CoachAssignmentRepository::class);
        $reloaded = $assignments->find($assignment->getId());
        self::assertNotNull($reloaded);
        self::assertTrue($reloaded->isConfirmed(), 'AC-02-35: status becomes Confirmed.');
    }

    /**
     * AC-02-36: declining removes it from the coach's view (Events to
     * Confirm) with an optional reason; the event still exists.
     */
    public function testCoachDeclinesAnAssignmentWithAReason(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $coach = $this->coachMembershipFor($trainer, $this->account('coach@practiceperfect.test'));
        $event = $this->createEvent($trainer, ['title' => 'To Be Declined Unique 4']);
        $assignment = $this->createCoachAssignment($event, $coach, CoachAssignment::STATUS_PENDING);

        $this->client->loginUser($this->account('coach@practiceperfect.test'));
        $crawler = $this->client->request('GET', sprintf('/coach/assignments/%d/decline', $assignment->getId()));
        $form = $crawler->selectButton('Decline')->form(['decline_assignment[reason]' => 'Schedule conflict.']);
        $this->client->submit($form);

        self::assertResponseRedirects('/coach/activities');
        $this->client->followRedirect();
        self::assertSelectorTextNotContains('body', 'To Be Declined Unique 4');

        $this->activateTenant($trainer);
        /** @var CoachAssignmentRepository $assignments */
        $assignments = self::getContainer()->get(CoachAssignmentRepository::class);
        $reloaded = $assignments->find($assignment->getId());
        self::assertNotNull($reloaded);
        self::assertTrue($reloaded->isDeclined());
        self::assertSame('Schedule conflict.', $reloaded->getDeclineReason());

        /** @var EventRepository $events */
        $events = self::getContainer()->get(EventRepository::class);
        self::assertNotNull($events->find($event->getId()), 'AC-02-36: the event still exists and needs a new coach.');
    }

    /**
     * CoachAssignmentVoter::COACH_ASSIGNMENT_CONFIRM: a coach who is not the
     * one named on the assignment cannot confirm it — a direct ownership
     * check, not merely the coarse /coach/ role gate (a second real coach
     * account is created here specifically so this exercises the voter's
     * own ownership logic, not just ROLE_COACH). The controller checks the
     * voter BEFORE the CSRF token, so this 403s on ownership regardless of
     * what (if any) token is submitted — no token needs to be crafted here.
     */
    public function testACoachCannotConfirmAnotherCoachsAssignment(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $coach = $this->coachMembershipFor($trainer, $this->account('coach@practiceperfect.test'));
        $event = $this->createEvent($trainer, ['title' => 'Someone Elses Assignment']);
        $assignment = $this->createCoachAssignment($event, $coach, CoachAssignment::STATUS_PENDING);

        $otherCoachAccount = $this->createSecondCoach($trainer);

        $this->client->loginUser($otherCoachAccount);
        $this->client->request('POST', sprintf('/coach/assignments/%d/confirm', $assignment->getId()));

        self::assertResponseStatusCodeSame(403);
    }

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

    private function createSecondCoach(Trainer $trainer, string $email = 'second-coach@practiceperfect.test'): Account
    {
        /** @var UserPasswordHasherInterface $hasher */
        $hasher = self::getContainer()->get(UserPasswordHasherInterface::class);
        $account = new Account($email, '', AccountRole::Coach);
        $account->changePasswordHash($hasher->hashPassword($account, 'password'));
        $account->verifyEmail();

        $em = $this->entityManagerFor();
        $em->persist($account);
        $em->persist(new AccountProfile($account, 'Other', 'Coach'));
        $em->flush();

        $this->activateTenant($trainer);
        $em->persist(new CoachMembership($trainer, $account, CoachMembership::STATUS_ACTIVE));
        $em->flush();

        return $account;
    }

    private function entityManagerFor(): EntityManagerInterface
    {
        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);

        return $em;
    }
}
