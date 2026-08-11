<?php

declare(strict_types=1);

namespace App\Tests\Billing;

use App\Billing\Repository\StripeCustomerLinkRepository;
use App\Billing\Service\StripeGateway;
use App\Tests\Support\BillingFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-05.07 — Parent Manages Payment Methods.
 */
final class PaymentMethodTest extends WebTestCase
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
     * AC-05-20: "Manage Payment Methods" redirects to the Stripe Customer
     * Portal.
     */
    public function testManagePaymentMethodsRedirectsToStripeCustomerPortal(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $pat = $this->account('player@practiceperfect.test');

        $this->client->loginUser($pat);
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', '/portal/tokens');
        $form = $crawler->selectButton('Manage Payment Methods')->form();
        $this->client->submit($form);

        self::assertTrue($this->client->getResponse()->isRedirect(), 'AC-05-20: redirected to the Stripe Customer Portal.');
        self::assertStringContainsString('billing.stripe.test', (string) $this->client->getResponse()->headers->get('Location'));
    }

    /**
     * AC-01-30/AC-05-21: a child's own login is denied outright — payment
     * methods are a whole-family, parent-only concern.
     */
    public function testAChildsOwnLoginCannotManagePaymentMethods(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $alex = $this->alexPlayer();
        $this->ensureActivePlayerMembership($trainer, $alex);
        $childAccount = $this->giveChildOwnLogin($alex, 'alex-payment-method-login@practiceperfect.test', $trainer);

        // The voter is checked before the CSRF token
        // (PortalPaymentMethodController::manage()'s own ordering), so a
        // 403 here is the ACCESS denial this test targets, not a CSRF
        // rejection racing to be first.
        $this->client->loginUser($childAccount);
        $this->client->request('POST', '/portal/billing/payment-methods/manage');

        self::assertResponseStatusCodeSame(403, 'AC-01-30/AC-05-21: a child\'s own login is denied outright.');
    }

    /**
     * AC-05-12/21: exactly one Stripe Customer per account — a SECOND,
     * unrelated purchase (a token purchase, after an earlier RSVP payment)
     * reuses the SAME Stripe Customer id rather than creating a new one,
     * which is the platform-side mechanic behind Stripe itself then being
     * able to show "Use card ending in [last 4]" on the later purchase.
     */
    public function testTheSameStripeCustomerIsReusedAcrossUnrelatedPurchases(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $pat = $this->account('player@practiceperfect.test');

        /** @var StripeGateway $gateway */
        $gateway = self::getContainer()->get(StripeGateway::class);
        $firstCustomerId = $gateway->customerIdFor($pat);
        $secondCustomerId = $gateway->customerIdFor($pat);

        self::assertSame($firstCustomerId, $secondCustomerId, 'AC-05-12/21: the same Stripe Customer id every time.');

        /** @var StripeCustomerLinkRepository $links */
        $links = self::getContainer()->get(StripeCustomerLinkRepository::class);
        $link = $links->findForAccount($pat);
        self::assertNotNull($link);
        self::assertSame($firstCustomerId, $link->getStripeCustomerId());
    }

    /**
     * AC-05-21/BR-05-16: the same Stripe Customer — and so the same saved
     * cards — is shared across every one of the parent's trainers, not
     * reset per trainer context.
     */
    public function testTheSameStripeCustomerIsSharedAcrossDifferentTrainers(): void
    {
        $trainerA = $this->trainer('peak-performance');
        $trainerB = $this->trainer('baseline-athletics');
        $pat = $this->account('player@practiceperfect.test');

        /** @var StripeGateway $gateway */
        $gateway = self::getContainer()->get(StripeGateway::class);

        $this->activateTenant($trainerA);
        $customerIdUnderTrainerA = $gateway->customerIdFor($pat);

        $this->activateTenant($trainerB);
        $customerIdUnderTrainerB = $gateway->customerIdFor($pat);

        self::assertSame($customerIdUnderTrainerA, $customerIdUnderTrainerB, 'AC-05-21/BR-05-16: shared across trainers — not per-tenant.');
    }
}
