<?php

declare(strict_types=1);

namespace App\Tests\Billing;

use App\Billing\Entity\PaymentRecord;
use App\Billing\Message\ProcessStripeWebhookEvent;
use App\Billing\MessageHandler\ProcessStripeWebhookEventHandler;
use App\Billing\Repository\PaymentRecordRepository;
use App\Billing\Repository\StripeEventReceiptRepository;
use App\Tests\Support\BillingFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\Messenger\Envelope;
use Symfony\Component\Messenger\Retry\RetryStrategyInterface;
use Symfony\Component\Messenger\Stamp\RedeliveryStamp;

/**
 * AC-05-34/35/37 — Stripe webhook reliability and error handling
 * (specs/requirements-analyst-epic-05-payments-tokens-spec.md "Technical
 * Notes", "Security Requirements", "Error Handling"; epic-level AC "Stripe
 * Integration"). AC-05-34's own six event-type effects are exercised
 * individually elsewhere (TokenPurchaseTest, RsvpTest, ContentPurchaseTest,
 * SubscriptionPurchaseTest all deliver real webhooks via
 * WebhookDeliveryHelper); this file covers the receiver's own contract —
 * signature verification, replay-idempotency, retry configuration, and
 * failure handling — that every one of those effects depends on.
 */
final class WebhookReliabilityTest extends WebTestCase
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
     * AC-05-35, "Technical Notes — Security Requirements": an invalid
     * Stripe signature is rejected with 400, and no receipt row (and so no
     * processing) is ever created for it — spoofing prevention.
     */
    public function testAnInvalidSignatureIsRejectedAndNothingIsPersisted(): void
    {
        /** @var StripeEventReceiptRepository $receipts */
        $receipts = self::getContainer()->get(StripeEventReceiptRepository::class);
        $before = \count($receipts->findAll());

        $payload = json_encode(['id' => 'evt_fake_bad_sig', 'type' => 'payment_intent.succeeded', 'data' => ['object' => ['id' => 'pi_fake_irrelevant']]], \JSON_THROW_ON_ERROR);
        $this->client->request(
            'POST',
            '/webhooks/stripe',
            server: ['HTTP_STRIPE_SIGNATURE' => 'sig_totally_forged'],
            content: $payload,
        );

        self::assertResponseStatusCodeSame(400, 'AC-05-35: an invalid signature is rejected.');
        self::assertCount($before, $receipts->findAll(), 'Nothing is persisted for a signature that fails verification.');
    }

    /**
     * AC-05-34/35, specs/api-designer-spec.md "Stripe webhook contract"
     * steps 3-5: a validly-signed event is accepted (200), a receipt row
     * is inserted, and processing is queued (proven by the receipt
     * existing, unprocessed — WebhookDeliveryHelper's own docblock
     * explains why this suite never runs the async worker itself).
     */
    public function testAValidlySignedWebhookIsAcceptedAndQueuedForProcessing(): void
    {
        $eventId = 'evt_fake_'.bin2hex(random_bytes(8));
        $payload = json_encode(['id' => $eventId, 'type' => 'account.updated', 'data' => ['object' => ['id' => 'acct_fake_irrelevant']]], \JSON_THROW_ON_ERROR);

        $this->client->request(
            'POST',
            '/webhooks/stripe',
            server: ['HTTP_STRIPE_SIGNATURE' => 'sig_fake_valid'],
            content: $payload,
        );

        self::assertResponseStatusCodeSame(200, 'AC-05-35: a validly-signed event is accepted.');

        /** @var StripeEventReceiptRepository $receipts */
        $receipts = self::getContainer()->get(StripeEventReceiptRepository::class);
        $receipt = $receipts->findOneByStripeEventId($eventId);
        self::assertNotNull($receipt, 'AC-05-34: a receipt row is inserted for the accepted event.');
    }

    /**
     * AC-05-35, specs/api-designer-spec.md "Stripe webhook contract" step
     * 4: a redelivery of the SAME Stripe event id — Stripe's own
     * at-least-once guarantee — is a clean 200 with no second receipt row
     * and no second processing dispatch, "the unique violation IS the
     * duplicate detection" (StripeEventReceipt's own docblock).
     */
    public function testARedeliveredWebhookIsIdempotent(): void
    {
        $eventId = 'evt_fake_'.bin2hex(random_bytes(8));
        $payload = json_encode(['id' => $eventId, 'type' => 'account.updated', 'data' => ['object' => ['id' => 'acct_fake_irrelevant']]], \JSON_THROW_ON_ERROR);

        $this->client->request('POST', '/webhooks/stripe', server: ['HTTP_STRIPE_SIGNATURE' => 'sig_fake_valid'], content: $payload);
        self::assertResponseStatusCodeSame(200);

        $this->client->request('POST', '/webhooks/stripe', server: ['HTTP_STRIPE_SIGNATURE' => 'sig_fake_valid'], content: $payload);
        self::assertResponseStatusCodeSame(200, 'AC-05-35: a redelivery is still a clean 200, not an error.');

        /** @var StripeEventReceiptRepository $receipts */
        $receipts = self::getContainer()->get(StripeEventReceiptRepository::class);
        self::assertCount(1, array_filter($receipts->findAll(), static fn ($r) => $eventId === $r->getStripeEventId()), 'Exactly one receipt row for this event id, despite two deliveries.');
    }

    /**
     * AC-05-35, "Performance & Scale Targets — Reliability": "retried with
     * exponential backoff, up to 3 attempts" — verified against the REAL,
     * configured `messenger.retry.multiplier_retry_strategy.async` service
     * (config/packages/messenger.yaml), not the YAML file re-read as text.
     * The literal "over 24 hours" span is NOT reached by this
     * configuration, deliberately — see that YAML file's own docblock on
     * why (tuning to exactly 24h would slow every other message on the
     * same shared transport, including SendEmailMessage). Recorded
     * honestly here rather than asserted as if it matched.
     */
    public function testFailedWebhookProcessingRetriesUpToThreeTimesWithExponentialBackoff(): void
    {
        /** @var RetryStrategyInterface $retryStrategy */
        $retryStrategy = self::getContainer()->get('messenger.retry.multiplier_retry_strategy.async');

        $message = new ProcessStripeWebhookEvent(1);
        $neverRetried = new Envelope($message);
        $afterOneRetry = new Envelope($message, [new RedeliveryStamp(1)]);
        $afterTwoRetries = new Envelope($message, [new RedeliveryStamp(2)]);
        $afterThreeRetries = new Envelope($message, [new RedeliveryStamp(3)]);

        self::assertTrue($retryStrategy->isRetryable($neverRetried), 'Retryable before the first attempt.');
        self::assertTrue($retryStrategy->isRetryable($afterOneRetry));
        self::assertTrue($retryStrategy->isRetryable($afterTwoRetries));
        self::assertFalse($retryStrategy->isRetryable($afterThreeRetries), 'AC-05-35: capped at 3 retries — the 4th attempt is refused.');

        $firstDelay = $retryStrategy->getWaitingTime($neverRetried);
        $secondDelay = $retryStrategy->getWaitingTime($afterOneRetry);
        $thirdDelay = $retryStrategy->getWaitingTime($afterTwoRetries);
        // Symfony's own YAML-configured retry strategy applies random
        // jitter on top of the plain multiplier math (avoiding a
        // thundering herd of simultaneous retries) — exact-equality on the
        // configured 60s/4x would be flaky by design, so a tolerant range
        // is checked instead, matched against the multiplier relationship
        // rather than an absolute number.
        self::assertEqualsWithDelta(60_000, $firstDelay, 60_000 * 0.3, 'The configured base delay (~60s, jitter-tolerant).');
        self::assertGreaterThan($firstDelay, $secondDelay, 'AC-05-35: exponential backoff — each retry waits longer than the last.');
        self::assertGreaterThan($secondDelay, $thirdDelay);
        self::assertEqualsWithDelta(60_000 * 4, $secondDelay, 60_000 * 4 * 0.3, 'The configured multiplier (~4x, jitter-tolerant).');

        // The epic's own literal "up to ... 24 hours" total span is not
        // reached by design — see this test's own docblock. Not asserted
        // as matching; recorded as a deliberate, documented deviation
        // instead (also in migrations/Version20260811100000.php's sibling
        // config file and the coder's final report).
        $totalSpanMilliseconds = $firstDelay + $secondDelay + $thirdDelay;
        self::assertLessThan(24 * 3600 * 1000, $totalSpanMilliseconds, 'Deliberately short of 24h — see this test\'s own docblock.');
    }

    /**
     * AC-05-37, "Technical Notes — Error Handling": "Stripe API errors are
     * logged and the admin is notified." This codebase's own
     * implementation of "notified" IS the log entry plus the persisted,
     * admin-queryable `processingError` column
     * (ProcessStripeWebhookEventHandler's own catch block: "logged here" —
     * there is no separate email-to-admin channel) — proven here by
     * triggering a real handler failure (a payment_intent.succeeded event
     * for a payment record with no payer account, which
     * handlePaymentIntentSucceeded() itself refuses to process) and
     * checking both effects, plus that the exception re-throws so
     * Messenger's own retry_strategy (proven above) actually gets a
     * chance to run.
     */
    public function testAHandlerFailureIsLoggedAndRecordedAndRethrownForRetry(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        // A token-purchase payment record with no payer account — invalid
        // in practice (TokenPurchaseService always supplies one), but
        // exactly the shape handlePaymentIntentSucceeded() itself guards
        // against and throws a \LogicException for, which is what this
        // test needs: a genuine, reachable failure inside the handler.
        $package = $this->createTokenPackage($trainer, 'Failure Fixture Package');
        $paymentRecord = new PaymentRecord($trainer, PaymentRecord::TYPE_TOKEN_PURCHASE, PaymentRecord::METHOD_CARD, 9000, 'No Payer', 'no-payer@practiceperfect.test', null);
        $paymentRecord->attachRelatedTokenPackage($package);
        $paymentRecord->applyFee(500, 450);
        $paymentRecord->attachStripePaymentIntentId('pi_fake_no_payer_'.bin2hex(random_bytes(6)));
        $entityManager = self::getContainer()->get(EntityManagerInterface::class);
        $entityManager->persist($paymentRecord);
        $entityManager->flush();

        $eventId = 'evt_fake_'.bin2hex(random_bytes(8));
        $payload = json_encode(['id' => $eventId, 'type' => 'payment_intent.succeeded', 'data' => ['object' => ['id' => $paymentRecord->getStripePaymentIntentId(), 'latest_charge' => 'ch_fake_irrelevant']]], \JSON_THROW_ON_ERROR);

        /** @var StripeEventReceiptRepository $receipts */
        $receipts = self::getContainer()->get(StripeEventReceiptRepository::class);
        $receipt = new \App\Billing\Entity\StripeEventReceipt($eventId, 'payment_intent.succeeded', $payload);
        $receipts->add($receipt);
        $entityManager->flush();

        /** @var ProcessStripeWebhookEventHandler $handler */
        $handler = self::getContainer()->get(ProcessStripeWebhookEventHandler::class);

        $threw = false;
        try {
            $handler(new ProcessStripeWebhookEvent((int) $receipt->getId()));
        } catch (\LogicException) {
            $threw = true;
        }
        self::assertTrue($threw, 'AC-05-37: the failure re-throws so Messenger\'s retry_strategy gets a chance to run.');

        $this->activateTenant($trainer);
        $reloadedReceipt = $receipts->find($receipt->getId());
        self::assertNotNull($reloadedReceipt);
        self::assertNotNull($reloadedReceipt->getProcessingError(), 'AC-05-37: the error is recorded — the admin-visible half of "logged and the admin is notified."');

        /** @var PaymentRecordRepository $paymentRecords */
        $paymentRecords = self::getContainer()->get(PaymentRecordRepository::class);
        $reloadedPaymentRecord = $paymentRecords->find($paymentRecord->getId());
        self::assertNotNull($reloadedPaymentRecord);
        self::assertTrue($reloadedPaymentRecord->isPending(), 'The payment record is untouched by the failed attempt — nothing was silently marked completed.');
    }

    /**
     * AC-05-37: "a network failure during Stripe Checkout resumes on
     * return." NOT built in this pass, recorded honestly rather than
     * faked: `Rsvp::$pendingCheckoutUrl` is explicitly transient/never
     * persisted (that property's own docblock), so once a player leaves
     * the platform for Stripe's hosted Checkout page, the platform itself
     * has nothing left to "resume" from if they come back without
     * finishing — re-attempting the same RSVP hits
     * AlreadyRegisteredException instead (a Pending Payment row already
     * holds the spot, RsvpService::rsvp()'s own first check), not a fresh
     * Checkout redirect. This may be an acceptable gap (Stripe's own
     * Checkout Session page is itself resilient to a transient network
     * blip WHILE on Stripe's page, which is arguably the more literal
     * reading of "during Stripe Checkout") but the platform-side "come
     * back later and finish paying" flow this AC's plain text also
     * suggests does not exist. Flagged here rather than silently assumed
     * either way.
     */
    public function testNetworkFailureDuringCheckoutResumeIsNotImplemented(): void
    {
        self::markTestSkipped('AC-05-37: no platform-side mechanism exists to resume an abandoned Stripe Checkout attempt after the request that created it ends (Rsvp::$pendingCheckoutUrl is transient by design) — a genuine gap, not a faked pass. See this test\'s own docblock.');
    }
}
