<?php

declare(strict_types=1);

namespace App\Tests\Support;

use App\Billing\Entity\StripeEventReceipt;
use App\Billing\Message\ProcessStripeWebhookEvent;
use App\Billing\MessageHandler\ProcessStripeWebhookEventHandler;
use App\Billing\Repository\StripeEventReceiptRepository;
use Doctrine\ORM\EntityManagerInterface;

/**
 * Simulates a Stripe webhook having been received AND processed, for tests
 * whose subject is the EFFECT of a webhook (a token balance credited, an
 * RSVP confirmed, a subscription entitlement granted), not the webhook
 * receiver itself (signature verification, replay-idempotency — that is
 * `WebhookReliabilityTest`'s own job, AC-05-34/35).
 *
 * `App\Billing\Message\ProcessStripeWebhookEvent` is routed to the `async`
 * transport (config/packages/messenger.yaml) with no `sync://` override in
 * the test environment, so a real `$messageBus->dispatch()` would only
 * enqueue a row in `messenger_messages` — nothing in this suite runs a
 * worker to consume it. This helper skips the queue: it inserts the
 * `StripeEventReceipt` row exactly as `WebhookController` would (step 3 of
 * specs/api-designer-spec.md "Stripe webhook contract"), then invokes
 * `ProcessStripeWebhookEventHandler` directly — the same object a real
 * worker would call, with the same single-argument shape (only the
 * receipt's own id), just called in-process instead of through Messenger's
 * own transport plumbing.
 *
 * Requires the including test case to also `use FixtureHelpers` and expose
 * `self::getContainer()`. The tenant must already be active
 * (`activateTenant($trainer)`) is NOT a precondition here — the handler
 * resolves and activates its own tenant per event, exactly as the real
 * async worker would (see that class's own docblock).
 */
trait WebhookDeliveryHelper
{
    /**
     * @param array<string, mixed> $extraObjectFields merged into the fake
     *                                                 event's `data.object`, beyond `id`/`latest_charge`
     */
    protected function deliverPaymentIntentSucceeded(string $paymentIntentId, array $extraObjectFields = []): void
    {
        $this->deliverStripeWebhook('payment_intent.succeeded', array_merge([
            'id' => $paymentIntentId,
            'latest_charge' => 'ch_fake_'.bin2hex(random_bytes(8)),
        ], $extraObjectFields));
    }

    protected function deliverPaymentIntentFailed(string $paymentIntentId): void
    {
        $this->deliverStripeWebhook('payment_intent.payment_failed', ['id' => $paymentIntentId]);
    }

    /**
     * BR-05-12: an out-of-band (Stripe Dashboard) refund reconciliation.
     */
    protected function deliverChargeRefunded(string $paymentIntentId, int $amountRefundedMinorUnits, ?string $stripeRefundId = null): void
    {
        $this->deliverStripeWebhook('charge.refunded', [
            'payment_intent' => $paymentIntentId,
            'amount_refunded' => $amountRefundedMinorUnits,
            'refunds' => ['data' => [['id' => $stripeRefundId ?? 're_fake_'.bin2hex(random_bytes(8))]]],
        ]);
    }

    protected function deliverAccountUpdated(string $stripeAccountId, bool $chargesEnabled, bool $detailsSubmitted): void
    {
        $this->deliverStripeWebhook('account.updated', [
            'id' => $stripeAccountId,
            'charges_enabled' => $chargesEnabled,
            'details_submitted' => $detailsSubmitted,
        ]);
    }

    protected function deliverPlatformSubscriptionCreated(string $stripeSubscriptionId): void
    {
        $this->deliverStripeWebhook('customer.subscription.created', ['id' => $stripeSubscriptionId]);
    }

    protected function deliverPlatformSubscriptionDeleted(string $stripeSubscriptionId): void
    {
        $this->deliverStripeWebhook('customer.subscription.deleted', ['id' => $stripeSubscriptionId]);
    }

    /**
     * @param array<string, mixed> $object
     */
    protected function deliverStripeWebhook(string $type, array $object, ?string $stripeEventId = null): StripeEventReceipt
    {
        $eventId = $stripeEventId ?? 'evt_fake_'.bin2hex(random_bytes(10));
        $payload = json_encode(['id' => $eventId, 'type' => $type, 'data' => ['object' => $object]], \JSON_THROW_ON_ERROR);

        /** @var StripeEventReceiptRepository $receipts */
        $receipts = self::getContainer()->get(StripeEventReceiptRepository::class);
        $receipt = new StripeEventReceipt($eventId, $type, $payload);
        $receipts->add($receipt);
        $this->webhookEntityManager()->flush();

        /** @var ProcessStripeWebhookEventHandler $handler */
        $handler = self::getContainer()->get(ProcessStripeWebhookEventHandler::class);
        $handler(new ProcessStripeWebhookEvent((int) $receipt->getId()));

        return $receipt;
    }

    private function webhookEntityManager(): EntityManagerInterface
    {
        /** @var EntityManagerInterface $entityManager */
        $entityManager = self::getContainer()->get(EntityManagerInterface::class);

        return $entityManager;
    }
}
