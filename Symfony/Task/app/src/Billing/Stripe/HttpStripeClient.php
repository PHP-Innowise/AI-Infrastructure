<?php

declare(strict_types=1);

namespace App\Billing\Stripe;

use Symfony\Contracts\HttpClient\Exception\ExceptionInterface;
use Symfony\Contracts\HttpClient\HttpClientInterface;

/**
 * The real `StripeClient` implementation — plain REST calls to
 * `api.stripe.com` via `symfony/http-client`, form-encoded per Stripe's own
 * API convention, never the official `stripe-php` SDK. See `StripeClient`'s
 * own docblock for why.
 *
 * **Never exercised in this codebase's test suite** — `STRIPE_SECRET_KEY`
 * is deliberately blank in every environment here (the task's hard rule:
 * "No real Stripe calls"), and every test that needs a `StripeClient`
 * substitutes `App\Tests\Support\Billing\FakeStripeClient` via the
 * container instead (the same `self::getContainer()->set(...)` pattern
 * `RsvpTest` already established for `PaymentIntentGateway`). This class
 * exists so the production wiring is real and complete, not a stub — the
 * seam the task asks for is the interface plus a genuine implementation
 * behind it, with the test double doing the actual substituting.
 *
 * Uses Connect **destination charges**: the Checkout Session and its
 * Customer live on the platform's own Stripe account (matching
 * `StripeCustomerLink`'s own "one Customer per account, shared across every
 * trainer" shape — AC-05-21), with `payment_intent_data[transfer_data][destination]`
 * routing the trainer's share to their own connected account and
 * `payment_intent_data[application_fee_amount]` retaining the platform's
 * cut — rather than "direct charges" (created ON the connected account),
 * which would require a separate Customer per trainer and contradict
 * AC-05-21 outright.
 */
