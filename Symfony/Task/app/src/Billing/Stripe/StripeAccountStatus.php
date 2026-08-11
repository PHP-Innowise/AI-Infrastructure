<?php

declare(strict_types=1);

namespace App\Billing\Stripe;

/**
 * AC-05-1: whether Stripe considers onboarding actually complete —
 * `chargesEnabled && detailsSubmitted`, Stripe's own standard Connect
 * Express readiness pair, never inferred from the mere fact that the
 * trainer was redirected back from Stripe (a browser redirect proves
 * nothing about whether onboarding was actually finished).
 */
final readonly class StripeAccountStatus
{
    public function __construct(
        public bool $chargesEnabled,
        public bool $detailsSubmitted,
    ) {
    }

    public function isOnboardingComplete(): bool
    {
        return $this->chargesEnabled && $this->detailsSubmitted;
    }
}
