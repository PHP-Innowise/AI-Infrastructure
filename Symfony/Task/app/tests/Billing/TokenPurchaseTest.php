<?php

declare(strict_types=1);

namespace App\Tests\Billing;

use App\Billing\Repository\PaymentRecordRepository;
use App\Identity\Repository\ChildApprovalRequestRepository;
use App\Tests\Support\BillingFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use App\Tests\Support\WebhookDeliveryHelper;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-05.02 — Player Purchases Tokens.
 */
final class TokenPurchaseTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;
    use BillingFixtureHelpers;
    use WebhookDeliveryHelper;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-05-4: buying a listed package — sees the balance, picks a
     * package, is redirected to Stripe Checkout, and once the webhook
     * confirms payment the balance updates with a confirmation email.
     */
    public function testAdultBuysAListedTokenPackageAndBalanceUpdatesOnceWebhookConfirms(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $package = $this->createTokenPackage($trainer, '10 tokens / $90', ['tokenCount' => 10, 'priceMinorUnits' => 9000]);
        $pat = $this->account('player@practiceperfect.test');
        $balanceBefore = $this->tokenBalance($trainer, $pat);

        $this->client->loginUser($pat);
        $this->switchPlayerToTrainer($this->client, $trainer);

        $crawler = $this->client->request('GET', '/portal/tokens');
        self::assertSelectorTextContains('body', (string) $balanceBefore, 'AC-05-4: "You have N tokens with [Trainer]."');

        $crawler = $this->client->request('GET', '/portal/tokens/purchase');
        $form = $crawler->selectButton('Purchase')->form([
            'token_purchase[packageId]' => (string) $package->getId(),
        ]);
        $this->client->submit($form);

        self::assertTrue($this->client->getResponse()->isRedirect(), 'AC-05-4: redirected to Stripe Checkout.');
        $checkoutUrl = (string) $this->client->getResponse()->headers->get('Location');
        self::assertStringContainsString('checkout.stripe.test', $checkoutUrl);

        // The balance does not move yet — only the webhook confirming
        // payment credits the ledger (ProcessStripeWebhookEventHandler's
        // own docblock on handlePaymentIntentSucceeded()).
        self::assertSame($balanceBefore, $this->tokenBalance($trainer, $pat), 'Not credited until the webhook confirms.');

        $this->activateTenant($trainer);
        /** @var PaymentRecordRepository $paymentRecords */
        $paymentRecords = self::getContainer()->get(PaymentRecordRepository::class);
        // Matched on THIS package, not merely "a pending token-purchase
        // record for Pat" — see testChildInitiatedTokenPurchaseQueuesForParentApprovalThenCompletes()'s
        // own comment on why a looser filter is unsafe here.
        $pending = current(array_filter(
            $paymentRecords->findForPayerHistory($pat),
            static fn ($p) => 'token_purchase' === $p->getType() && $p->isPending() && $p->getRelatedTokenPackage()?->getId() === $package->getId(),
        ));
        self::assertNotFalse($pending, 'A pending token-purchase payment record backs the Checkout Session.');

        $this->deliverPaymentIntentSucceeded((string) $pending->getStripePaymentIntentId());

        $this->activateTenant($trainer);
        self::assertSame($balanceBefore + 10, $this->tokenBalance($trainer, $pat), 'AC-05-4: balance updates once Checkout completes.');

        $subjects = array_map(static fn ($m) => method_exists($m, 'getSubject') ? $m->getSubject() : '', self::getMailerMessages());
        self::assertTrue((bool) array_filter($subjects, static fn (string $s): bool => str_contains($s, 'Token')), 'AC-05-4: a confirmation email is sent.');
    }

    /**
     * AC-05-4: "or a custom amount" — priced at the trainer's own
     * $/token rate.
     */
    public function testAdultBuysACustomTokenAmount(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $pat = $this->account('player@practiceperfect.test');

        $this->client->loginUser($pat);
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', '/portal/tokens/purchase');
        $form = $crawler->selectButton('Purchase')->form([
            'token_purchase[customTokenCount]' => '7',
        ]);
        $this->client->submit($form);

        self::assertTrue($this->client->getResponse()->isRedirect(), 'AC-05-4: a custom amount also redirects to Checkout.');
        self::assertStringContainsString('checkout.stripe.test', (string) $this->client->getResponse()->headers->get('Location'));
    }

    /**
     * AC-05-5, BR-05-2: tokens are stored and usable at the parent-trainer
     * level, not per child — gifted once, spent by two DIFFERENT players
     * (the parent acting as themselves, and the parent's own child) draws
     * from the SAME single balance. The second half of this same test also
     * IS AC-05-9's own scenario: "a parent RSVPing a child with tokens
     * deducts from the parent's token balance with that trainer (not a
     * child-specific balance), and the child is registered."
     */
    public function testTokenBalanceIsSharedAcrossAllOfTheParentsPlayersAtThatTrainer(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $pat = $this->account('player@practiceperfect.test');
        $balanceBefore = $this->tokenBalance($trainer, $pat);
        $this->giveTokens($trainer, $pat, 10);
        $alex = $this->alexPlayer();
        $this->ensureActivePlayerMembership($trainer, $alex);

        $eventForPat = $this->createEvent($trainer, ['title' => 'Pat Token Session', 'tokenPricingEnabled' => true, 'tokenPrice' => 3]);
        $eventForAlex = $this->createEvent($trainer, ['title' => 'Alex Token Session', 'tokenPricingEnabled' => true, 'tokenPrice' => 2]);

        $this->client->loginUser($pat);
        $this->switchPlayerToTrainer($this->client, $trainer);

        // Pat spends on themselves.
        $crawler = $this->client->request('GET', sprintf('/portal/events/%d', $eventForPat->getId()));
        $form = $crawler->selectButton('Register & Pay')->form(['rsvp[paymentMethod]' => 'token']);
        $this->client->submit($form);
        self::assertResponseRedirects('/portal/reservations');

        $this->activateTenant($trainer);
        self::assertSame($balanceBefore + 10 - 3, $this->tokenBalance($trainer, $pat), 'AC-05-5: Pat\'s own spend draws from the shared balance.');

        // The SAME parent, now acting for Alex — a context-switch, which
        // "always bypasses" approval since it IS the parent's own action
        // (RsvpTest's own precedent).
        $this->switchToChild($this->client, 'Alex');
        $crawler = $this->client->request('GET', sprintf('/portal/events/%d', $eventForAlex->getId()));
        $form = $crawler->selectButton('Register & Pay')->form(['rsvp[paymentMethod]' => 'token']);
        $this->client->submit($form);
        self::assertResponseRedirects('/portal/reservations');

        $this->activateTenant($trainer);
        self::assertSame($balanceBefore + 10 - 3 - 2, $this->tokenBalance($trainer, $pat), 'AC-05-5/9: Alex\'s spend ALSO draws from the same parent-trainer balance, not a separate one.');

        /** @var \App\Scheduling\Repository\RsvpRepository $rsvps */
        $rsvps = self::getContainer()->get(\App\Scheduling\Repository\RsvpRepository::class);
        $alexRsvp = current($rsvps->findForEvent($eventForAlex));
        self::assertNotFalse($alexRsvp);
        self::assertSame(\App\Scheduling\Entity\Rsvp::STATUS_CONFIRMED, $alexRsvp->getStatus(), 'AC-05-9: the child is registered.');
    }

    /**
     * AC-05-6: a child with their own login sees the parent's balance and
     * can initiate a purchase, which queues for parent approval; only once
     * the parent approves does the purchase complete (redirected to
     * Checkout) and, once THAT completes, the tokens land in the
     * parent-trainer balance.
     */
    public function testChildInitiatedTokenPurchaseQueuesForParentApprovalThenCompletes(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $package = $this->createTokenPackage($trainer, '10 tokens / $90 (child flow)', ['tokenCount' => 10, 'priceMinorUnits' => 9000]);
        $alex = $this->alexPlayer();
        $this->ensureActivePlayerMembership($trainer, $alex);
        $childAccount = $this->giveChildOwnLogin($alex, 'alex-token-purchase-login@practiceperfect.test', $trainer);
        $pat = $this->account('player@practiceperfect.test');
        $balanceBefore = $this->tokenBalance($trainer, $pat);

        $this->client->loginUser($childAccount);
        $crawler = $this->client->request('GET', '/portal/tokens');
        self::assertSelectorTextContains('body', (string) $balanceBefore, 'AC-05-6: the child viewing tokens sees the PARENT\'S balance.');

        $crawler = $this->client->request('GET', '/portal/tokens/purchase');
        $form = $crawler->selectButton('Purchase')->form([
            'token_purchase[packageId]' => (string) $package->getId(),
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/portal/tokens', message: 'AC-05-6: queued for parent approval, no Checkout redirect yet.');
        self::assertSame($balanceBefore, $this->tokenBalance($trainer, $pat));

        $this->activateTenant($trainer);
        /** @var ChildApprovalRequestRepository $requests */
        $requests = self::getContainer()->get(ChildApprovalRequestRepository::class);
        $pending = current(array_filter($requests->findForParent($pat), static fn ($r) => $r->isPending() && 'token_purchase' === $r->getActionType()));
        self::assertNotFalse($pending, 'A pending token-purchase approval request was created.');
        self::assertSame($package->getId(), $pending->getRequestedTokenPackageId());

        // The parent approves — only NOW does the purchase actually
        // proceed to Stripe Checkout.
        $this->client->loginUser($pat);
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', sprintf('/portal/tokens/purchase-approvals/%d/approve', $pending->getId()));
        $form = $crawler->selectButton('Approve')->form();
        $this->client->submit($form);

        self::assertTrue($this->client->getResponse()->isRedirect(), 'AC-05-6: approval redirects straight to Stripe Checkout.');
        self::assertStringContainsString('checkout.stripe.test', (string) $this->client->getResponse()->headers->get('Location'));

        $this->activateTenant($trainer);
        /** @var PaymentRecordRepository $paymentRecords */
        $paymentRecords = self::getContainer()->get(PaymentRecordRepository::class);
        // Matched on THIS package specifically, not merely "the most recent
        // pending token-purchase record for Pat" — testAdultBuysACustomTokenAmount()
        // above deliberately never delivers its own webhook, leaving ITS
        // OWN pending record for the rest of this process, and
        // `created_at` is TIMESTAMP(0) (whole-second precision), so two
        // records created within the same second do not sort reliably by
        // "most recent" alone.
        $pendingRecord = current(array_filter(
            $paymentRecords->findForPayerHistory($pat),
            static fn ($p) => 'token_purchase' === $p->getType() && $p->isPending() && $p->getRelatedTokenPackage()?->getId() === $package->getId(),
        ));
        self::assertNotFalse($pendingRecord);

        $this->deliverPaymentIntentSucceeded((string) $pendingRecord->getStripePaymentIntentId());

        $this->activateTenant($trainer);
        self::assertSame($balanceBefore + 10, $this->tokenBalance($trainer, $pat), 'AC-05-6: only after parent approval AND Checkout completion do the tokens land in the parent-trainer balance.');
    }
}
