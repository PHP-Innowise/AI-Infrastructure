<?php

declare(strict_types=1);

namespace App\Crm\Exception;

/**
 * BR-03-8: "at most one active instance of a given flag per player" —
 * reapplying the same flag while an earlier instance is still active is
 * refused; reapplying after resolution is fine (a fresh row). Translated
 * from the database's own partial-unique-constraint violation
 * (`uniq_player_flag_active_type`), the final backstop under concurrency.
 */
final class DuplicateActiveFlagException extends \RuntimeException
{
    public static function forType(string $flagType): self
    {
        return new self(sprintf('This player already has an active "%s" flag.', $flagType));
    }
}
