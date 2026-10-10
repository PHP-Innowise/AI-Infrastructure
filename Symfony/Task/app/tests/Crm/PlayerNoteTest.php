<?php

declare(strict_types=1);

namespace App\Tests\Crm;

use App\Crm\Entity\PlayerNote;
use App\Crm\Exception\NoteEditWindowExpiredException;
use App\Crm\Repository\PlayerNoteRepository;
use App\Crm\Service\PlayerNoteService;
use App\Crm\Voter\PlayerVoter;
use App\Tests\Support\CrmFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\Security\Core\Authorization\AuthorizationCheckerInterface;

/**
 * US-03.05 — Trainer Adds Notes to Player.
 */
final class PlayerNoteTest extends WebTestCase
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
     * AC-03-19: a general note (up to 1000 chars) appears chronologically
     * with author and timestamp. AC-03-30: the player detail view's Notes
     * section shows exactly this — a chronological list with an "+ Add
     * Note" button — the same content, viewed from US-03.07's own AC.
     */
    public function testTrainerAddsAGeneralNote(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/players/'.$membership->getId());

        $form = $crawler->selectButton('Add note')->form([
            'add_note[text]' => 'Great attitude in practice this week.',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', 'Great attitude in practice this week.');
        self::assertSelectorTextContains('body', 'Tina Trainer');

        $this->activateTenant($trainer);
        /** @var PlayerNoteRepository $notes */
        $notes = self::getContainer()->get(PlayerNoteRepository::class);
        $general = $notes->findGeneralForPlayer($membership->getPlayer());
        self::assertCount(1, $general);
        self::assertSame(PlayerNote::TYPE_GENERAL, $general[0]->getNoteType());
    }

    /**
     * AC-03-20: a note tied to a specific event, displayed as "[Event
     * Title] - [Date]: [Note]".
     */
    public function testTrainerAddsAPerEventNote(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer);
        $event = $this->createEvent($trainer, ['title' => 'Footwork Clinic '.uniqid()]);
        $rsvp = $this->createRsvp($event, $membership->getPlayer());
        $coach = $this->coachMembershipFor($trainer, $this->account('coach@practiceperfect.test'));
        $this->createAttendanceRecord($event, $rsvp, \App\Scheduling\Entity\AttendanceRecord::STATUS_PRESENT, $coach);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/players/'.$membership->getId());

        $form = $crawler->selectButton('Add note')->form([
            'add_note[text]' => 'Worked on crossover footwork.',
            'add_note[event]' => (string) $event->getId(),
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', '['.$event->getTitle().']');
        self::assertSelectorTextContains('body', 'Worked on crossover footwork.');

        $this->activateTenant($trainer);
        /** @var PlayerNoteRepository $notes */
        $notes = self::getContainer()->get(PlayerNoteRepository::class);
        $sessionNotes = $notes->findForEventAndPlayer($event, $membership->getPlayer());
        self::assertCount(1, $sessionNotes);
        self::assertSame(PlayerNote::TYPE_SESSION, $sessionNotes[0]->getNoteType());
    }

    /**
     * AC-03-21/BR-03-12: the creator can edit their own note within 24
     * hours; after that it becomes read-only.
     */
    public function testTrainerCanEditTheirOwnNoteWithinTheWindowButNotAfter(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer);
        $note = $this->createPlayerNote($trainer, $membership->getPlayer(), PlayerNote::TYPE_GENERAL, 'Original text.', $trainer->getOwnerAccount());

        /** @var PlayerNoteService $noteService */
        $noteService = self::getContainer()->get(PlayerNoteService::class);
        $noteService->edit($note, $trainer->getOwnerAccount(), 'Edited within the window.', new \DateTimeImmutable());
        self::assertSame('Edited within the window.', $note->getNoteText(), 'AC-03-21: editable within 24 hours.');

        $this->expectException(NoteEditWindowExpiredException::class);
        $noteService->edit($note, $trainer->getOwnerAccount(), 'Too late.', new \DateTimeImmutable('+25 hours'));
    }

    /**
     * AC-03-21: the trainer can delete their own note at any time — no
     * 24-hour limit on delete (unlike edit).
     */
    public function testTrainerCanDeleteTheirOwnNoteAtAnyTime(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer);
        $note = $this->createPlayerNote($trainer, $membership->getPlayer(), PlayerNote::TYPE_GENERAL, 'Delete me.', $trainer->getOwnerAccount());
        // Backdate creation well past the 24h edit window to prove delete is unaffected.
        $reflection = new \ReflectionProperty($note, 'createdAt');
        $reflection->setAccessible(true);
        $reflection->setValue($note, new \DateTimeImmutable('-10 days'));
        $this->crmFlush();

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/players/'.$membership->getId());
        $form = $crawler->selectButton('Delete')->form();
        $this->client->submit($form);

        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var PlayerNoteRepository $notes */
        $notes = self::getContainer()->get(PlayerNoteRepository::class);
        self::assertEmpty($notes->findGeneralForPlayer($membership->getPlayer()), 'AC-03-21: deletable at any time.');
    }

    /**
     * AC-03-21/BR-03-12: the trainer cannot edit or delete a coach's note —
     * read-only to them.
     */
    public function testTrainerCannotEditOrDeleteACoachAuthoredNote(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer);
        $coachAccount = $this->account('coach@practiceperfect.test');
        $event = $this->createEvent($trainer, ['title' => 'Coach Note Session '.uniqid()]);
        $note = $this->createPlayerNote($trainer, $membership->getPlayer(), PlayerNote::TYPE_SESSION, 'Coach feedback text.', $coachAccount, $event);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        /** @var AuthorizationCheckerInterface $authChecker */
        $authChecker = self::getContainer()->get(AuthorizationCheckerInterface::class);

        self::assertFalse(
            $authChecker->isGranted(PlayerVoter::PLAYER_NOTE_MANAGE, $note),
            'AC-03-21/BR-03-12: a coach-authored note is read-only to the trainer.',
        );
    }

    /**
     * BR-03-12: Super Admin can edit any note at any time — no 24-hour
     * limit, regardless of author.
     */
    public function testSuperAdminCanEditAnyNoteAtAnyTime(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer);
        $note = $this->createPlayerNote($trainer, $membership->getPlayer(), PlayerNote::TYPE_GENERAL, 'Old note.', $trainer->getOwnerAccount());
        $reflection = new \ReflectionProperty($note, 'createdAt');
        $reflection->setAccessible(true);
        $reflection->setValue($note, new \DateTimeImmutable('-30 days'));
        $this->crmFlush();

        $superAdmin = $this->account('admin@practiceperfect.test');
        $this->client->loginUser($superAdmin);
        /** @var \App\Platform\Tenancy\AdministrativeScope $scope */
        $scope = self::getContainer()->get(\App\Platform\Tenancy\AdministrativeScope::class);
        $scope->openFor($trainer, $superAdmin);

        /** @var AuthorizationCheckerInterface $authChecker */
        $authChecker = self::getContainer()->get(AuthorizationCheckerInterface::class);
        self::assertTrue(
            $authChecker->isGranted(PlayerVoter::PLAYER_NOTE_MANAGE, $note),
            'BR-03-12: Super Admin can edit any note at any time, regardless of its age.',
        );
    }

    private function crmFlush(): void
    {
        /** @var \Doctrine\ORM\EntityManagerInterface $em */
        $em = self::getContainer()->get(\Doctrine\ORM\EntityManagerInterface::class);
        $em->flush();
    }
}
