<?php

declare(strict_types=1);

namespace App\Forms\Exception;

/**
 * BR-08-10: "the same email address cannot submit to the same form twice."
 */
final class DuplicateSubmissionException extends \RuntimeException
{
    public static function forEmail(string $email): self
    {
        return new self(sprintf('"%s" has already submitted this form.', $email));
    }
}
