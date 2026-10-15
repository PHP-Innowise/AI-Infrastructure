<?php

declare(strict_types=1);

namespace App\Tests\Crm;

use App\Crm\Entity\PlayerFlag;
use App\Crm\Entity\PlayerNote;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Scheduling\Entity\AttendanceRecord;
use App\Tests\Support\CrmFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-03.07 — Trainer Views Player Detail.
 */
final class PlayerDetailTest extends WebTestCase
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
     * AC-03-27: Basic Profile — name, age, gender, skill level, school/team,
     * registration date.
     */
    public function testPlayerDetailShowsTheBasicProfile(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer);
        $membership->setSkillLevel('Intermediate');
        $this->flush();

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', '/trainer/players/'.$membership->getId());

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', $membership->getPlayer()->getFirstName());
        self::assertSelectorTextContains('body', 'Intermediate');
        self::assertSelectorTextContains('body', '15'); // age, from the -15 years DOB fixture helper uses
    }

    /**
     * AC-03-33: the trainer can edit the player profile's limited fields
     * (skill level).
     */
    public function testTrainerEditsThePlayersSkillLevel(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/players/'.$membership->getId());
        $form = $crawler->selectButton('Save changes')->form([
            'player_crm_fields[skillLevel]' => 'Elite',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var PlayerTrainerMembershipRepository $memberships */
        $memberships = self::getContainer()->get(PlayerTrainerMembershipRepository::class);
        $reloaded = $memberships->find($membership->getId());
        self::assertNotNull($reloaded);
        self::assertSame('Elite', $reloaded->getSkillLevel());
    }

    /**
     * AC-03-31: Event History — date, event, attendance status, per-event
     * notes, attendance rate, no-show count, last event date.
     */
    public function testPlayerDetailShowsEventHistoryWithAttendanceSummary(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer);
        $coach = $this->coachMembershipFor($trainer, $this->account('coach@practiceperfect.test'));

        $presentEvent = $this->createEvent($trainer, ['title' => 'History Present '.uniqid()]);
        $presentRsvp = $this->createRsvp($presentEvent, $membership->getPlayer());
        $this->createAttendanceRecord($presentEvent, $presentRsvp, AttendanceRecord::STATUS_PRESENT, $coach);

        $absentEvent = $this->createEvent($trainer, ['title' => 'History Absent '.uniqid()]);
        $absentRsvp = $this->createRsvp($absentEvent, $membership->getPlayer());
        $this->createAttendanceRecord($absentEvent, $absentRsvp, AttendanceRecord::STATUS_ABSENT, $coach);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', '/trainer/players/'.$membership->getId());

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', $presentEvent->getTitle());
        self::assertSelectorTextContains('body', $absentEvent->getTitle());
        self::assertSelectorTextContains('body', '1 of 2 events');
        self::assertSelectorTextContains('body', '1 no-shows');
    }

    /**
     * AC-03-32 (optional MVP): a read-only Coach Feedback section.
     */
    public function testPlayerDetailShowsCoachFeedbackReadOnly(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer);
        $coachAccount = $this->account('coach@practiceperfect.test');
        $event = $this->createEvent($trainer, ['title' => 'Feedback Session '.uniqid()]);
        $this->createPlayerNote($trainer, $membership->getPlayer(), PlayerNote::TYPE_SESSION, 'Great footwork today.', $coachAccount, $event);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/players/'.$membership->getId());

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Great footwork today.');
        // Read-only: no edit/delete control for the trainer over coach feedback.
        self::assertCount(0, $crawler->selectButton('Delete')->filter('form[action*="feedback"]'));
    }

    /**
     * AC-03-29: the Flags section shows who applied and when.
     */
    public function testPlayerDetailFlagsSectionShowsWhoAndWhen(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer);
        $this->applyFlagToPlayer($trainer, $membership->getPlayer(), PlayerFlag::TYPE_MEDICAL_RESTRICTION, $trainer->getOwnerAccount(), 'Cleared for light drills only.');

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', '/trainer/players/'.$membership->getId());

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Medical restriction');
        self::assertSelectorTextContains('body', 'Tina Trainer');
        self::assertSelectorTextContains('body', 'Cleared for light drills only.');
    }
}
