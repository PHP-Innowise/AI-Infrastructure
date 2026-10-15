<?php

declare(strict_types=1);

namespace App\Tests\Growth;

use App\Content\Repository\PlaylistAccessGrantRepository;
use App\Growth\Entity\Coupon;
use App\Growth\Repository\CouponRedemptionRepository;
use App\Growth\Repository\CouponRepository;
use App\Growth\Service\CouponPricingService;
use App\Scheduling\Repository\RsvpRepository;
use App\Billing\Repository\PaymentRecordRepository;
use App\Tests\Support\BillingFixtureHelpers;
use App\Tests\Support\ContentFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\GrowthFixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use App\Tests\Support\WebhookDeliveryHelper;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-06.06 — Player Uses Coupon Code.
 */
final class CouponCheckoutTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;
    use ContentFixtureHelpers;
    use BillingFixtureHelpers;
    use GrowthFixtureHelpers;
    use WebhookDeliveryHelper;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-06-20: the RSVP payment screen carries a "Have a coupon code?"
     * field.
     */
    public function testEventCheckoutShowsTheCouponPrompt(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, ['usdPricingEnabled' => true, 'usdPriceMinorUnits' => 2000]);

        $pat = $this->account('player@practiceperfect.test');
        $this->client->loginUser($pat);
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', sprintf('/portal/events/%d', $event->getId()));

        self::assertResponseIsSuccessful();
        self::assertSelectorExists('input[name="rsvp[couponCode]"]', 'AC-06-20: "Have a coupon code?" field is present.');
    }

    /**
     * AC-06-21/22/25, BR-06-9: a valid coupon discounts the RSVP's card
     * charge — Stripe receives only the final amount — and once the
     * payment succeeds, the use is logged and the coupon's usage count
     * increments.
     */
    public function testValidCouponDiscountsAnEventRsvpAndRecordsTheRedemption(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, ['usdPricingEnabled' => true, 'usdPriceMinorUnits' => 2000]);
        $coupon = $this->createCoupon($trainer, 'SUMMER20', $this->account('trainer@practiceperfect.test'), [
            'discountType' => Coupon::DISCOUNT_PERCENTAGE,
            'discountValue' => 20,
            'appliesTo' => Coupon::APPLIES_TO_EVENTS,
        ]);

        $pat = $this->account('player@practiceperfect.test');
        $this->client->loginUser($pat);
        $this->switchPlayerToTrainer($this->client, $trainer);

        $crawler = $this->client->request('GET', sprintf('/portal/events/%d', $event->getId()));
        $form = $crawler->selectButton('Register & Pay')->form([
            'rsvp[paymentMethod]' => 'usd',
            'rsvp[couponCode]' => 'SUMMER20',
        ]);
        $this->client->submit($form);

        self::assertTrue($this->client->getResponse()->isRedirect(), 'AC-06-22: redirected to Stripe Checkout.');
        self::assertStringContainsString('checkout.stripe.test', (string) $this->client->getResponse()->headers->get('Location'));

        // AC-06-22: Stripe receives only the final, already-discounted
        // amount — $20 - 20% = $16.
        self::assertNotNull($this->fakeStripeClient()->lastCheckoutSessionRequest);
        self::assertSame(1600, $this->fakeStripeClient()->lastCheckoutSessionRequest->amountMinorUnits, 'AC-06-22: Stripe receives only the final discounted amount.');

        $this->activateTenant($trainer);
        /** @var RsvpRepository $rsvps */
        $rsvps = self::getContainer()->get(RsvpRepository::class);
        $rsvp = $rsvps->findOneByEventAndPlayer($event, $this->patPlayer());
        self::assertNotNull($rsvp);

        /** @var PaymentRecordRepository $paymentRecords */
        $paymentRecords = self::getContainer()->get(PaymentRecordRepository::class);
        $pending = $paymentRecords->findOneChargeByRsvp($rsvp);
        self::assertNotNull($pending);
        self::assertSame(1600, $pending->getAmountMinorUnits());

        // The webhook confirms the card payment — a real Stripe webhook
        // echoes back whatever Checkout Session metadata was set
        // (`SchedulingPaymentIntentGateway::startCardCheckout()`'s own
        // 'coupon_code' entry), which `InMemoryStripeClient`'s fake
        // checkout session does not itself simulate propagating — passed
        // explicitly here, matching
        // `SubscriptionPurchaseTest`'s own established pattern for the
        // exact same gap.
        $this->deliverPaymentIntentSucceeded((string) $pending->getStripePaymentIntentId(), [
            'metadata' => ['coupon_code' => 'SUMMER20'],
        ]);

        $this->activateTenant($trainer);
        $confirmedRsvp = $rsvps->findOneByEventAndPlayer($event, $this->patPlayer());
        self::assertNotNull($confirmedRsvp);
        self::assertTrue($confirmedRsvp->isConfirmed(), 'The RSVP confirms once the webhook reports success.');

        // AC-06-25: the use is logged and the coupon's usage count
        // increments.
        /** @var CouponRedemptionRepository $redemptions */
        $redemptions = self::getContainer()->get(CouponRedemptionRepository::class);
        $rows = $redemptions->findAllForCoupon($coupon);
        self::assertCount(1, $rows, 'AC-06-25: exactly one redemption logged.');
        self::assertSame(2000, $rows[0]->getOriginalPriceMinorUnits());
        self::assertSame(400, $rows[0]->getDiscountAmountMinorUnits());
        self::assertSame(1600, $rows[0]->getFinalPriceMinorUnits());
        self::assertSame($this->patPlayer()->getId(), $rows[0]->getPlayer()->getId());

        /** @var CouponRepository $coupons */
        $coupons = self::getContainer()->get(CouponRepository::class);
        $reloadedCoupon = $coupons->findOneByTrainerAndCode($trainer, 'SUMMER20');
        self::assertNotNull($reloadedCoupon);
        self::assertSame(1, $reloadedCoupon->getUsageCount(), 'AC-06-25: usage count incremented.');
    }

    /**
     * AC-06-23: "Invalid or expired code" — no discount applied, no
     * payment attempted, when the code does not exist for this trainer.
     */
    public function testInvalidCouponCodeBlocksPaymentWithAnError(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, ['usdPricingEnabled' => true, 'usdPriceMinorUnits' => 2000]);

        $pat = $this->account('player@practiceperfect.test');
        $this->client->loginUser($pat);
        $this->switchPlayerToTrainer($this->client, $trainer);

        $crawler = $this->client->request('GET', sprintf('/portal/events/%d', $event->getId()));
        $form = $crawler->selectButton('Register & Pay')->form([
            'rsvp[paymentMethod]' => 'usd',
            'rsvp[couponCode]' => 'DOES-NOT-EXIST',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects(sprintf('/portal/events/%d', $event->getId()));
        self::assertNull($this->fakeStripeClient()->lastCheckoutSessionRequest, 'AC-06-23: no payment was ever attempted.');

        $this->client->followRedirect();
        self::assertSelectorTextContains('body', 'Invalid or expired code', 'AC-06-23: the exact error message.');
    }

    /**
     * AC-06-22, US-06.06: a coupon applies to a CONTENT purchase too
     * ("applies to Events, Content, or Both").
     */
    public function testValidCouponDiscountsAContentPurchase(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createLearnPlaylist($trainer, ['title' => 'Coupon-eligible Playlist']);
        $playlist->updatePricing(5000, 5);
        $this->flushGrowth();
        $this->createCoupon($trainer, 'CONTENT10', $this->account('trainer@practiceperfect.test'), [
            'discountType' => Coupon::DISCOUNT_FIXED,
            'discountValue' => 1000,
            'appliesTo' => Coupon::APPLIES_TO_CONTENT,
        ]);

        $pat = $this->account('player@practiceperfect.test');
        $this->client->loginUser($pat);
        $this->switchPlayerToTrainer($this->client, $trainer);

        $crawler = $this->client->request('GET', sprintf('/portal/content/playlists/%d/checkout', $playlist->getId()));
        $form = $crawler->selectButton('Purchase')->form([
            'purchase_method[method]' => 'usd',
            'purchase_method[couponCode]' => 'CONTENT10',
        ]);
        $this->client->submit($form);

        self::assertTrue($this->client->getResponse()->isRedirect());
        self::assertSame(4000, $this->fakeStripeClient()->lastCheckoutSessionRequest?->amountMinorUnits, 'BR-06-9: $50 - $10 fixed = $40.');

        $this->activateTenant($trainer);
        /** @var PlaylistAccessGrantRepository $grants */
        $grants = self::getContainer()->get(PlaylistAccessGrantRepository::class);
        self::assertNull($grants->findOneByPlaylistAndPlayer($playlist, $this->patPlayer()), 'Still locked until the webhook confirms.');

        /** @var PaymentRecordRepository $paymentRecords */
        $paymentRecords = self::getContainer()->get(PaymentRecordRepository::class);
        $pending = $paymentRecords->findOneChargeByPlaylistAndPlayerAccount($playlist, $pat);
        self::assertNotNull($pending);

        // Content's own subscriber needs BOTH 'player_id' (its only
        // beneficiary-player linkage — see PaymentOutcomeSubscriber's own
        // docblock) and 'coupon_code' echoed back — see this test class's
        // RSVP-flow test for why these are passed explicitly.
        $this->deliverPaymentIntentSucceeded((string) $pending->getStripePaymentIntentId(), [
            'metadata' => ['player_id' => (string) $this->patPlayer()->getId(), 'coupon_code' => 'CONTENT10'],
        ]);

        $this->activateTenant($trainer);
        self::assertNotNull($grants->findOneByPlaylistAndPlayer($playlist, $this->patPlayer()), 'Unlocked once the webhook confirms.');

        /** @var CouponRedemptionRepository $redemptions */
        $redemptions = self::getContainer()->get(CouponRedemptionRepository::class);
        /** @var CouponRepository $coupons */
        $coupons = self::getContainer()->get(CouponRepository::class);
        $coupon = $coupons->findOneByTrainerAndCode($trainer, 'CONTENT10');
        self::assertNotNull($coupon);
        self::assertCount(1, $redemptions->findAllForCoupon($coupon));
    }

    /**
     * AC-06-25: "when the usage limit is reached, the code
     * auto-deactivates" — a coupon with usageLimit: 1 is no longer
     * redeemable after its one use.
     */
    public function testCouponAutoDeactivatesOnceItsUsageLimitIsReached(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, ['usdPricingEnabled' => true, 'usdPriceMinorUnits' => 1000]);
        $this->createCoupon($trainer, 'ONLYONE1', $this->account('trainer@practiceperfect.test'), [
            'discountType' => Coupon::DISCOUNT_PERCENTAGE,
            'discountValue' => 10,
            'appliesTo' => Coupon::APPLIES_TO_EVENTS,
            'usageLimit' => 1,
        ]);

        $pat = $this->account('player@practiceperfect.test');
        $this->client->loginUser($pat);
        $this->switchPlayerToTrainer($this->client, $trainer);

        $crawler = $this->client->request('GET', sprintf('/portal/events/%d', $event->getId()));
        $form = $crawler->selectButton('Register & Pay')->form(['rsvp[paymentMethod]' => 'usd', 'rsvp[couponCode]' => 'ONLYONE1']);
        $this->client->submit($form);

        $this->activateTenant($trainer);
        /** @var RsvpRepository $rsvps */
        $rsvps = self::getContainer()->get(RsvpRepository::class);
        $rsvp = $rsvps->findOneByEventAndPlayer($event, $this->patPlayer());
        self::assertNotNull($rsvp);
        /** @var PaymentRecordRepository $paymentRecords */
        $paymentRecords = self::getContainer()->get(PaymentRecordRepository::class);
        $pending = $paymentRecords->findOneChargeByRsvp($rsvp);
        self::assertNotNull($pending);
        $this->deliverPaymentIntentSucceeded((string) $pending->getStripePaymentIntentId(), [
            'metadata' => ['coupon_code' => 'ONLYONE1'],
        ]);

        $this->activateTenant($trainer);
        /** @var CouponRepository $coupons */
        $coupons = self::getContainer()->get(CouponRepository::class);
        $coupon = $coupons->findOneByTrainerAndCode($trainer, 'ONLYONE1');
        self::assertNotNull($coupon);
        self::assertSame(1, $coupon->getUsageCount());
        self::assertFalse($coupon->isActive(), 'AC-06-25: auto-deactivated once the usage limit is reached.');
        self::assertFalse($coupon->isRedeemable(new \DateTimeImmutable()));
    }

    /**
     * AC-06-24/BR-06-10: only one coupon per transaction. Structurally
     * two-fold: (1) the checkout form carries exactly one `couponCode`
     * field — there is no mechanism to submit a second code alongside it
     * — and (2) `coupon_redemption.payment_record_id` is UNIQUE, so a
     * second redemption attempt against the SAME payment (a webhook
     * redelivery, or any other double-call) is a no-op returning the
     * existing row rather than ever creating a second one.
     */
    public function testOnlyOneCouponRedemptionIsEverRecordedPerTransaction(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $couponA = $this->createCoupon($trainer, 'STACKA', $this->account('trainer@practiceperfect.test'), ['discountValue' => 10]);
        $pat = $this->account('player@practiceperfect.test');
        $payment = $this->createCompletedPaymentRecord($trainer, $pat, 1800);

        /** @var CouponPricingService $couponPricing */
        $couponPricing = self::getContainer()->get(CouponPricingService::class);
        $first = $couponPricing->redeem($couponA, $this->patPlayer(), $payment, 2000, 200, 1800);
        // A second attempt to redeem against the SAME payment (as if a
        // second coupon, or a redelivered event, tried to stack another
        // discount onto the one transaction) — AC-06-24: blocked, not
        // combined; the existing redemption is returned unchanged.
        $second = $couponPricing->redeem($couponA, $this->patPlayer(), $payment, 2000, 200, 1800);

        self::assertSame($first->getId(), $second->getId(), 'AC-06-24: only one redemption is ever recorded per transaction.');

        /** @var CouponRedemptionRepository $redemptions */
        $redemptions = self::getContainer()->get(CouponRedemptionRepository::class);
        self::assertCount(1, $redemptions->findAllForCoupon($couponA));

        /** @var CouponRepository $coupons */
        $coupons = self::getContainer()->get(CouponRepository::class);
        $reloaded = $coupons->findOneByTrainerAndCode($trainer, 'STACKA');
        self::assertNotNull($reloaded);
        self::assertSame(1, $reloaded->getUsageCount(), 'AC-06-24: usage count reflects exactly one redemption, not two.');
    }

    /**
     * AC-06-20: the checkout form itself offers exactly one coupon field —
     * structurally, there is no way to submit two codes on one purchase.
     */
    public function testCheckoutFormOffersExactlyOneCouponField(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, ['usdPricingEnabled' => true, 'usdPriceMinorUnits' => 1500]);

        $pat = $this->account('player@practiceperfect.test');
        $this->client->loginUser($pat);
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', sprintf('/portal/events/%d', $event->getId()));

        self::assertCount(1, $crawler->filter('input[name="rsvp[couponCode]"]'), 'AC-06-24: exactly one coupon code field — no way to submit a second code.');
    }

    private function flushGrowth(): void
    {
        self::getContainer()->get(\Doctrine\ORM\EntityManagerInterface::class)->flush();
    }
}
