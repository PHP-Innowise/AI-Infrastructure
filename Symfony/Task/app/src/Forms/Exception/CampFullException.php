<?php

declare(strict_types=1);

namespace App\Forms\Exception;

/**
 * AC-08-15/BR-08-7: "If the camp's capacity has been reached, the form
 * shows a 'Camp Full' message and does not allow submission."
 */
final class CampFullException extends \RuntimeException
{
    public static function forForm(int $formId): self
    {
        return new self(sprintf('Form %d has reached capacity.', $formId));
    }
}
