<?php

declare(strict_types=1);

namespace App\Billing\MessageHandler;

use App\Billing\Entity\PaymentRecord;
use App\Billing\Entity\PlatformSubscription;
use App\Billing\Entity\SubscriptionEntitlement;
use App\Billing\Entity\TrainerBillingSettings;
use App\Billing\Event\PaymentRecordSettled;
use App\Billing\Message\ProcessStripeWebhookEvent;
use App\Billing\Repository\PaymentRecordRepository;
use App\Billing\Repository\PlatformSubscriptionRepository;
use App\Billing\Repository\StripeEventReceiptRepository;
use App\Billing\Repository\SubscriptionEntitlementRepository;
use App\Billing\Repository\TrainerBillingSettingsRepository;
use App\Billing\Service\BillingMailer;
use App\Billing\Service\TokenLedgerService;
use App\Platform\Repository\TrainerRepository;
use App\Platform\Service\CrossTenantReadService;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;
use Psr\Log\LoggerInterface;
use Symfony\Component\EventDispatcher\EventDispatcherInterface;
use Symfony\Component\Messenger\Attribute\AsMessageHandler;

/**
 * AC-05-34: the async webhook handler — "the message carries only the
 * receipt row's id... the handler re-reads rawPayload from the persisted
 * row" (specs/api-designer-spec.md "Stripe webhook contract" step 5/6).
 *
 * **Six events, six effects** (AC-05-34's own list), plus the
 * `customer.subscription.*` reading settled here rather than left
 * ambiguous: BR-05-14 states plainly that a Player Subscription is "not a
 * separate Stripe subscription" — it is funded by a one-time (`mode=payment`)
 * Checkout Session, so `payment_intent.succeeded` is what grants a player's
 * entitlement (this handler's own `type === player_subscription` branch,
 * "no grace period" per BR-05-14). `customer.subscription.created/deleted`
 * therefore can only describe a genuine Stripe Subscription object, which
 * in this codebase's actual scope is exclusively the TRAINER's own $15/mo
 * platform subscription (BR-05-13) — this handler reads those two events as
 * "activate/cancel the trainer's PlatformSubscription," not as a
 * player-facing effect. Recorded here and in the coder's final report as an
 * interpretation of an epic-level AC that does not itself distinguish the
 * two subscription concepts.
 *
 * Every branch: resolve the trainer via `CrossTenantReadService` (the only
 * read possible before a tenant is known), activate `TenantContext`, THEN
 * do the real, RLS-bound work — matching architecture "Messenger
 * middleware: tenant context is established... per message."
 *
 * @see specs/api-designer-spec.md "Stripe webhook contract"
 * @see specs/requirements-analyst-epic-05-payments-tokens-spec.md AC-05-34..37
 */
