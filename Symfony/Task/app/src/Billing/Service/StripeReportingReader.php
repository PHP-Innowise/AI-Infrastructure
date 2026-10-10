<?php

declare(strict_types=1);

namespace App\Billing\Service;

use App\Billing\Repository\TrainerBillingSettingsRepository;
use App\Billing\Stripe\StripeClient;
use App\Billing\Stripe\StripeEarningsSummary;
use App\Platform\Entity\Trainer;

/**
 * "The only source of any figure presented as earnings, payout or revenue.
 * Reads live; persists nothing" (architect-architecture.md "Cross-cutting
 * services"). AC-05-25/26: `billing_trainer_earnings` calls this and
 * NOTHING else for money figures — no repository sums `payment_record`
 * amounts into a display value anywhere in this codebase.
 *
 * When Stripe is unavailable this throws rather than returning a
 * locally-summed fallback (architect-architecture.md "The earnings
 * boundary": "When Stripe is unavailable the earnings panel shows an error
 * — never a locally-summed fallback, which would silently violate
 * AC-07-7").
 *
 * @see specs/architect-architecture.md "The earnings boundary"
 */
final readonly class StripeReportingReader
{
    public function __construct(
        private StripeClient $stripeClient,
        private TrainerBillingSettingsRepository $billingSettings,
    ) {
    }

    /**
     * @throws \RuntimeException if the trainer has no Connect account yet,
     *                            or the Stripe read itself fails
     */
    public function earningsSummaryFor(Trainer $trainer): StripeEarningsSummary
    {
        $settings = $this->billingSettings->getOrCreateForTrainer($trainer);
        $accountId = $settings->getStripeConnectAccountId();

        if (null === $accountId) {
            throw new \RuntimeException('This trainer has not connected Stripe yet.');
        }

        return $this->stripeClient->retrieveConnectBalanceSummary($accountId);
    }
}
