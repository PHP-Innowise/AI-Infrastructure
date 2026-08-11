<?php

declare(strict_types=1);

namespace App\Billing\Service;

use App\Billing\Entity\PaymentRecord;
use App\Billing\Entity\PlatformSubscription;
use App\Billing\Entity\StripeCustomerLink;
use App\Billing\Entity\TrainerBillingSettings;
use App\Billing\Repository\PaymentRecordRepository;
use App\Billing\Repository\PlatformSubscriptionRepository;
use App\Billing\Repository\StripeCustomerLinkRepository;
use App\Billing\Repository\TrainerBillingSettingsRepository;
use App\Billing\Stripe\StripeCheckoutSession;
use App\Billing\Stripe\StripeCheckoutSessionRequest;
use App\Billing\Stripe\StripeClient;
use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use Doctrine\ORM\EntityManagerInterface;

/**
 * "The **only** component that calls the Stripe write API. Derives
 * idempotency keys from a payment record persisted before the call"
 * (architect-architecture.md "Cross-cutting services"). Every other
 * Billing service (`SchedulingPaymentIntentGateway`,
 * `ContentPaymentIntentGateway`, `TokenPurchaseService`,
 * `SubscriptionPurchaseService`, `StripeConnectService`,
 * `PaymentMethodService`) goes through this rather than injecting
 * `StripeClient` directly.
 *
 * **Idempotency is a database constraint, outbound half**: every write
 * call's idempotency key is derived deterministically from a `PaymentRecord`
 * id persisted before the call — "a timeout retry reuses the key"
 * (architect-architecture.md "Payment records").
 *
 * @see specs/architect-architecture.md "Cross-cutting services", "Payment records"
 */
