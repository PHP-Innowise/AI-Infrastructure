<?php

declare(strict_types=1);

namespace App\Tests\Scheduling;

use App\Identity\Entity\Account;
use App\Identity\Entity\ChildApprovalRequest;
use App\Identity\Repository\ChildApprovalRequestRepository;
use App\Platform\Entity\Trainer;
use App\Scheduling\Billing\FundingShortfall;
use App\Scheduling\Billing\PaymentIntentGateway;
use App\Scheduling\Billing\PaymentIntentRequest;
use App\Scheduling\Billing\PaymentIntentResult;
use App\Scheduling\Entity\Rsvp;
use App\Scheduling\Repository\RsvpRepository;
use App\Tests\Support\BillingFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\DomCrawler\Crawler;
use Symfony\Component\DomCrawler\Form;

/**
 * US-02.08 — Player Cancels RSVP.
 */
final class RsvpCancellationTest extends WebTestCase
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
     * AC-02-29: cancels from "My Reservations" via "Cancel RSVP", the
     * confirm copy is "Are you sure? This will free your spot.", and on
     * confirmation the RSVP is canceled, removed from the roster, the spot
     * opens, and a cancellation email is sent.
     */
    public function testFreeEventRsvpCanBeCanceledFromMyReservations(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, ['title' => 'Cancelable Free Session', 'capacity' => 5]);
        $this->createRsvp($event, $this->patPlayer());

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', '/portal/reservations');

        // AC-02-29's exact confirm copy is wired into every row's button.
        self::assertStringContainsString('Are you sure? This will free your spot.', (string) $this->client->getResponse()->getContent());

        $form = $this->cancelFormForEvent($crawler, 'Cancelable Free Session');
        $this->client->submit($form);

        self::assertResponseRedirects('/portal/reservations');
        self::assertQueuedEmailCount(1);
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', 'canceled');

        $this->activateTenant($trainer);
        /** @var RsvpRepository $rsvps */
        $rsvps = self::getContainer()->get(RsvpRepository::class);
        self::assertCount(0, $rsvps->findForEvent($event), 'AC-02-29: removed from the (active) roster.');
        self::assertSame(0, $rsvps->countHeld($event), 'AC-02-29: the spot is released.');
    }

    /**
     * AC-02-30: cannot cancel after the event has started — RsvpVoter's own
     * ownership+timing check denies before the service is ever reached.
     */
    public function testCannotCancelAfterEventStarted(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, [
            'title' => 'Already Started Session',
            'startsAt' => new \DateTimeImmutable('-2 hours'),
            'endsAt' => new \DateTimeImmutable('-1 hour'),
        ]);
        $rsvp = $this->createRsvp($event, $this->patPlayer());

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $this->client->request('POST', sprintf('/portal/rsvps/%d/cancel', $rsvp->getId()));

        self::assertResponseStatusCodeSame(403, 'AC-02-30: cannot cancel after the event has started.');
    }

    /**
     * AC-02-31/32, BR-02-11, AC-05-14/17: canceling a paid, confirmed RSVP
     * >= 24 hours before the event start refunds automatically ("Full
     * refund processed" — a Stripe refund initiated for USD, tracked in
     * transaction history via the new refund PaymentRecord row
     * `PaymentRecord::forRefund()` creates), and a refund confirmation
     * email is sent — exercised against a stub gateway reporting
     * Succeeded, same technique and rationale as
     * RsvpTest::testPaidEventConfirmsOnceGatewayReportsSuccess(). The
     * TOKEN half of AC-05-14/17 ("tokens returned... instant") is proven
     * separately, against the real Epic-05 ledger rather than a stub, by
     * testPaidTokenCancellationAtLeast24HoursOutRefundsTokensInstantly()
     * below.
     */
    public function testPaidCancellationAtLeast24HoursOutRefundsAndNotifies(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, [
            'title' => 'Far Out Paid Session',
            'startsAt' => new \DateTimeImmutable('+5 days'),
            'endsAt' => new \DateTimeImmutable('+5 days +1 hour'),
            'usdPricingEnabled' => true,
            'usdPriceMinorUnits' => 2000,
        ]);
        $rsvp = $this->createRsvp($event, $this->patPlayer(), Rsvp::METHOD_USD, Rsvp::STATUS_CONFIRMED);

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

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', '/portal/reservations');
        $form = $this->cancelFormForEvent($crawler, 'Far Out Paid Session');
        $this->client->submit($form);

        self::assertResponseRedirects('/portal/reservations');

        // disableReboot() makes assertQueuedEmailCount()'s own
        // profiler-based read unreliable (see RsvpTest's own note) — the
        // plain message list is checked directly instead.
        $messages = self::getMailerMessages();
        $subjects = array_map(static fn ($m) => method_exists($m, 'getSubject') ? $m->getSubject() : '', $messages);
        self::assertTrue(
            (bool) array_filter($subjects, static fn (string $s): bool => str_contains($s, 'Refund processed')),
            'AC-02-32: a refund confirmation email is among those sent.',
        );

        $this->activateTenant($trainer);
        /** @var RsvpRepository $rsvps */
        $rsvps = self::getContainer()->get(RsvpRepository::class);
        $reloaded = $rsvps->find($rsvp->getId());
        self::assertNotNull($reloaded);
        self::assertTrue($reloaded->isCanceled());
    }

    /**
     * AC-05-14/17: canceling a token-paid, confirmed RSVP >= 24 hours
     * before the event start refunds the tokens to the parent-trainer
     * balance INSTANTLY — proven against the real Epic-05 ledger (not a
     * stub), in the same request as the cancellation itself, with the
     * refund also showing up in transaction history as its own row
     * (BR-05-5's "partial token refunds are possible" is not exercised
     * separately — this RSVP's own full price is refunded in full, the
     * only shape BR-02-11/BR-05-10's 24-hour rule ever produces).
     */
    public function testPaidTokenCancellationAtLeast24HoursOutRefundsTokensInstantly(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $pat = $this->account('player@practiceperfect.test');
        $this->giveTokens($trainer, $pat, 10);
        $balanceBeforeRsvp = $this->tokenBalance($trainer, $pat);
        $event = $this->createEvent($trainer, [
            'title' => 'Far Out Token Session',
            'startsAt' => new \DateTimeImmutable('+5 days'),
            'endsAt' => new \DateTimeImmutable('+5 days +1 hour'),
            'tokenPricingEnabled' => true,
            'tokenPrice' => 4,
        ]);

        $this->client->loginUser($pat);
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', sprintf('/portal/events/%d', $event->getId()));
        $form = $crawler->selectButton('Register & Pay')->form(['rsvp[paymentMethod]' => Rsvp::METHOD_TOKEN]);
        $this->client->submit($form);
        self::assertResponseRedirects('/portal/reservations');

        $this->activateTenant($trainer);
        self::assertSame($balanceBeforeRsvp - 4, $this->tokenBalance($trainer, $pat), 'The RSVP spent 4 tokens.');
        /** @var RsvpRepository $rsvps */
        $rsvps = self::getContainer()->get(RsvpRepository::class);
        $rsvp = current($rsvps->findForEvent($event));
        self::assertNotFalse($rsvp);

        $crawler = $this->client->request('GET', '/portal/reservations');
        $form = $this->cancelFormForEvent($crawler, 'Far Out Token Session');
        $this->client->submit($form);

        self::assertResponseRedirects('/portal/reservations');

        $this->activateTenant($trainer);
        self::assertSame($balanceBeforeRsvp, $this->tokenBalance($trainer, $pat), 'AC-05-14/17: the tokens are back INSTANTLY — same request, no async step.');

        /** @var \App\Billing\Repository\PaymentRecordRepository $paymentRecords */
        $paymentRecords = self::getContainer()->get(\App\Billing\Repository\PaymentRecordRepository::class);
        $refundRow = current(array_filter(
            $paymentRecords->findForPayerHistory($pat),
            static fn ($p) => $p->isRefund() && $p->getRelatedRsvp()?->getId() === $rsvp->getId(),
        ));
        self::assertNotFalse($refundRow, 'AC-05-17: the refund is tracked in transaction history as its own row.');
        self::assertSame(4, $refundRow->getAmountMinorUnits(), 'The full 4 tokens are refunded — this RSVP\'s own full price.');
    }

    /**
     * BR-02-11, AC-05-15: canceling a paid, confirmed RSVP < 24 hours
     * before the event start does NOT refund — a warning is shown first
     * (`cancelFormForEvent()`'s own confirm-copy precedent covers the
     * generic confirm step; the 24-hour-specific wording is US-05.05's own
     * UI copy, not re-asserted here) and, on confirmation, only the
     * cancellation email is sent, never a refund confirmation (the
     * gateway's own refund endpoint is never even called, since
     * RsvpService checks 24-hour eligibility first — exercised here
     * against the REAL Epic-05 gateway, not a stub, since this path never
     * reaches it at all).
     */
    public function testPaidCancellationLessThan24HoursOutDoesNotRefund(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, [
            'title' => 'Soon Paid Session',
            'startsAt' => new \DateTimeImmutable('+5 hours'),
            'endsAt' => new \DateTimeImmutable('+6 hours'),
            'usdPricingEnabled' => true,
            'usdPriceMinorUnits' => 2000,
        ]);
        $this->createRsvp($event, $this->patPlayer(), Rsvp::METHOD_USD, Rsvp::STATUS_CONFIRMED);

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', '/portal/reservations');
        $form = $this->cancelFormForEvent($crawler, 'Soon Paid Session');
        $this->client->submit($form);

        self::assertResponseRedirects('/portal/reservations');
        self::assertQueuedEmailCount(1, message: 'BR-02-11: only the cancellation email — no refund, so no refund confirmation.');
    }

    /**
     * AC-02-33: a child's own cancellation attempt requires parent
     * approval, the same as a purchase — the RSVP stays confirmed until
     * decided. Approving it actually cancels the RSVP (with the same
     * refund policy a direct player cancellation would get); denying it
     * (covered separately below) leaves the RSVP untouched.
     */
    public function testChildInitiatedCancellationRequiresParentApproval(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $alex = $this->alexPlayer();
        $this->ensureActivePlayerMembership($trainer, $alex);
        $childAccount = $this->giveChildOwnLogin($alex, 'alex-cancel-login@practiceperfect.test', $trainer);
        $event = $this->createEvent($trainer, ['title' => 'Child Cancellation Needs Approval']);
        $rsvp = $this->createRsvp($event, $alex);

        $this->client->loginUser($childAccount);
        $crawler = $this->client->request('GET', '/portal/reservations');
        $form = $this->cancelFormForEvent($crawler, 'Child Cancellation Needs Approval');
        $this->client->submit($form);

        self::assertResponseRedirects('/portal/reservations');

        $this->activateTenant($trainer);
        /** @var RsvpRepository $rsvps */
        $rsvps = self::getContainer()->get(RsvpRepository::class);
        $stillConfirmed = $rsvps->find($rsvp->getId());
        self::assertNotNull($stillConfirmed);
        self::assertTrue($stillConfirmed->isConfirmed(), 'AC-02-33: not canceled yet — awaiting parent approval.');

        $cancellationRequest = $this->findPendingCancellationRequest();
        self::assertNotNull($cancellationRequest, 'A pending RSVP-cancellation approval request was created.');

        // The parent approves it — the RSVP is actually canceled now. The
        // trainer context must be selected explicitly (switchPlayerToTrainer's
        // own docblock) or, once Pat carries more than one trainer
        // relationship from an earlier test elsewhere in a full-suite run,
        // this trainer-scoped ChildApprovalRequest would 404.
        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', sprintf('/portal/rsvps/approvals/%d/approve', $cancellationRequest->getId()));
        $form = $crawler->selectButton('Approve')->form();
        $this->client->submit($form);

        self::assertResponseRedirects('/portal/approvals');

        $this->activateTenant($trainer);
        $rsvps2 = self::getContainer()->get(RsvpRepository::class);
        $nowCanceled = $rsvps2->find($rsvp->getId());
        self::assertNotNull($nowCanceled);
        self::assertTrue($nowCanceled->isCanceled(), 'AC-02-33: canceled once the parent approves.');
    }

    /**
     * AC-02-33's natural complement: a denied cancellation request leaves
     * the RSVP exactly as it was.
     */
    public function testDeniedCancellationRequestLeavesTheRsvpUntouched(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $alex = $this->alexPlayer();
        $this->ensureActivePlayerMembership($trainer, $alex);
        $childAccount = $this->giveChildOwnLogin($alex, 'alex-cancel-deny-login@practiceperfect.test', $trainer);
        $event = $this->createEvent($trainer, ['title' => 'Child Cancellation Gets Denied']);
        $rsvp = $this->createRsvp($event, $alex);

        $this->client->loginUser($childAccount);
        $crawler = $this->client->request('GET', '/portal/reservations');
        $form = $this->cancelFormForEvent($crawler, 'Child Cancellation Gets Denied');
        $this->client->submit($form);
        self::assertResponseRedirects('/portal/reservations');

        $this->activateTenant($trainer);
        $cancellationRequest = $this->findPendingCancellationRequest();
        self::assertNotNull($cancellationRequest);

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', sprintf('/portal/rsvps/approvals/%d/deny', $cancellationRequest->getId()));
        $form = $crawler->selectButton('Deny')->form();
        $this->client->submit($form);

        self::assertResponseRedirects('/portal/approvals');

        $this->activateTenant($trainer);
        /** @var RsvpRepository $rsvps */
        $rsvps = self::getContainer()->get(RsvpRepository::class);
        $reloaded = $rsvps->find($rsvp->getId());
        self::assertNotNull($reloaded);
        self::assertTrue($reloaded->isConfirmed(), 'A denied cancellation leaves the RSVP as it was.');
    }

    /**
     * "My Reservations" lists every one of Pat's RSVPs, including the
     * baseline fixture one from AppFixtures — never assume this test's own
     * event is the only row (or the first "Cancel RSVP" button on the
     * page): scope to the row naming this test's own event title, matching
     * the tr:contains() row-scoping technique CoachAssignmentTest already
     * establishes for the same reason.
     */
    private function cancelFormForEvent(Crawler $crawler, string $eventTitle): Form
    {
        return $crawler->filter(sprintf('tr:contains("%s")', $eventTitle))->selectButton('Cancel RSVP')->form();
    }

    /**
     * findForParent() returns every request regardless of status, and this
     * suite's own two cancellation-approval tests both run against the
     * same shared parent fixture with no per-test database reset — an
     * already-decided request from an earlier test method must never be
     * mistaken for this test's own fresh one, so isPending() is checked
     * here too, not just the action type.
     */
    private function findPendingCancellationRequest(): ?ChildApprovalRequest
    {
        /** @var ChildApprovalRequestRepository $requests */
        $requests = self::getContainer()->get(ChildApprovalRequestRepository::class);
        $pending = $requests->findForParent($this->account('player@practiceperfect.test'));
        $found = current(array_filter(
            $pending,
            static fn (ChildApprovalRequest $r): bool => ChildApprovalRequest::ACTION_RSVP_CANCELLATION === $r->getActionType() && $r->isPending(),
        ));

        return false === $found ? null : $found;
    }
}
