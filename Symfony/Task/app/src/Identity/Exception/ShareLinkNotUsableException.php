<?php

declare(strict_types=1);

namespace App\Identity\Exception;

/**
 * AC-01-42 and the ShareLinkVoter::SHARELINK_RESOLVE deny row of the voter
 * test matrix: an inactive, expired or exhausted ShareLink is a clear,
 * specific failure — never a generic 404.
 */
final class ShareLinkNotUsableException extends \RuntimeException
{
    public static function expired(): self
    {
        return new self('This invitation has expired.');
    }

    public static function exhausted(): self
    {
        return new self('This invitation has already been used.');
    }

    public static function revoked(): self
    {
        return new self('This invitation is no longer active.');
    }

    public static function unknown(): self
    {
        return new self('This invitation code is not recognised.');
    }
}