final readonly class StripeGateway
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private StripeClient $stripeClient,
        private PaymentRecordRepository $paymentRecords,
        private StripeCustomerLinkRepository $customerLinks,
        private TrainerBillingSettingsRepository $billingSettings,
        private PlatformSubscriptionRepository $platformSubscriptions,
        private FeeCalculator $feeCalculator,
    ) {
    }

    /**
     * The deterministic idempotency key every Stripe write derives from a
     * persisted `PaymentRecord` id.
     */
    public function idempotencyKeyFor(PaymentRecord $paymentRecord, string $purpose = 'checkout'): string
    {
        return sprintf('payment_record_%d_%s', (int) $paymentRecord->getId(), $purpose);
    }

    /**
     * BR-05-6: creates a Stripe Checkout Session for $paymentRecord (already
     * persisted, `pending`) and attaches the resulting PaymentIntent id.
     * $paymentRecord's own `trainer`/`amountMinorUnits`/`platformFeeMinorUnits`
     * drive the request; token-method records never reach here (a token
     * spend never touches Stripe at all).
     */
    /**
     * @param array<string, string> $metadata See
     *                                        `StripeCheckoutSessionRequest::$metadata`'s own docblock — carries request-time
     *                                        information forward to webhook-confirmation time for whatever
     *                                        `PaymentRecord` itself has no column for (a content purchase's
     *                                        beneficiary player, a subscription's chosen activation date).
     */
    public function createCheckoutSession(PaymentRecord $paymentRecord, string $successUrl, string $cancelUrl, string $mode = StripeCheckoutSessionRequest::MODE_PAYMENT, array $metadata = []): StripeCheckoutSession
    {
        if (PaymentRecord::METHOD_CARD !== $paymentRecord->getPaymentMethod()) {
            throw new \LogicException('Only card-method payment records create a Stripe Checkout Session.');
        }

        $settings = $this->billingSettings->getOrCreateForTrainer($paymentRecord->getTrainer());
        $customerId = null !== $paymentRecord->getPayerAccount()
            ? $this->customerIdFor($paymentRecord->getPayerAccount())
            : null;

        $session = $this->stripeClient->createCheckoutSession(new StripeCheckoutSessionRequest(
            mode: $mode,
            amountMinorUnits: $paymentRecord->getAmountMinorUnits(),
            description: $this->descriptionFor($paymentRecord),
            successUrl: $successUrl,
            cancelUrl: $cancelUrl,
            idempotencyKey: $this->idempotencyKeyFor($paymentRecord),
            stripeCustomerId: $customerId,
            connectedAccountId: StripeCheckoutSessionRequest::MODE_PAYMENT === $mode ? $settings->getStripeConnectAccountId() : null,
            applicationFeeMinorUnits: StripeCheckoutSessionRequest::MODE_PAYMENT === $mode ? $paymentRecord->getPlatformFeeMinorUnits() : null,
            metadata: $metadata,
        ));

        if (null !== $session->paymentIntentId) {
            $paymentRecord->attachStripePaymentIntentId($session->paymentIntentId);
            $this->entityManager->flush();
        }

        return $session;
    }

    /**
     * "Money and the platform fee — Where it runs": computed once, from the
     * rate in force at this exact moment, snapshotted onto $paymentRecord
     * and never recomputed. A `token`-method record still snapshots the
     * rate (the CHECK constraint requires every non-refund row to) but the
     * fee itself is always 0 — no real money moves through Stripe for a
     * token spend, so there is nothing to send as `application_fee_amount`.
     */
    public function applyCurrentFee(PaymentRecord $paymentRecord): void
    {
        $settings = $this->billingSettings->getOrCreateForTrainer($paymentRecord->getTrainer());
        $rate = $settings->getPlatformFeeBasisPoints();

        $fee = PaymentRecord::METHOD_CARD === $paymentRecord->getPaymentMethod()
            ? $this->feeCalculator->compute($paymentRecord->getAmountMinorUnits(), $rate)
            : 0;

        $paymentRecord->applyFee($rate, $fee);
    }

    /**
     * BR-05-11: refunds $paymentRecord (a completed, card-method charge) in
     * full or in part. Returns the new refund `PaymentRecord` on success,
     * null if Stripe reports the refund attempt failed — never throws for
     * an ordinary decline, so the caller can show a clean "refund failed"
     * outcome rather than a 500.
     */
    public function refund(PaymentRecord $original, int $amountMinorUnits): ?PaymentRecord
    {
        if (PaymentRecord::METHOD_CARD !== $original->getPaymentMethod()) {
            throw new \LogicException('Only card-method payment records are refunded through Stripe.');
        }

        if (null === $original->getStripePaymentIntentId()) {
            return null;
        }

        // Persisted (and flushed for an id) BEFORE the Stripe call, per this
        // class's own "idempotency is a database constraint" rule — a
        // timeout-and-retry of this exact refund attempt reuses the same
        // key rather than risking a double refund.
        $refundRecord = PaymentRecord::forRefund($original, $amountMinorUnits);
        $this->paymentRecords->add($refundRecord);
        $this->entityManager->flush();

        $result = $this->stripeClient->createRefund(
            $original->getStripePaymentIntentId(),
            $amountMinorUnits,
            $this->idempotencyKeyFor($refundRecord, 'refund'),
        );

        if (!$result->succeeded) {
            $this->entityManager->remove($refundRecord);
            $this->entityManager->flush();

            return null;
        }

        $original->markRefunded($result->refundId);
        $this->entityManager->flush();

        return $refundRecord;
    }

    /**
     * US-05.01: Stripe Connect Express onboarding, step 1 — creates the
     * connected account if the trainer does not have one yet.
     */
    public function ensureConnectAccount(Trainer $trainer): TrainerBillingSettings
    {
        $settings = $this->billingSettings->getOrCreateForTrainer($trainer);

        if (null === $settings->getStripeConnectAccountId()) {
            $accountId = $this->stripeClient->createConnectAccount(
                $trainer->getOwnerAccount()->getEmail(),
                $trainer->getBusinessName(),
                sprintf('trainer_connect_%d', (int) $trainer->getId()),
            );
            $settings->attachStripeConnectAccount($accountId);
            $this->entityManager->flush();
        }

        return $settings;
    }

    public function startConnectOnboarding(Trainer $trainer, string $returnUrl, string $refreshUrl): string
    {
        $settings = $this->ensureConnectAccount($trainer);

        \assert(null !== $settings->getStripeConnectAccountId());

        return $this->stripeClient->createAccountLink($settings->getStripeConnectAccountId(), $returnUrl, $refreshUrl);
    }

    /**
     * AC-05-1: verified against Stripe's own Account object, never trusted
     * from the redirect alone.
     */
    public function refreshConnectStatus(Trainer $trainer): TrainerBillingSettings
    {
        $settings = $this->billingSettings->getOrCreateForTrainer($trainer);
        $accountId = $settings->getStripeConnectAccountId();

        if (null === $accountId) {
            return $settings;
        }

        $status = $this->stripeClient->retrieveAccountStatus($accountId);
        $settings->updateOnboardingStatus($status->isOnboardingComplete() ? TrainerBillingSettings::ONBOARDING_COMPLETE : TrainerBillingSettings::ONBOARDING_INCOMPLETE);
        $this->entityManager->flush();

        return $settings;
    }

    /**
     * AC-05-21: exactly one Stripe Customer per account, shared across
     * every trainer.
     */
    public function customerIdFor(Account $account): string
    {
        $link = $this->customerLinks->findForAccount($account);

        if (null !== $link) {
            return $link->getStripeCustomerId();
        }

        $customerId = $this->stripeClient->createOrRetrieveCustomer(
            $account->getEmail(),
            null,
            sprintf('customer_for_account_%d', (int) $account->getId()),
        );

        $link = new StripeCustomerLink($account, $customerId);
        $this->customerLinks->add($link);
        $this->entityManager->flush();

        return $customerId;
    }

    /**
     * AC-05-20: redirects to the Stripe Customer Portal.
     */
    public function billingPortalUrlFor(Account $account, string $returnUrl): string
    {
        return $this->stripeClient->createBillingPortalSession($this->customerIdFor($account), $returnUrl);
    }

    /**
     * BR-05-13: the trainer's own $15/month subscription to the platform
     * owner, provisioned directly (not via an interactive Checkout redirect
     * — there is no trainer-facing "subscribe" click in any epic; Epic-07's
     * `administration_trainer_create` is the actual trigger, per
     * specs/api-designer-spec.md's own description of that route, and
     * Epic-07 does not exist in this codebase — this method is the prepared
     * extension point for it).
     */
    public function provisionPlatformSubscription(Trainer $trainer): void
    {
        $settings = $this->billingSettings->getOrCreateForTrainer($trainer);
        $subscription = $this->platformSubscriptions->getOrCreateForTrainer($trainer);

        if (null !== $subscription->getStripeSubscriptionId()) {
            return;
        }

        $customerId = $this->customerIdFor($trainer->getOwnerAccount());
        $stripeSubscriptionId = $this->stripeClient->createSubscription(
            $customerId,
            $settings->getMonthlySubscriptionPriceMinorUnits(),
            sprintf('platform_subscription_%d', (int) $trainer->getId()),
        );

        $subscription->attachStripeSubscription($stripeSubscriptionId);
        $subscription->updateStatus(PlatformSubscription::STATUS_ACTIVE);
        $this->entityManager->flush();
    }

    private function descriptionFor(PaymentRecord $paymentRecord): string
    {
        return match ($paymentRecord->getType()) {
            PaymentRecord::TYPE_TOKEN_PURCHASE => 'Token purchase',
            PaymentRecord::TYPE_EVENT_RSVP => 'Event registration',
            PaymentRecord::TYPE_CONTENT_PURCHASE => 'Content access',
            PaymentRecord::TYPE_PLAYER_SUBSCRIPTION => 'Unlimited access subscription',
            PaymentRecord::TYPE_CAMP_REGISTRATION => 'Camp registration',
            default => 'PracticePerfect payment',
        };
    }
}
