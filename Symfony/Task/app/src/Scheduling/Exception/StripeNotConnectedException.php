<?php

declare(strict_types=1);

namespace App\Scheduling\Exception;

/**
 * AC-05-3: "If a trainer has not connected Stripe, they cannot create paid
 * events... blocked with the message 'Connect Stripe first.'" Scoped to
 * `usd` pricing specifically — a token-priced event needs no Stripe call at
 * spend time (tokens may even be entirely gifted, AC-05-33, with no card
 * ever charged), so only enabling a real-money price is gated.
 */
final class StripeNotConnectedException extends \RuntimeException
{
    public function __construct()
    {
        parent::__construct('Connect Stripe first.');
    }
}