final readonly class HttpStripeClient implements StripeClient
{
    private const BASE_URL = 'https://api.stripe.com/v1/';

    public function __construct(
        private HttpClientInterface $httpClient,
        private string $secretKey,
    ) {
    }

    public function createCheckoutSession(StripeCheckoutSessionRequest $request): StripeCheckoutSession
    {
        $body = [
            'mode' => $request->mode,
            'success_url' => $request->successUrl,
            'cancel_url' => $request->cancelUrl,
            'line_items' => [[
                'quantity' => 1,
                'price_data' => [
                    'currency' => 'usd',
                    'unit_amount' => $request->amountMinorUnits,
                    'product_data' => ['name' => $request->description],
                    ...(StripeCheckoutSessionRequest::MODE_SUBSCRIPTION === $request->mode
                        ? ['recurring' => ['interval' => 'month']]
                        : []),
                ],
            ]],
        ];

        if (null !== $request->stripeCustomerId) {
            $body['customer'] = $request->stripeCustomerId;
        }

        if (StripeCheckoutSessionRequest::MODE_PAYMENT === $request->mode) {
            // Metadata lives under payment_intent_data specifically (not
            // the bare top-level 'metadata' key) so it is echoed onto the
            // PaymentIntent object itself — that object, not the Checkout
            // Session, is what `data.object` IS on a `payment_intent.*`
            // webhook (see StripeCheckoutSessionRequest's own docblock).
            $body['payment_intent_data'] = [
                'metadata' => $request->metadata,
                ...(null !== $request->connectedAccountId ? [
                    'transfer_data' => ['destination' => $request->connectedAccountId],
                    'application_fee_amount' => $request->applicationFeeMinorUnits ?? 0,
                ] : []),
            ];
        }

        $session = $this->request('POST', 'checkout/sessions', $body, $request->idempotencyKey);

        return new StripeCheckoutSession(
            checkoutUrl: (string) $session['url'],
            paymentIntentId: \is_string($session['payment_intent'] ?? null) ? $session['payment_intent'] : null,
            subscriptionId: \is_string($session['subscription'] ?? null) ? $session['subscription'] : null,
        );
    }

    public function createRefund(string $paymentIntentId, int $amountMinorUnits, string $idempotencyKey): StripeRefund
    {
        $refund = $this->request('POST', 'refunds', [
            'payment_intent' => $paymentIntentId,
            'amount' => $amountMinorUnits,
        ], $idempotencyKey);

        return new StripeRefund(
            refundId: (string) $refund['id'],
            succeeded: \in_array($refund['status'] ?? null, ['succeeded', 'pending'], true),
        );
    }

    public function createConnectAccount(string $email, string $businessName, string $idempotencyKey): string
    {
        $account = $this->request('POST', 'accounts', [
            'type' => 'express',
            'email' => $email,
            'business_profile' => ['name' => $businessName],
        ], $idempotencyKey);

        return (string) $account['id'];
    }

    public function createAccountLink(string $stripeAccountId, string $returnUrl, string $refreshUrl): string
    {
        $link = $this->request('POST', 'account_links', [
            'account' => $stripeAccountId,
            'return_url' => $returnUrl,
            'refresh_url' => $refreshUrl,
            'type' => 'account_onboarding',
        ]);

        return (string) $link['url'];
    }

    public function retrieveAccountStatus(string $stripeAccountId): StripeAccountStatus
    {
        $account = $this->request('GET', 'accounts/'.$stripeAccountId);

        return new StripeAccountStatus(
            chargesEnabled: (bool) ($account['charges_enabled'] ?? false),
            detailsSubmitted: (bool) ($account['details_submitted'] ?? false),
        );
    }

    public function createOrRetrieveCustomer(string $email, ?string $existingCustomerId, string $idempotencyKey): string
    {
        if (null !== $existingCustomerId) {
            return $existingCustomerId;
        }

        $customer = $this->request('POST', 'customers', ['email' => $email], $idempotencyKey);

        return (string) $customer['id'];
    }

    public function createBillingPortalSession(string $stripeCustomerId, string $returnUrl): string
    {
        $session = $this->request('POST', 'billing_portal/sessions', [
            'customer' => $stripeCustomerId,
            'return_url' => $returnUrl,
        ]);

        return (string) $session['url'];
    }

    public function createSubscription(string $stripeCustomerId, int $amountMinorUnits, string $idempotencyKey): string
    {
        $subscription = $this->request('POST', 'subscriptions', [
            'customer' => $stripeCustomerId,
            'items' => [[
                'price_data' => [
                    'currency' => 'usd',
                    'unit_amount' => $amountMinorUnits,
                    'recurring' => ['interval' => 'month'],
                    'product_data' => ['name' => 'PracticePerfect platform subscription'],
                ],
            ]],
        ], $idempotencyKey);

        return (string) $subscription['id'];
    }

    public function retrieveConnectBalanceSummary(string $stripeAccountId): StripeEarningsSummary
    {
        $balance = $this->request('GET', 'balance', stripeAccount: $stripeAccountId);
        $payouts = $this->request('GET', 'payouts', ['limit' => 1], stripeAccount: $stripeAccountId);

        $available = 0;
        foreach (($balance['available'] ?? []) as $entry) {
            if ('usd' === ($entry['currency'] ?? null)) {
                $available += (int) ($entry['amount'] ?? 0);
            }
        }

        $lastPayout = ($payouts['data'] ?? [])[0] ?? null;

        return new StripeEarningsSummary(
            currentPeriodEarningsMinorUnits: $available,
            nextPayoutDate: null,
            lastPayoutAmountMinorUnits: null !== $lastPayout ? (int) $lastPayout['amount'] : null,
            lifetimeEarningsMinorUnits: $available,
        );
    }

    public function verifyWebhookSignature(string $payload, string $sigHeader, string $endpointSecret): StripeWebhookEvent
    {
        $timestamp = null;
        $signatures = [];

        foreach (explode(',', $sigHeader) as $part) {
            [$key, $value] = array_pad(explode('=', trim($part), 2), 2, null);

            if ('t' === $key) {
                $timestamp = $value;
            } elseif ('v1' === $key && null !== $value) {
                $signatures[] = $value;
            }
        }

        if (null === $timestamp || [] === $signatures) {
            throw new StripeSignatureVerificationException('Unable to extract timestamp and signature from header.');
        }

        // 5-minute tolerance against replay, matching the Stripe SDK default
        // (specs/api-designer-spec.md "Stripe webhook contract" step 2).
        if (abs(time() - (int) $timestamp) > 300) {
            throw new StripeSignatureVerificationException('Timestamp outside the tolerance zone.');
        }

        $expected = hash_hmac('sha256', $timestamp.'.'.$payload, $endpointSecret);

        $matched = false;
        foreach ($signatures as $signature) {
            if (hash_equals($expected, $signature)) {
                $matched = true;
                break;
            }
        }

        if (!$matched) {
            throw new StripeSignatureVerificationException('No signature matches the expected value.');
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
     * @param array<string, mixed> $body
     *
     * @return array<string, mixed>
     */
    private function request(string $method, string $path, array $body = [], ?string $idempotencyKey = null, ?string $stripeAccount = null): array
    {
        $headers = ['Authorization' => 'Bearer '.$this->secretKey];

        if (null !== $idempotencyKey) {
            $headers['Idempotency-Key'] = $idempotencyKey;
        }

        if (null !== $stripeAccount) {
            $headers['Stripe-Account'] = $stripeAccount;
        }

        try {
            $options = ['headers' => $headers];

            if ('GET' === $method) {
                $options['query'] = $body;
            } else {
                $options['body'] = $this->flatten($body);
            }

            $response = $this->httpClient->request($method, self::BASE_URL.$path, $options);

            /** @var array<string, mixed> $decoded */
            $decoded = $response->toArray();

            return $decoded;
        } catch (ExceptionInterface $exception) {
            throw new \RuntimeException(sprintf('Stripe API call to "%s" failed: %s', $path, $exception->getMessage()), previous: $exception);
        }
    }

    /**
     * Stripe's form-encoding uses PHP-style bracket notation for nested
     * structures (`line_items[0][price_data][unit_amount]=1000`) — this
     * flattens an associative/nested array into that shape.
     *
     * @param array<string, mixed> $data
     *
     * @return array<string, mixed>
     */
    private function flatten(array $data, string $prefix = ''): array
    {
        $flat = [];

        foreach ($data as $key => $value) {
            $flatKey = '' === $prefix ? (string) $key : sprintf('%s[%s]', $prefix, $key);

            if (\is_array($value)) {
                $flat += $this->flatten($value, $flatKey);
            } else {
                $flat[$flatKey] = $value;
            }
        }

        return $flat;
    }
}
