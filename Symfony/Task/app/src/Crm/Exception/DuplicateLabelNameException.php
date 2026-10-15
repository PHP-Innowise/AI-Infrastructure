<?php

declare(strict_types=1);

namespace App\Crm\Exception;

/**
 * BR-03-3: label names collide case-insensitively within one trainer.
 * Translated from the database's own unique-constraint violation
 * (`uniq_label_trainer_name_normalized`) the same way `AlreadyRegisteredException`
 * translates `uniq_rsvp_event_player` — the constraint is the final backstop
 * under concurrency, this exception is the readable surface over it.
 */
final class DuplicateLabelNameException extends \RuntimeException
{
    public static function forName(string $name): self
    {
        return new self(sprintf('A label named "%s" already exists.', $name));
    }
}
