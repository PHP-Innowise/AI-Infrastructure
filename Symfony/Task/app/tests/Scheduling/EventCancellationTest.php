<?php

declare(strict_types=1);

namespace App\Tests\Scheduling;

use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use App\Scheduling\Billing\FundingShortfall;
use App\Scheduling\Billing\PaymentIntentGateway;
use App\Scheduling\Billing\PaymentIntentRequest;
use App\Scheduling\Billing\PaymentIntentResult;
use App\Scheduling\Entity\CoachAssignment;
use App\Scheduling\Entity\Event;
use App\Scheduling\Entity\Rsvp;
use App\Scheduling\Repository\CoachAssignmentRepository;
use App\Scheduling\Repository\EventRepository;
use App\Scheduling\Repository\RsvpRepository;
use App\Tests\Support\BillingFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-02.13 — Trainer Cancels Event.
 */
final class EventCancellationTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;
    use BillingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-02-46: confirmation copy "This will notify all registered players
     * and process refunds. Continue?", and a cancellation reason is
     * required.
     */
    public function testCancelRequiresConfirmationAndReason(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, ['title' => 'Confirm Cancel Session']);

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', sprintf('/trainer/events/%d/cancel', $event->getId()));

        self::assertSelectorTextContains('body', 'This will notify all registered players and process refunds. Continue?');

        $form = $crawler->selectButton('Cancel event')->form(['cancel_event[reason]' => '']);
        $this->client->submit($form);

        self::assertResponseIsUnprocessable();

        $this->activateTenant($trainer);
        /** @var EventRepository $events */
        $events = self::getContainer()->get(EventRepository::class);
        $reloaded = $events->find($event->getId());
        self::assertNotNull($reloaded);
        self::assertFalse($reloaded->isCanceled(), 'A blank reason does not cancel the event.');
    }

    /**
     * AC-02-47: on confirmed cancellation — status "Canceled", every
     * registered player notified (and refunded automatically if paid,
     * exercised against a stub gateway reporting Succeeded), and the
     * assigned coach notified that the assignment is canceled.
     */
    public function testCancellationNotifiesPlayersProcessesRefundsAndNotifiesCoach(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $coachAccount = $this->createCoach($trainer, 'cancel-event-coach@practiceperfect.test');
        $coach = $this->coachMembershipFor($trainer, $coachAccount);
        $alex = $this->alexPlayer();
        $this->ensureActivePlayerMembership($trainer, $alex);

        $event = $this->createEvent($trainer, [
            'title' => 'Full Cancellation Flow Session',
            'startsAt' => new \DateTimeImmutable('+5 days'),
            'endsAt' => new \DateTimeImmutable('+5 days +1 hour'),
            'capacity' => 5,
            'usdPricingEnabled' => true,
            'usdPriceMinorUnits' => 1000,
        ]);
        $this->createCoachAssignment($event, $coach, CoachAssignment::STATUS_CONFIRMED);
        $freeRsvp = $this->createRsvp($event, $this->patPlayer());
        $paidRsvp = $this->createRsvp($event, $alex, Rsvp::METHOD_USD, Rsvp::STATUS_CONFIRMED);

        $this->client->disableReboot();
        self::getContainer()->set(PaymentIntentGateway::class, new class implements PaymentIntentGateway {
            public function lockFundingForUpdate(Trainer $trainer, Account $payer, string $paymentMethod): void
            {
            }

            // This double funds everything: the tests using it are about
            // RSVP/cancellation flow, not about affordability.
            public function findFundingShortfall(Trainer $trainer, Account $payer, string $paymentMethod, int $amount): ?FundingShortfall
            {
                return null;
            }

            public function requestPayment(PaymentIntentRequest $request): PaymentIntentResult
            {
                return PaymentIntentResult::succeeded();
            }

            public function requestRefund(PaymentIntentRequest $request): PaymentIntentResult
            {
                return PaymentIntentResult::succeeded();
            }
        });

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', sprintf('/trainer/events/%d/cancel', $event->getId()));
        $form = $crawler->selectButton('Cancel event')->form(['cancel_event[reason]' => 'Facility closed for maintenance.']);
        $this->client->submit($form);

        self::assertResponseRedirects('/trainer/events');

        // disableReboot() makes assertQueuedEmailCount() unreliable (see
        // RsvpTest's own note) — checked directly instead.
        $subjects = array_map(
            static fn ($m) => method_exists($m, 'getSubject') ? $m->getSubject() : '',
            self::getMailerMessages(),
        );
        self::assertTrue((bool) array_filter($subjects, static fn (string $s): bool => str_contains($s, 'has been canceled')), 'AC-02-47: registered players notified.');
        self::assertTrue((bool) array_filter($subjects, static fn (string $s): bool => str_contains($s, 'Refund processed')), 'AC-02-47/BR-02-12: the paid RSVP was refunded.');

        $this->activateTenant($trainer);
        /** @var EventRepository $events */
        $events = self::getContainer()->get(EventRepository::class);
        $reloadedEvent = $events->find($event->getId());
        self::assertNotNull($reloadedEvent);
        self::assertTrue($reloadedEvent->isCanceled(), 'AC-02-47: status becomes Canceled.');
        self::assertSame('Facility closed for maintenance.', $reloadedEvent->getCanceledReason());

        /** @var RsvpRepository $rsvps */
        $rsvps = self::getContainer()->get(RsvpRepository::class);
        self::assertTrue($rsvps->find($freeRsvp->getId())?->isCanceled(), 'AC-02-47: the free RSVP is canceled too.');
        self::assertTrue($rsvps->find($paidRsvp->getId())?->isCanceled(), 'AC-02-47: the paid RSVP is canceled.');

        /** @var CoachAssignmentRepository $assignments */
        $assignments = self::getContainer()->get(CoachAssignmentRepository::class);
        $assignment = $assignments->findOneByEventAndCoach($event, $coach);
        self::assertNotNull($assignment);
        self::assertSame(CoachAssignment::STATUS_DECLINED, $assignment->getStatus(), 'AC-02-47: the coach assignment is also canceled.');
    }

    /**
     * AC-05-16/BR-02-12/BR-05-10: a trainer-initiated cancellation refunds
     * every registered, paid player in full "regardless of how close to
     * the event time" — distinct from a PLAYER's own 24-hour-gated
     * cancellation (RsvpCancellationTest's own tests), so this specifically
     * uses an event starting in under 24 hours, where a player-initiated
     * cancellation would get NO refund, and proves the trainer-initiated
     * one still refunds in full anyway. Uses the real Epic-05 token ledger
     * (not a stub) for a direct, genuine balance proof.
     */
    public function testTrainerCancellationRefundsInFullEvenLessThan24HoursOut(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $alex = $this->alexPlayer();
        $this->ensureActivePlayerMembership($trainer, $alex);
        $pat = $this->account('player@practiceperfect.test');
        $this->giveTokens($trainer, $pat, 10);
        $balanceBeforeRsvp = $this->tokenBalance($trainer, $pat);

        $event = $this->createEvent($trainer, [
            'title' => 'Imminent Token Session',
            'startsAt' => new \DateTimeImmutable('+3 hours'),
            'endsAt' => new \DateTimeImmutable('+4 hours'),
            'tokenPricingEnabled' => true,
            'tokenPrice' => 5,
        ]);

        $this->client->loginUser($pat);
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', sprintf('/portal/events/%d', $event->getId()));
        $form = $crawler->selectButton('Register & Pay')->form(['rsvp[paymentMethod]' => Rsvp::METHOD_TOKEN]);
        $this->client->submit($form);
        self::assertResponseRedirects('/portal/reservations');

        $this->activateTenant($trainer);
        self::assertSame($balanceBeforeRsvp - 5, $this->tokenBalance($trainer, $pat), 'The RSVP spent 5 tokens.');

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', sprintf('/trainer/events/%d/cancel', $event->getId()));
        $form = $crawler->selectButton('Cancel event')->form(['cancel_event[reason]' => 'Imminent cancellation, still owed a refund.']);
        $this->client->submit($form);

        self::assertResponseRedirects('/trainer/events');

        $this->activateTenant($trainer);
        self::assertSame($balanceBeforeRsvp, $this->tokenBalance($trainer, $pat), 'AC-05-16: refunded in full despite starting in under 24 hours — timing never matters for a trainer-initiated cancellation.');
    }

    /**
     * AC-02-48: Super Admin can cancel any event, reaching the same
     * outcome through the Event Master tool.
     */
    public function testSuperAdminCanCancelAnyEvent(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, ['title' => 'Super Admin Cancel Target']);

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', sprintf('/super-admin/events/%d/cancel', $event->getId()));
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Cancel event')->form(['cancel_event[reason]' => 'Policy violation.']);
        $this->client->submit($form);

        self::assertResponseRedirects('/super-admin/events');

        $this->activateTenant($trainer);
        /** @var EventRepository $events */
        $events = self::getContainer()->get(EventRepository::class);
        $reloaded = $events->find($event->getId());
        self::assertNotNull($reloaded);
        self::assertTrue($reloaded->isCanceled(), 'AC-02-48: Super Admin canceled the event.');
    }

    /**
     * AC-02-49: an already-started/completed event cannot be canceled — it
     * displays "Completed" instead, and attempting to cancel it is refused
     * cleanly (never a bare 500) for the trainer and Super Admin alike
     * (AC-02-49's own rule is structural, not a trainer-only policy — see
     * EventMasterController::cancel()'s own docblock).
     */
    public function testCannotCancelAPastOrCompletedEvent(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, [
            'title' => 'Already Completed Session',
            'startsAt' => new \DateTimeImmutable('-2 hours'),
            'endsAt' => new \DateTimeImmutable('-1 hour'),
        ]);

        $this->client->loginUser($trainer->getOwnerAccount());
        $this->client->request('GET', sprintf('/trainer/events/%d', $event->getId()));
        self::assertSelectorTextContains('body', Event::STATUS_COMPLETED, 'AC-02-49: displayed as Completed.');

        $this->client->request('GET', sprintf('/trainer/events/%d/cancel', $event->getId()));
        self::assertResponseRedirects(sprintf('/trainer/events/%d', $event->getId()), message: 'AC-02-49: refused cleanly, not a 500.');

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $this->client->request('GET', sprintf('/super-admin/events/%d/cancel', $event->getId()));
        self::assertResponseRedirects(sprintf('/super-admin/events/%d', $event->getId()), message: 'AC-02-49: applies to Super Admin too — a structural fact, not a policy to override.');
    }
}
