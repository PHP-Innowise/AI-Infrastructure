<?php

declare(strict_types=1);

namespace App\Crm\Exception;

/**
 * BR-03-12: "after 24 hours it becomes read-only (editing requires
 * contacting Super Admin)." Deliberately EDIT-only — BR-03-10/AC-03-21 place
 * no time limit on a trainer deleting their own note, only on editing one,
 * so `PlayerNoteService::delete()` never throws this. See that service's
 * own docblock for why edit and delete cannot share one time rule, and
 * therefore why this check lives in the service rather than in
 * `PlayerVoter` alongside the coach-authorship check.
 */
final class NoteEditWindowExpiredException extends \RuntimeException
{
    public static function create(): self
    {
        return new self('This note can no longer be edited — it is more than 24 hours old. Contact Super Admin.');
    }
}
