<?php

declare(strict_types=1);

namespace App\Forms\Exception;

/**
 * AC-08-31: "a form with paid registrations cannot be deleted until those
 * payments are refunded via Stripe." Widened here to ANY submission, paid or
 * free — `form_submission.form_id` is `FK -> form(id) RESTRICT`
 * (database-designer-schema.md "`form_submission`"), so a hard delete is
 * structurally impossible while even one free registration exists, not only
 * a paid one. Recorded as a spec-vs-schema conflict in the coder's final
 * report rather than silently narrowed to "paid only" and left to fail as a
 * raw `SQLSTATE[23503]` the first time a free-only form is deleted.
 */
final class FormHasSubmissionsException extends \RuntimeException
{
    public static function withPaidCount(int $totalCount, int $paidUnrefundedCount): self
    {
        if ($paidUnrefundedCount > 0) {
            return new self(sprintf(
                'This form has %d paid registration(s) that must be refunded via Stripe before it can be deleted.',
                $paidUnrefundedCount,
            ));
        }

        return new self(sprintf(
            'This form has %d registration(s) and cannot be deleted while any remain. Disable it instead.',
            $totalCount,
        ));
    }
}
