<?php

declare(strict_types=1);

namespace App\Identity\Exception;

/**
 * BR-01-11: a coach can be active under only one trainer at a time.
 *
 * AC-01-41's domain refusal — not a voter decision, not a validation error on
 * a form field, but the outcome of an attempted second active
 * `CoachMembership` row for the same account. See
 * `uniq_coach_membership_active_account` and `MembershipService`.
 */
final class CoachAlreadyActiveElsewhereException extends \RuntimeException
{
    public static function forAccountId(int $accountId): self
    {
        return new self(sprintf(
            'Account #%d is already an active coach for another trainer. A coach can work for only one trainer at a time.',
            $accountId,
        ));
    }
}
