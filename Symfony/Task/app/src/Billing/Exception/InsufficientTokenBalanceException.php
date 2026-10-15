<?php

declare(strict_types=1);

namespace App\Billing\Exception;

/**
 * AC-05-8: "You have 1 token, need 2 tokens" — the shortfall this exception
 * carries is exactly what that message needs.
 */
final class InsufficientTokenBalanceException extends \RuntimeException
{
    private function __construct(
        string $message,
        public readonly int $currentBalance,
        public readonly int $required,
    ) {
        parent::__construct($message);
    }

    public static function forShortfall(int $currentBalance, int $required): self
    {
        return new self(
            sprintf('Insufficient token balance: have %d, need %d.', $currentBalance, $required),
            $currentBalance,
            $required,
        );
    }
}
