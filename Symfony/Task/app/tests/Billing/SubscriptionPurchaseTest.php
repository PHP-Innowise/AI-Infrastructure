<?php

declare(strict_types=1);

namespace App\Tests\Billing;

use App\Billing\Repository\PaymentRecordRepository;
use App\Billing\Repository\SubscriptionEntitlementRepository;
use App\Billing\Repository\TrainerBillingSettingsRepository;
use App\Platform\Entity\Trainer;
use App\Tests\Support\BillingFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use App\Tests\Support\WebhookDeliveryHelper;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * AC-05-29, BR-05-14 — Player Subscriptions ("Unlimited Access").
 *
 * All three tests deliberately run against 'peak-performance', in
 * declaration order, each depending on the state the previous one left —
 * documented here rather than left implicit. The fixture parent "Pat"
 * (player@practiceperfect.test) has no real membership with
 * 'baseline-athletics' (only his child "Blake" does), so switching him
 * there is a silent no-op that leaves him on whatever trainer context he
 * already had — there is no genuinely separate "second trainer" to use for
 * isolation the way other Billing tests use 'baseline-athletics'. BR-05-14
 * also scopes an entitlement to (trainer, PARENT), not per child, so even
 * switching to Pat's child "Alex" would not produce an independent payer.
 * Given that constraint, this suite embraces the shared state deliberately:
 * test 1 proves "not enabled" BEFORE subscriptions are enabled for this
 * trainer at all; test 2 enables them and completes a real purchase,
 * leaving a genuine, webhook-confirmed entitlement in place; test 3 proves
 * the "already exists" guard against exactly that entitlement, rather than
 * constructing its own (which would only re-collide with test 2's).
 */
final class SubscriptionPurchaseTest extends WebTestCase
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
     * AC-05-29: a trainer who has not enabled Player Subscriptions (no
     * price set) cannot be purchased from. Must run before the next test
     * enables subscriptions for this same trainer — see this class's own
     * docblock.
     */
    public function testCannotPurchaseWhenTheTrainerHasNotEnabledSubscriptions(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $pat = $this->account('player@practiceperfect.test');

        $this->client->loginUser($pat);
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', '/portal/subscription/purchase');
        $form = $crawler->selectButton('Subscribe')->form([
            'subscription_purchase[activationDate]' => (new \DateTimeImmutable('+5 days'))->format('Y-m-d'),
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/portal/subscription/purchase');
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', 'not enabled Player Subscriptions');
    }

    /**
     * AC-05-29: implemented as an entitlement grant, never a Stripe
     * Subscription — the trainer sets the price, the player picks a
     * future activation date, pays via Stripe, and the entitlement is
     * granted only once the webhook confirms payment ("no grace period"),
     * covering 30 days from that activation date. Leaves a real,
     * webhook-confirmed entitlement in place for the next test — see this
     * class's own docblock.
     */
    public function testPlayerPurchasesAnUnlimitedAccessSubscriptionActivatingOnAFutureDate(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->enableSubscriptions($trainer, 1500);
        $pat = $this->account('player@practiceperfect.test');

        $activationDate = new \DateTimeImmutable('+10 days');

        $this->client->loginUser($pat);
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', '/portal/subscription/purchase');
        $form = $crawler->selectButton('Subscribe')->form([
            'subscription_purchase[activationDate]' => $activationDate->format('Y-m-d'),
        ]);
        $this->client->submit($form);

        self::assertTrue($this->client->getResponse()->isRedirect(), 'AC-05-29: redirected to Stripe Checkout.');
        self::assertStringContainsString('checkout.stripe.test', (string) $this->client->getResponse()->headers->get('Location'));

        $this->activateTenant($trainer);
        /** @var SubscriptionEntitlementRepository $entitlements */
        $entitlements = self::getContainer()->get(SubscriptionEntitlementRepository::class);
        self::assertEmpty($entitlements->findNotYetExpired($trainer, $pat, new \DateTimeImmutable('today')), 'AC-05-29: no grace period — nothing granted before payment succeeds.');

        /** @var PaymentRecordRepository $paymentRecords */
        $paymentRecords = self::getContainer()->get(PaymentRecordRepository::class);
        $pending = current(array_filter(
            $paymentRecords->findForPayerHistory($pat),
            static fn ($p) => 'player_subscription' === $p->getType() && $p->isPending(),
        ));
        self::assertNotFalse($pending);

        $this->deliverPaymentIntentSucceeded((string) $pending->getStripePaymentIntentId(), [
            'metadata' => ['activation_date' => $activationDate->format('Y-m-d')],
        ]);

        $this->activateTenant($trainer);
        $granted = $entitlements->findNotYetExpired($trainer, $pat, $activationDate);
        self::assertCount(1, $granted, 'AC-05-29: the entitlement is granted once the webhook confirms payment.');
        self::assertSame($activationDate->format('Y-m-d'), $granted[0]->getActivationDate()->format('Y-m-d'), 'AC-05-29: activates on the player-chosen future date.');
        self::assertSame(
            $activationDate->modify('+30 days')->format('Y-m-d'),
            $granted[0]->getWindowEndsOn()->format('Y-m-d'),
            'BR-05-14: 30-day window from activation.',
        );

        $subjects = array_map(static fn ($m) => method_exists($m, 'getSubject') ? $m->getSubject() : '', self::getMailerMessages());
        self::assertTrue((bool) array_filter($subjects, static fn (string $s): bool => str_contains($s, 'ubscri')), 'A confirmation is sent.');
    }

    /**
     * BR-05-14: a player/parent who already holds a not-yet-expired
     * entitlement for this trainer cannot buy a second, overlapping one —
     * proven against the real entitlement the previous test's own
     * successful purchase left in place (see this class's own docblock for
     * why this is deliberate, not accidental, ordering).
     */
    public function testCannotPurchaseASecondOverlappingSubscription(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $pat = $this->account('player@practiceperfect.test');

        /** @var SubscriptionEntitlementRepository $entitlements */
        $entitlements = self::getContainer()->get(SubscriptionEntitlementRepository::class);
        self::assertNotEmpty(
            $entitlements->findNotYetExpired($trainer, $pat, new \DateTimeImmutable('today')),
            'Precondition: the previous test left a real, not-yet-expired entitlement in place.',
        );

        $this->client->loginUser($pat);
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', '/portal/subscription/purchase');
        $form = $crawler->selectButton('Subscribe')->form(['subscription_purchase[activationDate]' => (new \DateTimeImmutable('+3 days'))->format('Y-m-d')]);
        $this->client->submit($form);

        self::assertResponseRedirects('/portal/subscription/purchase');
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', 'already exists', 'BR-05-14: refused — an active or pending subscription already exists.');
    }

    private function enableSubscriptions(Trainer $trainer, int $priceMinorUnits): void
    {
        /** @var TrainerBillingSettingsRepository $settingsRepo */
        $settingsRepo = self::getContainer()->get(TrainerBillingSettingsRepository::class);
        $settings = $settingsRepo->getOrCreateForTrainer($trainer);
        $settings->updatePlayerSubscriptionPrice($priceMinorUnits);
        self::getContainer()->get(EntityManagerInterface::class)->flush();
    }
}
