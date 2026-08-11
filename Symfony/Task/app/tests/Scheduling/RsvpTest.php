<?php

declare(strict_types=1);

namespace App\Tests\Scheduling;

use App\Identity\Entity\Account;
use App\Identity\Repository\ChildApprovalRequestRepository;
use App\Platform\Entity\Trainer;
use App\Scheduling\Billing\PaymentIntentGateway;
use App\Scheduling\Billing\PaymentIntentRequest;
use App\Scheduling\Billing\PaymentIntentResult;
use App\Scheduling\Entity\Event;
use App\Scheduling\Entity\Rsvp;
use App\Scheduling\Repository\RsvpRepository;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-02.07 — Player RSVPs to Event.
 */
final class RsvpTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-02-24, AC-05-31: a free event (both usdPricingEnabled and
     * tokenPricingEnabled off — createEvent()'s own defaults) RSVP is
     * instantly confirmed with no payment step of any kind — status
     * "Registered," added to "My Reservations," a "You're registered!"
     * message, and a confirmation email. The RSVP-side "no payment step"
     * mechanics are Epic-02's own BR-02-8 (AC-05-31's own cross-epic
     * note); this is the same underlying fact from Epic-05's "free events
     * are a payment-method category" angle.
     */
    public function testFreeEventRsvpConfirmsInstantlyAndAddsToReservations(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, ['title' => 'Free Instant Confirm Session']);

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', sprintf('/portal/events/%d', $event->getId()));
        $form = $crawler->selectButton('RSVP')->form();
        $this->client->submit($form);

        self::assertResponseRedirects('/portal/reservations');
        // Checked against THIS request's own collected messages, before
        // followRedirect() starts a new one.
        self::assertQueuedEmailCount(1);

        $this->client->followRedirect();
        self::assertSelectorTextContains('body', "You're registered!");
        self::assertSelectorTextContains('body', 'Free Instant Confirm Session');

        $this->activateTenant($trainer);
        /** @var RsvpRepository $rsvps */
        $rsvps = self::getContainer()->get(RsvpRepository::class);
        $rsvp = current($rsvps->findForEvent($event));
        self::assertNotFalse($rsvp);
        self::assertSame(Rsvp::STATUS_CONFIRMED, $rsvp->getStatus(), 'AC-02-24: status is Registered/Confirmed.');
    }

    /**
     * AC-02-23/BR-02-10: a child's own RSVP attempt requires parent
     * approval regardless of price — even for a FREE event, which would
     * otherwise auto-confirm for an adult. Uses a distinct login for the
     * child (giveChildOwnLogin()) since a parent's own context-switch
     * always bypasses (it IS the approval) — see that helper's own
     * docblock.
     */
    public function testChildRsvpRequiresParentApprovalRegardlessOfPrice(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $alex = $this->alexPlayer();
        $this->ensureActivePlayerMembership($trainer, $alex);
        $childAccount = $this->giveChildOwnLogin($alex, 'alex-own-login@practiceperfect.test', $trainer);
        $event = $this->createEvent($trainer, ['title' => 'Child Free Event Needs Approval']);

        $this->client->loginUser($childAccount);
        $crawler = $this->client->request('GET', sprintf('/portal/events/%d', $event->getId()));
        $form = $crawler->selectButton('RSVP')->form();
        $this->client->submit($form);

        self::assertResponseRedirects('/portal/approvals');

        $this->activateTenant($trainer);
        /** @var RsvpRepository $rsvps */
        $rsvps = self::getContainer()->get(RsvpRepository::class);
        $rsvp = current($rsvps->findForEvent($event));
        self::assertNotFalse($rsvp);
        self::assertSame(
            Rsvp::STATUS_PENDING_PARENT_APPROVAL,
            $rsvp->getStatus(),
            'AC-02-23/BR-02-10: pending parent approval, even though this event is free.',
        );

        /** @var ChildApprovalRequestRepository $requests */
        $requests = self::getContainer()->get(ChildApprovalRequestRepository::class);
        $pending = $requests->findForParent($this->account('player@practiceperfect.test'));
        self::assertNotEmpty($pending, 'A pending approval request was created for the parent.');
    }

    /**
     * AC-02-25/60, AC-05-10: a paid event shows the price(s) and, choosing
     * card, redirects straight to Stripe Checkout in the SAME request
     * (specs/api-designer-spec.md "Billing module": "no separate 'create
     * checkout session' endpoint") — the RSVP itself stays Pending Payment
     * until the webhook confirms it, never claiming a confirmation that has
     * not happened.
     */
    public function testPaidEventShowsPriceAndRedirectsToStripeCheckout(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, [
            'title' => 'Paid Session Awaiting Payment',
            'usdPricingEnabled' => true,
            'usdPriceMinorUnits' => 1000,
            'tokenPricingEnabled' => true,
            'tokenPrice' => 2,
        ]);

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', sprintf('/portal/events/%d', $event->getId()));

        // AC-02-60: both options shown clearly.
        self::assertSelectorTextContains('body', '$10.00');
        self::assertSelectorTextContains('body', '2 tokens');

        $form = $crawler->selectButton('Register & Pay')->form(['rsvp[paymentMethod]' => Event::PAYMENT_USD]);
        $this->client->submit($form);

        self::assertTrue($this->client->getResponse()->isRedirect(), 'AC-05-10: redirected to Stripe Checkout.');
        $location = (string) $this->client->getResponse()->headers->get('Location');
        self::assertStringContainsString('checkout.stripe.test', $location);

        $this->activateTenant($trainer);
        /** @var RsvpRepository $rsvps */
        $rsvps = self::getContainer()->get(RsvpRepository::class);
        $rsvp = current($rsvps->findForEvent($event));
        self::assertNotFalse($rsvp);
        self::assertSame(Rsvp::STATUS_PENDING_PAYMENT, $rsvp->getStatus());
        self::assertNotNull($rsvp->getPaymentRecord(), 'AC-05-36: a payment record backs the Checkout Session, idempotency-key-bearing.');
    }

    /**
     * AC-02-26: once payment succeeds, the RSVP is confirmed, "My
     * Reservations" reflects it, and a confirmation email is sent. The
     * shipped Epic-05 gateway always returns Pending for a `usd` payment
     * (Stripe Checkout is genuinely asynchronous — see
     * SchedulingPaymentIntentGateway's own docblock), so this exercises the
     * confirm-on-success branch with the container's PaymentIntentGateway
     * swapped for a stub that reports Succeeded — proving RsvpService's own
     * handling of that outcome against the real interface, matching the
     * exact shape a `token` payment (which DOES resolve Succeeded
     * synchronously) produces in production.
     */
    public function testPaidEventConfirmsOnceGatewayReportsSuccess(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, [
            'title' => 'Paid Session Gateway Succeeds',
            'usdPricingEnabled' => true,
            'usdPriceMinorUnits' => 500,
        ]);

        // KernelBrowser reboots the kernel (a fresh container) before each
        // request by default — disableReboot() keeps this same container,
        // and this override, alive for every request in this method. Must
        // be set before the client's very first request, or a fresh,
        // rebooted container would fall right back to the real,
        // Epic-05-wired gateway.
        $this->client->disableReboot();
        self::getContainer()->set(PaymentIntentGateway::class, new class implements PaymentIntentGateway {
            public function lockFundingForUpdate(Trainer $trainer, Account $payer, string $paymentMethod): void
            {
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

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', sprintf('/portal/events/%d', $event->getId()));
        $form = $crawler->selectButton('Register & Pay')->form(['rsvp[paymentMethod]' => Event::PAYMENT_USD]);
        $this->client->submit($form);

        self::assertResponseRedirects('/portal/reservations');
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', "You're registered!");

        $this->activateTenant($trainer);
        /** @var RsvpRepository $rsvps */
        $rsvps = self::getContainer()->get(RsvpRepository::class);
        $rsvp = current($rsvps->findForEvent($event));
        self::assertNotFalse($rsvp);
        self::assertSame(Rsvp::STATUS_CONFIRMED, $rsvp->getStatus(), 'AC-02-26: confirmed once payment succeeds.');
        self::assertNotNull($rsvp->getConfirmedAt());
    }

    /**
     * AC-05-13: a child's RSVP that requires payment is held "Pending
     * Parent Approval"; on approval a Stripe Checkout opens for the
     * parent — the same-request redirect
     * `PortalReservationController::decideApproval()`'s own docblock
     * documents, proven here end to end through the real approval flow
     * rather than assumed from that docblock alone.
     */
    public function testChildsPaidRsvpAwaitsParentApprovalThenOpensStripeCheckoutForTheParent(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $alex = $this->alexPlayer();
        $this->ensureActivePlayerMembership($trainer, $alex);
        $childAccount = $this->giveChildOwnLogin($alex, 'alex-paid-rsvp-login@practiceperfect.test', $trainer);
        $event = $this->createEvent($trainer, [
            'title' => 'Child Paid Session Needs Approval',
            'usdPricingEnabled' => true,
            'usdPriceMinorUnits' => 1500,
        ]);

        $this->client->loginUser($childAccount);
        $crawler = $this->client->request('GET', sprintf('/portal/events/%d', $event->getId()));
        $form = $crawler->selectButton('Register & Pay')->form(['rsvp[paymentMethod]' => Event::PAYMENT_USD]);
        $this->client->submit($form);

        self::assertResponseRedirects('/portal/approvals');

        $this->activateTenant($trainer);
        /** @var RsvpRepository $rsvps */
        $rsvps = self::getContainer()->get(RsvpRepository::class);
        $rsvp = current($rsvps->findForEvent($event));
        self::assertNotFalse($rsvp);
        self::assertSame(Rsvp::STATUS_PENDING_PARENT_APPROVAL, $rsvp->getStatus(), 'AC-05-13: held Pending Parent Approval.');

        /** @var ChildApprovalRequestRepository $requests */
        $requests = self::getContainer()->get(ChildApprovalRequestRepository::class);
        // Matched on THIS rsvp's own id, not merely "any pending rsvp-type
        // request for Pat" — testChildRsvpRequiresParentApprovalRegardlessOfPrice()
        // above deliberately leaves its own pending 'rsvp' request
        // unresolved for the rest of this process (no per-test database
        // reset), so a looser filter could grab that stale one instead of
        // this test's own fresh one.
        $pending = current(array_filter(
            $requests->findForParent($this->account('player@practiceperfect.test')),
            static fn ($r) => $r->isPending() && 'rsvp' === $r->getActionType() && $r->getRsvpId() === (int) $rsvp->getId(),
        ));
        self::assertNotFalse($pending, 'AC-05-13: the parent is notified via a pending approval request.');

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', sprintf('/portal/rsvps/approvals/%d/approve', $pending->getId()));
        $form = $crawler->selectButton('Approve')->form();
        $this->client->submit($form);

        self::assertTrue($this->client->getResponse()->isRedirect(), 'AC-05-13: approval opens a Stripe Checkout for the parent.');
        self::assertStringContainsString('checkout.stripe.test', (string) $this->client->getResponse()->headers->get('Location'));

        $this->activateTenant($trainer);
        $rsvps2 = self::getContainer()->get(RsvpRepository::class);
        $reloaded = $rsvps2->find($rsvp->getId());
        self::assertNotNull($reloaded);
        self::assertSame(Rsvp::STATUS_PENDING_PAYMENT, $reloaded->getStatus(), 'AC-05-13: awaiting the parent\'s own Checkout completion — not yet registered.');
    }

    /**
     * AC-02-27: a full event shows "Event Full" and offers no RSVP
     * control — viewed by a player who is NOT the one holding the single
     * spot, so the "Already registered" branch (checked first in the
     * template) cannot mask this assertion.
     */
    public function testFullEventShowsEventFullAndNoRsvpControl(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $alex = $this->alexPlayer();
        $this->ensureActivePlayerMembership($trainer, $alex);
        $event = $this->createEvent($trainer, ['title' => 'Already Full Session', 'capacity' => 1]);
        $this->createRsvp($event, $this->patPlayer());

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $this->switchToChild($this->client, 'Alex');
        $crawler = $this->client->request('GET', sprintf('/portal/events/%d', $event->getId()));

        self::assertSelectorTextContains('body', 'Event Full');
        self::assertSelectorTextContains('body', 'Check back later or contact the trainer');
        self::assertCount(0, $crawler->selectButton('RSVP'), 'AC-02-27: the RSVP control is not offered once full.');
    }

    /**
     * AC-02-28: cannot RSVP twice ("Already registered"), cannot RSVP if
     * ineligible (age here), and cannot RSVP to a past event.
     */
    public function testCannotRsvpTwiceOrWhenIneligibleOrToAPastEvent(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $pat = $this->patPlayer();

        $alreadyRegistered = $this->createEvent($trainer, ['title' => 'Already Registered Session']);
        $this->createRsvp($alreadyRegistered, $pat);

        $tooOld = $this->createEvent($trainer, ['title' => 'Ineligible By Age', 'minAge' => 1, 'maxAge' => 2]);

        $past = $this->createEvent($trainer, [
            'title' => 'Past Session',
            'startsAt' => new \DateTimeImmutable('-3 days'),
            'endsAt' => new \DateTimeImmutable('-3 days +1 hour'),
        ]);

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);

        $this->client->request('GET', sprintf('/portal/events/%d', $alreadyRegistered->getId()));
        self::assertSelectorTextContains('body', 'Already registered', 'AC-02-28: shown "Already registered".');
        $this->client->request('POST', sprintf('/portal/events/%d/rsvp', $alreadyRegistered->getId()));
        self::assertResponseStatusCodeSame(403, 'AC-02-28: a duplicate RSVP attempt is refused.');

        $this->client->request('GET', sprintf('/portal/events/%d', $tooOld->getId()));
        self::assertResponseStatusCodeSame(403, 'AC-02-28: an ineligible (by age) player cannot even view the event.');

        $this->client->request('POST', sprintf('/portal/events/%d/rsvp', $past->getId()));
        self::assertResponseStatusCodeSame(403, 'AC-02-28: cannot RSVP to a past event.');
    }
}
