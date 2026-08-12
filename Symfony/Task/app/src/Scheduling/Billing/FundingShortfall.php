<?php

declare(strict_types=1);

namespace App\Scheduling\Billing;

/**
 * How far short of a price the payer's funding falls, in whatever unit that
 * payment method counts in — tokens today, since `usd` is funded by a card
 * at checkout time and has nothing to be short of beforehand.
 *
 * Scheduling stays deliberately ignorant of what the unit IS: it asks
 * `PaymentIntentGateway::findFundingShortfall()` and renders the two numbers
 * AC-05-8 asks for ("You have 1 token, need 2 tokens"). Billing, which owns
 * the ledger, is the only side that knows they are tokens.
 */
final readonly class FundingShortfall
{
    public function __construct(
        public int $available,
        public int $required,
    ) {
    }
}
