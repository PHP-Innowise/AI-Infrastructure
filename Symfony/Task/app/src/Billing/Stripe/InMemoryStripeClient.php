<?php

declare(strict_types=1);

namespace App\Billing\Stripe;

/**
 * The shipped `StripeClient` implementation for as long as
 * `STRIPE_SECRET_KEY` is blank — which is every environment this codebase
 * runs in today (the task's own hard rule: "No real Stripe calls.
 * STRIPE_SECRET_KEY is deliberately blank"). Mirrors
 * `App\Scheduling\Billing\NoopPaymentIntentGateway`'s own precedent and
 * reasoning exactly: it cannot move money, never claims to, and every
 * value it returns is clearly synthetic (a `_fake_` infix) rather than
 * something that could be mistaken for a real Stripe id if it ever leaked
 * into a log or a screen.
 *
 * Deterministic and configurable, not merely canned: callers (mainly
 * `StripeGateway` and its own tests) can inspect what was requested via
 * `lastCheckoutSessionRequest()` and friends, and tests can steer specific
 * outcomes (`setAccountStatus()`, `failNextRefund()`) the same way
 * `RsvpTest::testPaidEventConfirmsOnceGatewayReportsSuccess()` swaps in a
 * purpose-built stub for one exact scenario — most tests need nothing more
 * than this class's own sensible defaults.
 *
 * `config/services.yaml` binds `StripeClient` to this class unconditionally
 * today; switching to `HttpStripeClient` once a real key is configured for
 * an actual deployment is a one-line change there, not a code change here.
 */
final class InMemoryStripeClient implements StripeClient
{
    private const STATE_INCOMPLETE = 'incomplete';
    private const STATE_COMPLETE = 'complete';

    /** @var array<string, string> stripeAccountId => onboarding state (self::STATE_*) */
    private array $accountStatuses = [];

    /** @var array<string, StripeEarningsSummary> stripeAccountId => configured summary */
    private array $earningsSummaries = [];

    private bool $nextRefundFails = false;

    public ?StripeCheckoutSessionRequest $lastCheckoutSessionRequest = null;

    public function createCheckoutSession(StripeCheckoutSessionRequest $request): StripeCheckoutSession
    {
        $this->lastCheckoutSessionRequest = $request;
        $id = bin2hex(random_bytes(12));

        return new StripeCheckoutSession(
            checkoutUrl: sprintf('https://checkout.stripe.test/fake/cs_fake_%s', $id),
            paymentIntentId: StripeCheckoutSessionRequest::MODE_PAYMENT === $request->mode ? 'pi_fake_'.$id : null,
            subscriptionId: StripeCheckoutSessionRequest::MODE_SUBSCRIPTION === $request->mode ? 'sub_fake_'.$id : null,
        );
    }

    public function createRefund(string $paymentIntentId, int $amountMinorUnits, string $idempotencyKey): StripeRefund
    {
        if ($this->nextRefundFails) {
            $this->nextRefundFails = false;

            return new StripeRefund(refundId: '', succeeded: false);
        }

        return new StripeRefund(refundId: 're_fake_'.bin2hex(random_bytes(12)), succeeded: true);
    }

    public function createConnectAccount(string $email, string $businessName, string $idempotencyKey): string
    {
        $accountId = 'acct_fake_'.bin2hex(random_bytes(10));
        $this->accountStatuses[$accountId] = self::STATE_INCOMPLETE;

        return $accountId;
    }

    public function createAccountLink(string $stripeAccountId, string $returnUrl, string $refreshUrl): string
    {
        return sprintf('https://connect.stripe.test/fake/setup/%s', $stripeAccountId);
    }

    public function retrieveAccountStatus(string $stripeAccountId): StripeAccountStatus
    {
        $state = $this->accountStatuses[$stripeAccountId] ?? self::STATE_INCOMPLETE;

        return self::STATE_COMPLETE === $state
            ? new StripeAccountStatus(chargesEnabled: true, detailsSubmitted: true)
            : new StripeAccountStatus(chargesEnabled: false, detailsSubmitted: false);
    }

    /**
     * Test/dev seam: marks an account as having finished Connect
     * onboarding — there is no real Stripe redirect flow to actually drive
     * this in-memory.
     */
    public function setAccountOnboardingComplete(string $stripeAccountId): void
    {
        $this->accountStatuses[$stripeAccountId] = self::STATE_COMPLETE;
    }

    public function createOrRetrieveCustomer(string $email, ?string $existingCustomerId, string $idempotencyKey): string
    {
        return $existingCustomerId ?? 'cus_fake_'.bin2hex(random_bytes(10));
    }

    public function createBillingPortalSession(string $stripeCustomerId, string $returnUrl): string
    {
        return sprintf('https://billing.stripe.test/fake/portal/%s', $stripeCustomerId);
    }

    public function createSubscription(string $stripeCustomerId, int $amountMinorUnits, string $idempotencyKey): string
    {
        return 'sub_fake_'.bin2hex(random_bytes(10));
    }

    public function retrieveConnectBalanceSummary(string $stripeAccountId): StripeEarningsSummary
    {
        return $this->earningsSummaries[$stripeAccountId] ?? new StripeEarningsSummary(
            currentPeriodEarningsMinorUnits: 0,
            nextPayoutDate: null,
            lastPayoutAmountMinorUnits: null,
            lifetimeEarningsMinorUnits: 0,
        );
    }

    /**
     * Test seam: AC-05-25 needs distinguishable, non-zero/non-null figures
     * for every field to prove the trainer earnings screen actually
     * renders all four — the zero/null defaults above cannot demonstrate
     * that on their own.
     */
    public function setConnectBalanceSummary(string $stripeAccountId, StripeEarningsSummary $summary): void
    {
        $this->earningsSummaries[$stripeAccountId] = $summary;
    }

    public function verifyWebhookSignature(string $payload, string $sigHeader, string $endpointSecret): StripeWebhookEvent
    {
        // In-memory webhook delivery (App\Billing\Service\StripeWebhookProcessor's
        // own tests) uses a fixed, obviously-fake header shape rather than a
        // real HMAC — this client never receives traffic from the real
        // internet, so there is no signature to actually forge/verify
        // against. `sig_fake_valid` is the one header value this method
        // accepts; anything else fails closed, so a test asserting the
        // failure path (AC-05-36) is still exercised for real.
        if ('sig_fake_valid' !== $sigHeader) {
            throw new StripeSignatureVerificationException('Invalid fake signature.');
        }

        /** @var array{id?: mixed, type?: mixed, data?: array{object?: mixed}} $decoded */
        $decoded = json_decode($payload, true, flags: \JSON_THROW_ON_ERROR);

        /** @var array<string, mixed> $object */
        $object = \is_array($decoded['data']['object'] ?? null) ? $decoded['data']['object'] : [];

        return new StripeWebhookEvent(
            id: (string) ($decoded['id'] ?? ''),
            type: (string) ($decoded['type'] ?? ''),
            data: $object,
        );
    }

    /**
     * Test seam: the next createRefund() call reports failure instead of
     * success, once.
     */
    public function failNextRefund(): void
    {
        $this->nextRefundFails = true;
    }
}
