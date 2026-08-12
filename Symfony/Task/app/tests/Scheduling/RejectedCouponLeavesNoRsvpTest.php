<?php

declare(strict_types=1);

namespace App\Tests\Scheduling;

use App\Billing\Repository\PaymentRecordRepository;
use App\Growth\Exception\InvalidCouponException;
use App\Scheduling\Entity\Event;
use App\Scheduling\Entity\Rsvp;
use App\Scheduling\Repository\RsvpRepository;
use App\Scheduling\Service\RsvpService;
use App\Tests\Support\BillingFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\GrowthFixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * AC-06-23: "Invalid or expired code — no discount applied, no payment
 * attempted."
 *
 * The message was right and the rest was not. The coupon was validated in
 * `completeConfirmationOrPayment()`, which runs AFTER `rsvp()`'s transaction
 * has committed, so a player who mistyped a code was told the code was
 * invalid and left registered anyway — `pending_payment`, no payment record,
 * and a seat held, because that status counts toward capacity.
 *
 * Manual testing found it on an RSVP that had already been canceled and
 * refunded: one failed attempt reactivated the row, and the player appeared
 * booked onto an event they had been given their money back for.
 *
 * "No payment attempted" was already true. These assert the half that was
 * not: no registration either.
 */
final class RejectedCouponLeavesNoRsvpTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;
    use BillingFixtureHelpers;
    use GrowthFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    public function testAnInvalidCodeCreatesNoRegistrationAtAll(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->connectStripe($trainer);

        $event = $this->createEvent($trainer, [
            'title' => 'Rejected Coupon Clinic',
            'usdPricingEnabled' => true,
            'usdPriceMinorUnits' => 4000,
            'tokenPricingEnabled' => false,
        ]);

        $paymentRecordsBefore = $this->paymentRecordCount();

        /** @var RsvpService $rsvps */
        $rsvps = self::getContainer()->get(RsvpService::class);

        try {
            $rsvps->rsvp($event, $this->patPlayer(), $this->account('player@practiceperfect.test'), Event::PAYMENT_USD, 'NO-SUCH-CODE');
            self::fail('A code the platform does not accept must be refused.');
        } catch (InvalidCouponException) {
            // AC-06-23's own message path — asserted over HTTP below.
        }

        $this->activateTenant($trainer);

        /** @var RsvpRepository $rsvpRepository */
        $rsvpRepository = self::getContainer()->get(RsvpRepository::class);
        self::assertNull(
            $rsvpRepository->findOneByEventAndPlayer($event, $this->patPlayer()),
            'A rejected coupon must leave no RSVP row — not even a canceled one to reactivate later.',
        );
        self::assertSame($paymentRecordsBefore, $this->paymentRecordCount(), 'AC-06-23: no payment attempted.');
    }

    /**
     * The case manual testing actually hit, and the one that reads worst to a
     * customer: the row already existed, canceled, and a failed attempt
     * brought it back.
     */
    public function testAnInvalidCodeDoesNotResurrectACanceledRegistration(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->connectStripe($trainer);

        $event = $this->createEvent($trainer, [
            'title' => 'Resurrection Clinic',
            'usdPricingEnabled' => true,
            'usdPriceMinorUnits' => 4000,
            'tokenPricingEnabled' => false,
        ]);

        $player = $this->patPlayer();
        $rsvp = $this->createRsvp($event, $player, Rsvp::METHOD_USD, Rsvp::STATUS_CANCELED);
        $rsvpId = $rsvp->getId();

        /** @var RsvpService $rsvps */
        $rsvps = self::getContainer()->get(RsvpService::class);

        try {
            $rsvps->rsvp($event, $player, $this->account('player@practiceperfect.test'), Event::PAYMENT_USD, 'NO-SUCH-CODE');
            self::fail('Expected the code to be refused.');
        } catch (InvalidCouponException) {
        }

        $this->activateTenant($trainer);

        /** @var RsvpRepository $rsvpRepository */
        $rsvpRepository = self::getContainer()->get(RsvpRepository::class);
        $reloaded = $rsvpRepository->find($rsvpId);

        self::assertNotNull($reloaded);
        self::assertSame(
            Rsvp::STATUS_CANCELED,
            $reloaded->getStatus(),
            'A refused code must not reactivate a registration the player had canceled.',
        );
    }

    /**
     * And the seat, which is what makes this more than cosmetic: a held
     * place on a nearly-full event is a place somebody else cannot take.
     */
    public function testARejectedCouponHoldsNoSeat(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->connectStripe($trainer);

        $event = $this->createEvent($trainer, [
            'title' => 'Seat Holding Clinic',
            'capacity' => 1,
            'usdPricingEnabled' => true,
            'usdPriceMinorUnits' => 4000,
            'tokenPricingEnabled' => false,
        ]);

        /** @var RsvpService $rsvps */
        $rsvps = self::getContainer()->get(RsvpService::class);

        try {
            $rsvps->rsvp($event, $this->patPlayer(), $this->account('player@practiceperfect.test'), Event::PAYMENT_USD, 'NO-SUCH-CODE');
            self::fail('Expected the code to be refused.');
        } catch (InvalidCouponException) {
        }

        $this->activateTenant($trainer);

        /** @var RsvpRepository $rsvpRepository */
        $rsvpRepository = self::getContainer()->get(RsvpRepository::class);
        self::assertSame(
            0,
            $rsvpRepository->countHeld($event),
            'The single seat on this event must still be available to somebody who can pay for it.',
        );
    }

    /**
     * Over HTTP, which is where the player meets it: the message AC-06-23
     * names, and a page that does not claim they are registered.
     */
    public function testThePlayerSeesTheMessageAndIsNotShownAsRegistered(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->connectStripe($trainer);

        $event = $this->createEvent($trainer, [
            'title' => 'Typo Clinic',
            'usdPricingEnabled' => true,
            'usdPriceMinorUnits' => 4000,
            'tokenPricingEnabled' => false,
        ]);

        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);

        $crawler = $this->client->request('GET', sprintf('/portal/events/%d', $event->getId()));
        $form = $crawler->selectButton('Register & Pay')->form([
            'rsvp[paymentMethod]' => Event::PAYMENT_USD,
            'rsvp[couponCode]' => 'NO-SUCH-CODE',
        ]);
        $this->client->submit($form);

        $crawler = $this->client->followRedirect();

        self::assertSelectorTextContains('body', 'Invalid or expired code');
        self::assertStringNotContainsString(
            'Already registered',
            $crawler->filter('main')->text(),
            'The page must not tell a player they are registered after refusing their request.',
        );
    }

    private function paymentRecordCount(): int
    {
        /** @var PaymentRecordRepository $paymentRecords */
        $paymentRecords = self::getContainer()->get(PaymentRecordRepository::class);

        return \count($paymentRecords->findAll());
    }
}
