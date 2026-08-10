<?php

declare(strict_types=1);

namespace App\Tests\Scheduling;

use App\Scheduling\Entity\Event;
use App\Scheduling\Repository\EventInvitationRepository;
use App\Scheduling\Repository\EventRepository;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-02.02 — Trainer Creates Private Event.
 */
final class PrivateEventTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-02-4, AC-02-5, AC-02-6: same flow as a public event, visibility set
     * to Private / Invite Only, individual player selection (multi-select,
     * not group-based) — saving creates the event, visible only in invited
     * players' calendars.
     */
    public function testTrainerCreatesAPrivateEventWithIndividualInvitees(): void
    {
        $trainer = $this->trainer('peak-performance');
        // Alex's own active membership may have been removed by an earlier
        // test elsewhere in this same run (see ensureActivePlayerMembership's
        // own docblock) — must be active to appear in the invite choice list.
        $this->ensureActivePlayerMembership($trainer, $this->alexPlayer());
        $alexId = $this->alexPlayer()->getId();

        $this->client->loginUser($trainer->getOwnerAccount());

        $crawler = $this->client->request('GET', '/trainer/events/new');
        $form = $crawler->selectButton('Create event')->form([
            'event[title]' => 'Private Coaching',
            'event[eventType]' => Event::TYPE_PRIVATE_SESSION,
            'event[startsAt]' => (new \DateTimeImmutable('+3 days'))->format('Y-m-d\TH:i'),
            'event[endsAt]' => (new \DateTimeImmutable('+3 days +1 hour'))->format('Y-m-d\TH:i'),
            'event[location]' => 'Main Gym',
            'event[capacity]' => '1',
            'event[visibility]' => Event::VISIBILITY_PRIVATE,
            'event[invitedPlayers]' => [(string) $alexId],
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var EventRepository $events */
        $events = self::getContainer()->get(EventRepository::class);
        $created = current(array_filter($events->findAllForActiveTenant(), static fn (Event $e): bool => 'Private Coaching' === $e->getTitle()));
        self::assertNotFalse($created);
        self::assertTrue($created->isPrivate(), 'AC-02-4: visibility is Private / Invite Only.');

        /** @var EventInvitationRepository $invitations */
        $invitations = self::getContainer()->get(EventInvitationRepository::class);
        self::assertTrue($invitations->isPlayerInvited($created, $this->alexPlayer()), 'AC-02-5, AC-02-6: the selected player is invited.');
    }

    /**
     * AC-02-7/BR-02-6: a non-invited player cannot see the private event
     * even if otherwise eligible, and direct-link access shows "Access
     * Denied".
     */
    public function testNonInvitedPlayerCannotAccessAPrivateEventViaDirectLink(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, ['visibility' => Event::VISIBILITY_PRIVATE, 'title' => 'Invite Only']);
        $this->createInvitation($event, $this->alexPlayer(), $trainer->getOwnerAccount());

        // Pat (the parent, acting as themselves — not Alex) was never
        // invited, even though Pat is otherwise an eligible, active player
        // under this trainer.
        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $this->client->request('GET', sprintf('/portal/events/%d', $event->getId()));

        self::assertResponseStatusCodeSame(403);
    }

    /**
     * AC-02-7: the invited player CAN see and reach the private event.
     */
    public function testInvitedPlayerCanAccessThePrivateEvent(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->ensureActivePlayerMembership($trainer, $this->alexPlayer());
        $event = $this->createEvent($trainer, ['visibility' => Event::VISIBILITY_PRIVATE, 'title' => 'Invite Only For Alex']);
        $this->createInvitation($event, $this->alexPlayer(), $trainer->getOwnerAccount());

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $this->switchToChild('Alex');

        $this->client->request('GET', sprintf('/portal/events/%d', $event->getId()));
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('h1', 'Invite Only For Alex');
    }

    /**
     * AC-02-19/BR-02-6: a private event does not appear in the Training
     * Calendar for a non-invited (but otherwise eligible) player.
     */
    public function testPrivateEventDoesNotAppearInCalendarForNonInvitedPlayer(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->createEvent($trainer, ['visibility' => Event::VISIBILITY_PRIVATE, 'title' => 'Hidden Session']);

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $this->client->request('GET', '/portal/calendar');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextNotContains('body', 'Hidden Session');
    }

    private function switchToChild(string $firstName): void
    {
        $crawler = $this->client->request('GET', '/portal/family');
        $form = $crawler->selectButton('Switch to '.$firstName)->form();
        $this->client->submit($form);
    }
}
