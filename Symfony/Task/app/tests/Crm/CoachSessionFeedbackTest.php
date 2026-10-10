<?php

declare(strict_types=1);

namespace App\Tests\Crm;

use App\Crm\Entity\PlayerNote;
use App\Crm\Exception\NoteEditWindowExpiredException;
use App\Crm\Repository\PlayerNoteRepository;
use App\Crm\Service\PlayerNoteService;
use App\Crm\Voter\PlayerVoter;
use App\Scheduling\Entity\AttendanceRecord;
use App\Tests\Support\CrmFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\Security\Core\Authorization\AuthorizationCheckerInterface;

/**
 * US-03.10 — Coach Adds Session Feedback.
 */
final class CoachSessionFeedbackTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;
    use CrmFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-03-46, AC-03-47: the coach selects a recent shared session and
     * writes up to 500 characters; visible to the trainer and to the coach.
     */
    public function testCoachAddsSessionFeedbackVisibleToTrainerAndCoach(): void
    {
        $trainer = $this->trainer('peak-performance');
        $coachAccount = $this->createCoach($trainer, 'feedback.coach.'.uniqid().'@example.test');
        $this->activateTenant($trainer);
        $coach = $this->coachMembershipFor($trainer, $coachAccount);

        $player = $this->freshPlayerMembership($trainer)->getPlayer();
        $event = $this->createEvent($trainer, ['title' => 'Feedback Event '.uniqid()]);
        $this->createCoachAssignment($event, $coach);
        $rsvp = $this->createRsvp($event, $player);
        $this->createAttendanceRecord($event, $rsvp, AttendanceRecord::STATUS_PRESENT, $coach);
        $membership = $this->membershipFor($trainer, $player);

        $this->client->loginUser($coachAccount);
        $crawler = $this->client->request('GET', '/coach/players/'.$membership->getId());
        $form = $crawler->selectButton('Add feedback')->form([
            'session_feedback[event]' => (string) $event->getId(),
            'session_feedback[text]' => 'Great improvement on footwork.',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', 'Great improvement on footwork.');

        // AC-03-48: visible to the trainer, linked to the session, with the
        // coach's name shown.
        $this->client->loginUser($trainer->getOwnerAccount());
        $this->client->request('GET', '/trainer/players/'.$membership->getId());
        self::assertSelectorTextContains('body', 'Great improvement on footwork.');
        self::assertSelectorTextContains('body', '['.$event->getTitle().']');
    }

    /**
     * AC-03-49: the coach can edit their own feedback within 24 hours.
     */
    public function testCoachEditsTheirOwnFeedbackWithinTheWindow(): void
    {
        $trainer = $this->trainer('peak-performance');
        $coachAccount = $this->createCoach($trainer, 'edit.coach.'.uniqid().'@example.test');
        $this->activateTenant($trainer);
        $coach = $this->coachMembershipFor($trainer, $coachAccount);
        $player = $this->freshPlayerMembership($trainer)->getPlayer();
        $event = $this->createEvent($trainer, ['title' => 'Edit Feedback Event '.uniqid()]);
        $feedback = $this->createPlayerNote($trainer, $player, PlayerNote::TYPE_SESSION, 'Original feedback.', $coachAccount, $event);

        /** @var PlayerNoteService $noteService */
        $noteService = self::getContainer()->get(PlayerNoteService::class);
        $noteService->edit($feedback, $coachAccount, 'Updated feedback.', new \DateTimeImmutable());

        self::assertSame('Updated feedback.', $feedback->getNoteText());

        $this->expectException(NoteEditWindowExpiredException::class);
        $noteService->edit($feedback, $coachAccount, 'Too late.', new \DateTimeImmutable('+25 hours'));
        unset($coach);
    }

    /**
     * AC-03-49: only the coach's OWN feedback is editable — another coach's
     * feedback is denied outright, at the voter.
     */
    public function testACoachCannotEditAnotherCoachsFeedback(): void
    {
        $trainer = $this->trainer('peak-performance');
        $authorCoach = $this->createCoach($trainer, 'author.coach.'.uniqid().'@example.test');
        $otherCoach = $this->createCoach($trainer, 'stranger.coach.'.uniqid().'@example.test');
        $this->activateTenant($trainer);
        $player = $this->freshPlayerMembership($trainer)->getPlayer();
        $event = $this->createEvent($trainer, ['title' => 'Ownership Event '.uniqid()]);
        $feedback = $this->createPlayerNote($trainer, $player, PlayerNote::TYPE_SESSION, 'Authored feedback.', $authorCoach, $event);

        $this->client->loginUser($otherCoach);
        /** @var AuthorizationCheckerInterface $authChecker */
        $authChecker = self::getContainer()->get(AuthorizationCheckerInterface::class);

        self::assertFalse(
            $authChecker->isGranted(PlayerVoter::PLAYER_FEEDBACK_EDIT, $feedback),
            'AC-03-49: own feedback only.',
        );
    }

    /**
     * AC-03-49: the trainer cannot edit coach feedback — read-only to them,
     * same as any other coach-authored note.
     */
    public function testTrainerCannotEditCoachFeedback(): void
    {
        $trainer = $this->trainer('peak-performance');
        $coachAccount = $this->createCoach($trainer, 'readonlytest.coach.'.uniqid().'@example.test');
        $this->activateTenant($trainer);
        $player = $this->freshPlayerMembership($trainer)->getPlayer();
        $event = $this->createEvent($trainer, ['title' => 'Trainer Readonly Event '.uniqid()]);
        $feedback = $this->createPlayerNote($trainer, $player, PlayerNote::TYPE_SESSION, 'Coach-only editable.', $coachAccount, $event);

        $this->client->loginUser($trainer->getOwnerAccount());
        /** @var AuthorizationCheckerInterface $authChecker */
        $authChecker = self::getContainer()->get(AuthorizationCheckerInterface::class);

        self::assertFalse(
            $authChecker->isGranted(PlayerVoter::PLAYER_NOTE_MANAGE, $feedback),
            'AC-03-49/BR-03-12: coach feedback is read-only to the trainer.',
        );
    }

    /**
     * AC-03-49: Super Admin can edit or delete any feedback at any time.
     */
    public function testSuperAdminCanEditAnyFeedbackAtAnyTime(): void
    {
        $trainer = $this->trainer('peak-performance');
        $coachAccount = $this->createCoach($trainer, 'sa.coach.'.uniqid().'@example.test');
        $this->activateTenant($trainer);
        $player = $this->freshPlayerMembership($trainer)->getPlayer();
        $event = $this->createEvent($trainer, ['title' => 'SA Feedback Event '.uniqid()]);
        $feedback = $this->createPlayerNote($trainer, $player, PlayerNote::TYPE_SESSION, 'Old feedback.', $coachAccount, $event);
        $reflection = new \ReflectionProperty($feedback, 'createdAt');
        $reflection->setAccessible(true);
        $reflection->setValue($feedback, new \DateTimeImmutable('-10 days'));
        $this->flush();

        $superAdmin = $this->account('admin@practiceperfect.test');
        $this->client->loginUser($superAdmin);
        /** @var \App\Platform\Tenancy\AdministrativeScope $scope */
        $scope = self::getContainer()->get(\App\Platform\Tenancy\AdministrativeScope::class);
        $scope->openFor($trainer, $superAdmin);

        /** @var AuthorizationCheckerInterface $authChecker */
        $authChecker = self::getContainer()->get(AuthorizationCheckerInterface::class);
        self::assertTrue(
            $authChecker->isGranted(PlayerVoter::PLAYER_FEEDBACK_EDIT, $feedback),
            'AC-03-49: Super Admin can edit any feedback at any time.',
        );
    }

    /**
     * AC-03-49 (delete half, completing api-designer-spec's own gap — see
     * the coder's final report): the coach can delete their own feedback
     * within 24 hours.
     */
    public function testCoachDeletesTheirOwnFeedbackWithinTheWindow(): void
    {
        $trainer = $this->trainer('peak-performance');
        $coachAccount = $this->createCoach($trainer, 'delete.coach.'.uniqid().'@example.test');
        $this->activateTenant($trainer);
        $coach = $this->coachMembershipFor($trainer, $coachAccount);
        $player = $this->freshPlayerMembership($trainer)->getPlayer();
        $event = $this->createEvent($trainer, ['title' => 'Delete Feedback Event '.uniqid()]);
        $this->createCoachAssignment($event, $coach);
        $this->createRsvp($event, $player);
        $feedback = $this->createPlayerNote($trainer, $player, PlayerNote::TYPE_SESSION, 'Delete this feedback.', $coachAccount, $event);
        $membership = $this->membershipFor($trainer, $player);

        $this->client->loginUser($coachAccount);
        // A real request establishes the session the CSRF token is bound to
        // — generating one directly from the container with no prior
        // request has no session to attach to.
        $crawler = $this->client->request('GET', '/coach/players/'.$membership->getId());
        $token = $crawler->filter('form[action*="/feedback/'.$feedback->getId().'/delete"] input[name="_token"]')->attr('value');

        $this->client->request(
            'POST',
            '/coach/players/'.$membership->getId().'/feedback/'.$feedback->getId().'/delete',
            ['_token' => $token],
        );

        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var PlayerNoteRepository $notes */
        $notes = self::getContainer()->get(PlayerNoteRepository::class);
        self::assertEmpty($notes->findSessionNotesForPlayerByAuthor($player, $coachAccount));
    }
}
