<?php

declare(strict_types=1);

namespace App\Scheduling\Exception;

use App\Scheduling\Billing\FundingShortfall;

/**
 * AC-05-8: "You have 1 token, need 2 tokens." The message this carries IS
 * the one shown to the player, built from the two numbers
 * `PaymentIntentGateway::findFundingShortfall()` reported.
 *
 * Thrown by `RsvpService::rsvp()` BEFORE it opens its transaction, so
 * nothing is created for a registration that cannot be paid for and the
 * caller's EntityManager is still open when the controller renders the
 * message — the whole point of the pre-check (see the gateway method's own
 * docblock, and `TokenLedgerService::spend()` for the closed-EntityManager
 * failure this replaces).
 */
final class InsufficientFundsException extends \RuntimeException
{
    private function __construct(
        string $message,
        public readonly int $available,
        public readonly int $required,
    ) {
        parent::__construct($message);
    }

    public static function forShortfall(FundingShortfall $shortfall): self
    {
        return new self(
            sprintf(
                'You have %d token%s, need %d token%s.',
                $shortfall->available,
                1 === $shortfall->available ? '' : 's',
                $shortfall->required,
                1 === $shortfall->required ? '' : 's',
            ),
            $shortfall->available,
            $shortfall->required,
        );
    }
}
