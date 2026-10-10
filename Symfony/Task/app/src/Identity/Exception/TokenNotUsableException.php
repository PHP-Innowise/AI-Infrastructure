<?php

declare(strict_types=1);

namespace App\Identity\Exception;

/**
 * A password-reset / account-setup / email-verification token that is
 * missing, already consumed, or expired.
 *
 * `specs/api-designer-spec.md` deliberately renders this as a generic error
 * page, never "token expired" — that would confirm a token was ever valid,
 * an enumeration hint the reset/verification flows are built to avoid.
 */
final class TokenNotUsableException extends \RuntimeException
{
    public static function create(): self
    {
        return new self('This link is invalid or has expired.');
    }
}
