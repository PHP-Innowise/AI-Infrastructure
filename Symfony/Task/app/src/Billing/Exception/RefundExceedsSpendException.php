<?php

declare(strict_types=1);

namespace App\Billing\Exception;

/**
 * I4: the sum of refunds referencing one spend must never exceed it.
 * BR-05-5 permits a partial refund, so this fires only once a second (or
 * over-large) refund attempt would push the running total past the
 * original spend.
 */
final class RefundExceedsSpendException extends \RuntimeException
{
    private function __construct(string $message)
    {
        parent::__construct($message);
    }

    public static function forAttempt(int $originalAmount, int $alreadyRefunded, int $attempted): self
    {
        return new self(sprintf(
            'Refund of %d would exceed the original spend of %d (%d already refunded).',
            $attempted,
            $originalAmount,
            $alreadyRefunded,
        ));
    }
}
