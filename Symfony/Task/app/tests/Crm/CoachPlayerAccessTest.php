<?php

declare(strict_types=1);

namespace App\Tests\Crm;

use App\Crm\Voter\PlayerVoter;
use App\Scheduling\Entity\AttendanceRecord;
use App\Tests\Support\CrmFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\Security\Core\Authorization\AuthorizationCheckerInterface;

/**
 * US-03.09 — Coach Views Assigned Players, and the "In Scope (MVP)"
 * event-scoped access rules (AC-03-64/65).
 */
final class CoachPlayerAccessTest extends WebTestCase
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
     * AC-03-64: a coach assigned to zero events sees zero players — proven
     * with a BRAND NEW coach who has no assignments at all.
     */
    public function testACoachAssignedToZeroEventsSeesZeroPlayers(): void
    {
        $trainer = $this->trainer('peak-performance');
        $unassignedCoach = $this->createCoach($trainer, 'unassigned.coach.'.uniqid().'@example.test');

        $this->client->loginUser($unassignedCoach);
        $this->client->request('GET', '/coach/players');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'not assigned to any events');
    }

    /**
     * AC-03-43: a coach sees only players RSVP'd to their own assigned
     * events, with an attendance summary — never the trainer's full roster.
     */
    public function testCoachSeesOnlyPlayersFromTheirOwnAssignedEvents(): void
    {
        $trainer = $this->trainer('peak-performance');
        $coachAccount = $this->createCoach($trainer, 'scoped.coach.'.uniqid().'@example.test');
        $this->activateTenant($trainer);
        $coach = $this->coachMembershipFor($trainer, $coachAccount);

        $reachablePlayer = $this->freshPlayerMembership($trainer, 'ReachablePlayer'.uniqid())->getPlayer();
        $event = $this->createEvent($trainer, ['title' => 'Coach Scoped Event '.uniqid()]);
        $this->createCoachAssignment($event, $coach);
        $this->createRsvp($event, $reachablePlayer);

        // A player RSVP'd to a DIFFERENT event this coach has no assignment
        // for must never appear.
        $unreachablePlayer = $this->freshPlayerMembership($trainer, 'UnreachablePlayer'.uniqid())->getPlayer();
        $otherEvent = $this->createEvent($trainer, ['title' => 'Other Trainer Event '.uniqid()]);
        $this->createRsvp($otherEvent, $unreachablePlayer);

        $this->client->loginUser($coachAccount);
        $crawler = $this->client->request('GET', '/coach/players');
        $text = $crawler->filter('body')->text();

        self::assertStringContainsString($reachablePlayer->getFirstName(), $text, 'AC-03-43: sees the player RSVP\'d to their own assigned event.');
        self::assertStringNotContainsString($unreachablePlayer->getFirstName(), $text, 'AC-03-64: never the trainer\'s full roster.');
    }

    /**
     * AC-03-65: player search for a coach is limited to their assigned
     * events — cannot search across the trainer's full player list.
     */
    public function testCoachSearchNeverReachesBeyondTheirAssignedEvents(): void
    {
        $trainer = $this->trainer('peak-performance');
        $coachAccount = $this->createCoach($trainer, 'search.coach.'.uniqid().'@example.test');
        $this->activateTenant($trainer);
        $coach = $this->coachMembershipFor($trainer, $coachAccount);

        $tag = 'SearchScope'.uniqid();
        $reachablePlayer = $this->freshPlayerMembership($trainer, 'Reachable'.$tag)->getPlayer();
        $event = $this->createEvent($trainer, ['title' => 'Search Scoped Event '.uniqid()]);
        $this->createCoachAssignment($event, $coach);
        $this->createRsvp($event, $reachablePlayer);

        $unreachablePlayer = $this->freshPlayerMembership($trainer, 'Unreachable'.$tag)->getPlayer();

        $this->client->loginUser($coachAccount);
        $crawler = $this->client->request('GET', '/coach/players?q='.$tag);
        $text = $crawler->filter('body')->text();

        self::assertStringContainsString($reachablePlayer->getFirstName(), $text);
        self::assertStringNotContainsString($unreachablePlayer->getFirstName(), $text, 'AC-03-65: search cannot surface a player outside reach.');
    }

    /**
     * AC-03-44: a coach-scoped player detail — attendance limited to
     * sessions with this coach, read-only labels/flags, past feedback.
     */
    public function testCoachPlayerDetailIsScopedToSharedSessionsOnly(): void
    {
        $trainer = $this->trainer('peak-performance');
        $coachAccount = $this->createCoach($trainer, 'detail.coach.'.uniqid().'@example.test');
        $this->activateTenant($trainer);
        $coach = $this->coachMembershipFor($trainer, $coachAccount);

        $player = $this->freshPlayerMembership($trainer)->getPlayer();
        $sharedEvent = $this->createEvent($trainer, ['title' => 'Shared Session '.uniqid()]);
        $this->createCoachAssignment($sharedEvent, $coach);
        $sharedRsvp = $this->createRsvp($sharedEvent, $player);
        $this->createAttendanceRecord($sharedEvent, $sharedRsvp, AttendanceRecord::STATUS_PRESENT, $coach);

        // A DIFFERENT coach records attendance for the same player elsewhere
        // — must not leak into this coach's own detail view.
        $otherCoachAccount = $this->createCoach($trainer, 'other.coach.'.uniqid().'@example.test');
        $otherCoach = $this->coachMembershipFor($trainer, $otherCoachAccount);
        $otherEvent = $this->createEvent($trainer, ['title' => 'Other Coach Session '.uniqid()]);
        $this->createCoachAssignment($otherEvent, $otherCoach);
        $otherRsvp = $this->createRsvp($otherEvent, $player);
        $this->createAttendanceRecord($otherEvent, $otherRsvp, AttendanceRecord::STATUS_PRESENT, $otherCoach);

        $membership = $this->membershipFor($trainer, $player);
        $this->client->loginUser($coachAccount);
        $crawler = $this->client->request('GET', '/coach/players/'.$membership->getId());

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', $sharedEvent->getTitle());
        self::assertSelectorTextNotContains('body', $otherEvent->getTitle());
    }

    /**
     * AC-03-44's note/Open architecture risk 6: a coach viewing a player
     * with no shared session history is unreachable (denied), not an empty
     * profile.
     */
    public function testCoachCannotViewAPlayerWithNoSharedSessionHistory(): void
    {
        $trainer = $this->trainer('peak-performance');
        $coachAccount = $this->createCoach($trainer, 'noshare.coach.'.uniqid().'@example.test');
        $unrelatedPlayer = $this->freshPlayerMembership($trainer, 'Unrelated'.uniqid());

        $this->client->loginUser($coachAccount);
        $this->client->request('GET', '/coach/players/'.$unrelatedPlayer->getId());

        self::assertResponseStatusCodeSame(403);
    }

    /**
     * AC-03-45: the coach cannot remove labels or flags — read-only, even
     * for a player they CAN reach.
     */
    public function testCoachCannotManageLabelsOrFlagsEvenForAReachablePlayer(): void
    {
        $trainer = $this->trainer('peak-performance');
        $coachAccount = $this->createCoach($trainer, 'readonly.coach.'.uniqid().'@example.test');
        $this->activateTenant($trainer);
        $coach = $this->coachMembershipFor($trainer, $coachAccount);

        $player = $this->freshPlayerMembership($trainer)->getPlayer();
        $event = $this->createEvent($trainer, ['title' => 'Readonly Event '.uniqid()]);
        $this->createCoachAssignment($event, $coach);
        $this->createRsvp($event, $player);
        $membership = $this->membershipFor($trainer, $player);

        $this->client->loginUser($coachAccount);
        /** @var AuthorizationCheckerInterface $authChecker */
        $authChecker = self::getContainer()->get(AuthorizationCheckerInterface::class);

        self::assertTrue($authChecker->isGranted(PlayerVoter::PLAYER_VIEW, $membership), 'The coach reaches this player at all.');
        self::assertFalse($authChecker->isGranted(PlayerVoter::PLAYER_LABEL_MANAGE, $membership), 'AC-03-45/AC-03-53: labels are trainer-only.');
        self::assertFalse($authChecker->isGranted(PlayerVoter::PLAYER_EDIT, $membership), 'AC-03-45: cannot edit player profiles.');
    }
}
