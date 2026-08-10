<?php

declare(strict_types=1);

namespace App\Identity\Voter;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * `Subject: —`: a player/parent or coach always edits their OWN availability
 * (or, for a parent, the child selected via the context switch) — the
 * controller never accepts a target player/coach id on this route, so there
 * is no object to authorize against. This is the pattern
 * `specs/api-designer-spec.md` names for a `—` subject row: "the query
 * itself is scoped to the current actor."
 *
 * `AvailabilityWindow` is trainer-scoped (architecture Decisions, "Player
 * availability" — isolation over convenience), so a player's Best Times
 * under one trainer are simply invisible while a different trainer's tenant
 * is active — that boundary is enforced by RLS/the Doctrine filter, not by
 * this voter.
 *
 * @see specs/security-voter-designer-design.md "Identity module" voter table
 * @see specs/requirements-analyst-epic-01-user-management-spec.md BR-01-25/26, AC-01-43..47
 */
/**
 * @extends Voter<string, null>
 */
final class AvailabilityVoter extends Voter
{
    public const AVAILABILITY_EDIT = 'AVAILABILITY_EDIT';

    protected function supports(string $attribute, mixed $subject): bool
    {
        return self::AVAILABILITY_EDIT === $attribute;
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        $actor = $token->getUser();

        return $actor instanceof Account
            && \in_array($actor->getRole(), [AccountRole::Player, AccountRole::Coach], true);
    }
}