#[AsMessageHandler]
final readonly class ProcessStripeWebhookEventHandler
{
    public function __construct(
        private StripeEventReceiptRepository $receipts,
        private PaymentRecordRepository $paymentRecords,
        private TrainerBillingSettingsRepository $billingSettings,
        private PlatformSubscriptionRepository $platformSubscriptions,
        private SubscriptionEntitlementRepository $entitlements,
        private TokenLedgerService $tokenLedger,
        private CrossTenantReadService $crossTenantRead,
        private TenantContext $tenantContext,
        private TrainerRepository $trainers,
        private EntityManagerInterface $entityManager,
        private EventDispatcherInterface $eventDispatcher,
        private BillingMailer $mailer,
        private LoggerInterface $logger,
    ) {
    }

    public function __invoke(ProcessStripeWebhookEvent $message): void
    {
        $receipt = $this->receipts->find($message->stripeEventReceiptId);

        if (null === $receipt) {
            // Nothing sensible to retry — the row itself is gone.
            return;
        }

        /** @var array{id?: mixed, type?: mixed, data?: array{object?: mixed}} $decoded */
        $decoded = json_decode($receipt->getRawPayload(), true, flags: \JSON_THROW_ON_ERROR);
        /** @var array<string, mixed> $object */
        $object = \is_array($decoded['data']['object'] ?? null) ? $decoded['data']['object'] : [];

        // A raw DBAL transaction, not EntityManager::wrapInTransaction():
        // that helper closes the EntityManager entirely on ANY exception
        // from its callback (see StripeConnectTest's own docblock, which
        // documents this the same discovery from the HTTP side), which
        // would leave nothing usable to record the failure with below in
        // this single, non-request-bound process. Rolling back at the
        // DBAL level instead undoes every mutation the per-event-type
        // handler made (e.g. handlePaymentIntentSucceeded() calls
        // markCompleted() before the token-purchase branch's own
        // payer-account check, which can still throw) while leaving the
        // EntityManager itself open for the failure-recording write that
        // follows — a failed attempt must leave NO partial state, or a
        // retry sees `$paymentRecord->isCompleted()` already true and
        // silently no-ops forever without ever crediting the tokens.
        $connection = $this->entityManager->getConnection();
        $connection->beginTransaction();

        try {
            match ($receipt->getEventType()) {
                'payment_intent.succeeded' => $this->handlePaymentIntentSucceeded($object),
                'payment_intent.payment_failed' => $this->handlePaymentIntentFailed($object),
                'charge.refunded' => $this->handleChargeRefunded($object),
                'customer.subscription.created' => $this->handlePlatformSubscriptionCreated($object),
                'customer.subscription.deleted' => $this->handlePlatformSubscriptionDeleted($object),
                'account.updated' => $this->handleAccountUpdated($object),
                default => $this->logger->info('Unhandled Stripe webhook event type.', ['type' => $receipt->getEventType()]),
            };

            $receipt->markProcessed();
            $this->entityManager->flush();
            $connection->commit();
        } catch (\Throwable $exception) {
            $connection->rollBack();
            // Every entity touched above (the receipt included) now
            // reflects rolled-back, stale in-memory state — cleared so the
            // failure-recording write below starts from a clean identity
            // map, matching a fresh find() rather than a detached object.
            $this->entityManager->clear();

            $freshReceipt = $this->receipts->find($message->stripeEventReceiptId);

            if (null !== $freshReceipt) {
                $freshReceipt->markFailed($exception->getMessage());
                $this->entityManager->flush();
            }

            // AC-05-37: "Stripe API errors are logged and the admin is
            // notified" — logged here; Messenger's own retry_strategy
            // (config/packages/messenger.yaml) is what re-attempts this
            // handler (AC-05-35).
            $this->logger->error('Stripe webhook processing failed.', [
                'stripeEventId' => $receipt->getStripeEventId(),
                'type' => $receipt->getEventType(),
                'error' => $exception->getMessage(),
            ]);

            throw $exception;
        } finally {
            $this->tenantContext->clear();
        }
    }

    /**
     * @param array<string, mixed> $object
     */
    private function handlePaymentIntentSucceeded(array $object): void
    {
        $paymentIntentId = (string) ($object['id'] ?? '');
        $located = $this->crossTenantRead->resolvePaymentRecordByStripePaymentIntent($paymentIntentId);

        if (null === $located) {
            return;
        }

        $this->activateTenant($located['trainerId']);

        $paymentRecord = $this->paymentRecords->find($located['id']);

        if (null === $paymentRecord || $paymentRecord->isCompleted()) {
            // Already handled (redelivery) or somehow gone — idempotent no-op.
            return;
        }

        $chargeId = \is_string($object['latest_charge'] ?? null) ? $object['latest_charge'] : null;
        $paymentRecord->markCompleted($chargeId);

        // AC-05-4: a card-funded token purchase credits the ledger only
        // once the webhook confirms it (never at Checkout-session creation
        // time, which is why the balance did not move when the redirect
        // happened).
        if (PaymentRecord::TYPE_TOKEN_PURCHASE === $paymentRecord->getType() && null !== $paymentRecord->getRelatedTokenPackage()) {
            $package = $paymentRecord->getRelatedTokenPackage();
            $entry = $this->tokenLedger->purchase(
                $paymentRecord->getTrainer(),
                $paymentRecord->getPayerAccount() ?? throw new \LogicException('A token purchase always has a payer account.'),
                $package->getTokenCount(),
                $paymentRecord,
                sprintf('Token purchase: %s', $package->getLabel()),
            );
            \assert(null !== $entry->getId());
            $this->mailer->sendTokenPurchaseConfirmed($paymentRecord, $package->getTokenCount());
        }

        // AC-05-29, BR-05-14: "the subscription token is granted only after
        // successful payment, with no grace period" — this webhook is the
        // ONLY place a SubscriptionEntitlement is created.
        if (PaymentRecord::TYPE_PLAYER_SUBSCRIPTION === $paymentRecord->getType()) {
            $this->grantSubscriptionEntitlement($paymentRecord, $object);
        }

        $this->entityManager->flush();

        $this->eventDispatcher->dispatch(new PaymentRecordSettled(
            (int) $paymentRecord->getId(),
            $paymentRecord->getType(),
            PaymentRecordSettled::OUTCOME_SUCCEEDED,
            $paymentRecord->getRelatedRsvp()?->getId(),
            $paymentRecord->getRelatedPlaylist()?->getId(),
            $this->stringMetadata($object),
            $paymentRecord->getRelatedFormSubmission()?->getId(),
        ));
    }

    /**
     * @param array<string, mixed> $object
     */
    private function handlePaymentIntentFailed(array $object): void
    {
        $paymentIntentId = (string) ($object['id'] ?? '');
        $located = $this->crossTenantRead->resolvePaymentRecordByStripePaymentIntent($paymentIntentId);

        if (null === $located) {
            return;
        }

        $this->activateTenant($located['trainerId']);

        $paymentRecord = $this->paymentRecords->find($located['id']);

        if (null === $paymentRecord || PaymentRecord::STATUS_PENDING !== $paymentRecord->getStatus()) {
            return;
        }

        $paymentRecord->markFailed();
        $this->entityManager->flush();

        $this->mailer->sendPaymentFailed($paymentRecord);

        $this->eventDispatcher->dispatch(new PaymentRecordSettled(
            (int) $paymentRecord->getId(),
            $paymentRecord->getType(),
            PaymentRecordSettled::OUTCOME_FAILED,
            $paymentRecord->getRelatedRsvp()?->getId(),
            $paymentRecord->getRelatedPlaylist()?->getId(),
        ));
    }

    /**
     * BR-05-12: reconciles an out-of-band (Stripe Dashboard) refund. A
     * refund the PLATFORM itself initiated (`StripeGateway::refund()`,
     * called synchronously from `SchedulingPaymentIntentGateway`/
     * `ContentPaymentIntentGateway`) already created its own local refund
     * `PaymentRecord` at that moment — this webhook's arrival for THAT
     * refund is a pure reconciliation confirmation with nothing left to do,
     * detected by the refund id already being known locally.
     *
     * @param array<string, mixed> $object
     */
    private function handleChargeRefunded(array $object): void
    {
        $paymentIntentId = \is_string($object['payment_intent'] ?? null) ? $object['payment_intent'] : null;

        if (null === $paymentIntentId) {
            return;
        }

        $located = $this->crossTenantRead->resolvePaymentRecordByStripePaymentIntent($paymentIntentId);

        if (null === $located) {
            return;
        }

        $this->activateTenant($located['trainerId']);

        $original = $this->paymentRecords->find($located['id']);

        if (null === $original) {
            return;
        }

        $stripeRefundId = \is_string($object['refunds']['data'][0]['id'] ?? null) ? $object['refunds']['data'][0]['id'] : null;

        if (null !== $stripeRefundId && null !== $this->paymentRecords->findOneByStripeRefundId($stripeRefundId)) {
            // Already reconciled by the platform's own synchronous refund
            // flow — nothing left to do (see this method's own docblock).
            return;
        }

        if (PaymentRecord::STATUS_REFUNDED === $original->getStatus()) {
            return;
        }

        $refundedAmount = (int) ($object['amount_refunded'] ?? $original->getAmountMinorUnits());
        $refundRecord = PaymentRecord::forRefund($original, max(1, $refundedAmount));
        $this->paymentRecords->add($refundRecord);
        $original->markRefunded($stripeRefundId);
        $this->entityManager->flush();

        // BR-05-12: "the platform does not enforce rules on manual
        // refunds" — an out-of-band token-package refund is reconciled in
        // transaction history above; reversing the tokens it originally
        // granted is a deliberate Super-Admin `adjustment` action (A7's own
        // "Super-Admin-only" rule), not something this automated handler
        // performs on its own. Recorded in the coder's final report.
        $this->eventDispatcher->dispatch(new PaymentRecordSettled(
            (int) $original->getId(),
            $original->getType(),
            PaymentRecordSettled::OUTCOME_REFUNDED,
            $original->getRelatedRsvp()?->getId(),
            $original->getRelatedPlaylist()?->getId(),
        ));
    }

    /**
     * @param array<string, mixed> $object
     */
    private function handlePlatformSubscriptionCreated(array $object): void
    {
        $stripeSubscriptionId = (string) ($object['id'] ?? '');
        $trainerId = $this->crossTenantRead->resolveTrainerIdForPlatformStripeSubscription($stripeSubscriptionId);

        if (null === $trainerId) {
            return;
        }

        $this->activateTenant($trainerId);

        $trainer = $this->trainers->find($trainerId) ?? throw new \LogicException('Trainer not found.');
        $subscription = $this->platformSubscriptions->getOrCreateForTrainer($trainer);
        $subscription->attachStripeSubscription($stripeSubscriptionId);
        $subscription->updateStatus(PlatformSubscription::STATUS_ACTIVE);
        $this->entityManager->flush();
    }

    /**
     * @param array<string, mixed> $object
     */
    private function handlePlatformSubscriptionDeleted(array $object): void
    {
        $stripeSubscriptionId = (string) ($object['id'] ?? '');
        $trainerId = $this->crossTenantRead->resolveTrainerIdForPlatformStripeSubscription($stripeSubscriptionId);

        if (null === $trainerId) {
            return;
        }

        $this->activateTenant($trainerId);

        $trainer = $this->trainers->find($trainerId) ?? throw new \LogicException('Trainer not found.');
        $subscription = $this->platformSubscriptions->getOrCreateForTrainer($trainer);
        $subscription->updateStatus(PlatformSubscription::STATUS_CANCELED);
        $this->entityManager->flush();
    }

    /**
     * AC-05-1: refreshes the trainer's Connect/onboarding status shown on
     * `billing_trainer_settings`.
     *
     * @param array<string, mixed> $object
     */
    private function handleAccountUpdated(array $object): void
    {
        $stripeAccountId = (string) ($object['id'] ?? '');
        $trainerId = $this->crossTenantRead->resolveTrainerIdForStripeConnectAccount($stripeAccountId);

        if (null === $trainerId) {
            return;
        }

        $this->activateTenant($trainerId);

        $trainer = $this->trainers->find($trainerId) ?? throw new \LogicException('Trainer not found.');
        $settings = $this->billingSettings->getOrCreateForTrainer($trainer);

        $chargesEnabled = (bool) ($object['charges_enabled'] ?? false);
        $detailsSubmitted = (bool) ($object['details_submitted'] ?? false);
        $status = $chargesEnabled && $detailsSubmitted ? TrainerBillingSettings::ONBOARDING_COMPLETE : TrainerBillingSettings::ONBOARDING_INCOMPLETE;

        $settings->updateOnboardingStatus($status);
        $this->entityManager->flush();
    }

    /**
     * AC-05-29: grants the entitlement. The purchaser-chosen activation
     * date has no `PaymentRecord` column to live in (see that entity's own
     * docblock), so it travels as Stripe Checkout metadata instead — set at
     * session creation (`SubscriptionPurchaseService`,
     * `StripeCheckoutSessionRequest::$metadata`'s own docblock explains
     * why) and echoed back onto the PaymentIntent this webhook's `$object`
     * IS. Falls back to today if somehow absent, rather than failing the
     * whole webhook.
     *
     * Idempotency (AC-05-35 replay-safety) is checked via
     * `SubscriptionEntitlementRepository::findOneByPaymentRecord()`, not a
     * `PaymentRecord`-side column — `PaymentRecord` has no `related*`
     * pointer to its entitlement (see
     * `PaymentRecord::TYPE_PLAYER_SUBSCRIPTION`'s own docblock); the
     * required, unique `subscription_entitlement.payment_record_id` is
     * already the single source of truth for this fact, so the check reads
     * that side instead of duplicating it.
     *
     * @param array<string, mixed> $object
     */
    private function grantSubscriptionEntitlement(PaymentRecord $paymentRecord, array $object): void
    {
        if (null !== $this->entitlements->findOneByPaymentRecord($paymentRecord)) {
            return;
        }

        $payer = $paymentRecord->getPayerAccount() ?? throw new \LogicException('A player subscription always has a payer account.');

        $metadata = \is_array($object['metadata'] ?? null) ? $object['metadata'] : [];
        $activationDateString = \is_string($metadata['activation_date'] ?? null) ? $metadata['activation_date'] : null;
        $activationDate = null !== $activationDateString
            ? new \DateTimeImmutable($activationDateString)
            : new \DateTimeImmutable('today');

        $entitlement = new SubscriptionEntitlement($paymentRecord->getTrainer(), $payer, $activationDate, $paymentRecord);
        $this->entitlements->add($entitlement);
        $this->entityManager->flush();

        $this->mailer->sendSubscriptionActivated($paymentRecord, $entitlement->getActivationDate(), $entitlement->getWindowEndsOn());
    }

    private function activateTenant(int $trainerId): void
    {
        $trainer = $this->trainers->find($trainerId) ?? throw new \LogicException(sprintf('Trainer %d not found.', $trainerId));
        $this->tenantContext->activateFor($trainer);
    }

    /**
     * @param array<string, mixed> $object
     *
     * @return array<string, string>
     */
    private function stringMetadata(array $object): array
    {
        $metadata = \is_array($object['metadata'] ?? null) ? $object['metadata'] : [];

        /** @var array<string, string> */
        return array_map(static fn (mixed $value): string => (string) $value, $metadata);
    }
}
