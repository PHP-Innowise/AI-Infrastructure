<?php

declare(strict_types=1);

namespace App\Identity\Exception;

/**
 * BR-01-2: email is unique across all accounts. AC-01-7/8: a duplicate on
 * trainer creation shows a clear error rather than a raw constraint
 * violation.
 */
final class DuplicateEmailException extends \RuntimeException
{
    public static function forEmail(string $email): self
    {
        return new self(sprintf('An account with the email "%s" already exists.', $email));
    }
}
