<?php

declare(strict_types=1);

namespace App\Billing\Stripe;

/**
 * The platform's ENTIRE surface onto the Stripe API — deliberately narrow,
 * naming only the operations `StripeGateway` (App\Billing\Service) actually
 * needs, not a general-purpose SDK wrapper. This is the "injectable Stripe
 * client interface with a test double" the task brief calls for: no code
 * outside this interface and its implementations ever names a Stripe type
 * or calls out to `api.stripe.com`.
 *
 * `HttpStripeClient` is the real implementation, built on
 * `symfony/http-client` (already a dependency) rather than the official
 * `stripe-php` SDK — deliberately: `STRIPE_SECRET_KEY` is blank in every
 * environment this codebase runs in (the task's own hard rule — "No real
 * Stripe calls"), so pulling in a large SDK purely for types that are never
 * exercised against a real key buys nothing a narrow interface plus a
 * fake test double does not already provide, and avoids a Composer
 * dependency this environment cannot safely verify has network access to
 * install. `FakeStripeClient` (tests/Support) is the only implementation
 * ever actually invoked by the test suite.
 *
 * Every write operation takes an idempotency key, derived by the CALLER
 * (`StripeGateway`) from a `PaymentRecord` id persisted before the call —
 * architect-architecture.md "Payment records": "a timeout retry reuses the
 * key." This interface does not generate keys itself.
 */
interface StripeClient
{
    /**
     * BR-05-6: "All USD payments go through Stripe Checkout." Used for a
     * token-package purchase, a card RSVP, a card content purchase, and a
     * player subscription purchase alike — `$request->mode` distinguishes
     * one-time payment from subscription.
     */
    public function createCheckoutSession(StripeCheckoutSessionRequest $request): StripeCheckoutSession;

    /**
     * BR-05-11: refund a charge, fully or partially — keyed by the
     * PaymentIntent id (captured synchronously at Checkout-session creation,
     * always available once a charge attempt exists) rather than the Charge
     * id (only known once the `payment_intent.succeeded` webhook arrives;
     * Stripe's own refund API accepts either). Stripe's own timeline (5-10
     * business days for the money to actually arrive) is never tracked
     * further locally past this call succeeding or failing.
     */
    public function createRefund(string $paymentIntentId, int $amountMinorUnits, string $idempotencyKey): StripeRefund;

    /**
     * US-05.01: Stripe Connect Express onboarding, step 1 — the platform
     * account.
     */
    public function createConnectAccount(string $email, string $businessName, string $idempotencyKey): string;

    /**
     * US-05.01: Stripe Connect Express onboarding, step 2 — the one-time
     * onboarding URL Stripe redirects the trainer to.
     */
    public function createAccountLink(string $stripeAccountId, string $returnUrl, string $refreshUrl): string;

    /**
     * AC-05-1: "sees status 'Stripe Connected ✓'" — verified against
     * Stripe's own Account object, never trusted from the redirect alone.
     */
    public function retrieveAccountStatus(string $stripeAccountId): StripeAccountStatus;

    /**
     * AC-05-21: "each parent/player account has exactly one Stripe Customer
     * ID... shared across all of that account's trainers." $existingCustomerId
     * is null on first purchase.
     */
    public function createOrRetrieveCustomer(string $email, ?string $existingCustomerId, string $idempotencyKey): string;

    /**
     * AC-05-20: "redirects to the Stripe Customer Portal."
     */
    public function createBillingPortalSession(string $stripeCustomerId, string $returnUrl): string;

    /**
     * BR-05-13: the trainer's own $15/month platform subscription, via
     * Stripe Billing — never a player's (those are token-based, BR-05-14).
     */
    public function createSubscription(string $stripeCustomerId, int $amountMinorUnits, string $idempotencyKey): string;

    /**
     * AC-05-25/26: the only source of any figure presented as earnings,
     * payout or revenue (architect-architecture.md "The earnings
     * boundary") — read live, never persisted.
     */
    public function retrieveConnectBalanceSummary(string $stripeAccountId): StripeEarningsSummary;

    /**
     * AC-05-35/36: verifies the `Stripe-Signature` header against the raw
     * body — the SDK's own HMAC scheme, reimplemented narrowly here rather
     * than pulling in `stripe-php` for this one static computation (see
     * this interface's own docblock).
     *
     * @throws StripeSignatureVerificationException
     */
    public function verifyWebhookSignature(string $payload, string $sigHeader, string $endpointSecret): StripeWebhookEvent;
}
