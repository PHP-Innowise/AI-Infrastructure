<?php

declare(strict_types=1);

namespace App\Identity\Voter;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\CoachMembership;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * `COACH_INVITE` governs the trainer's act of inviting (and resending an
 * invite) — never the invited coach's eventual acceptance, which BR-01-11's
 * exclusivity rule refuses at `MembershipService`, a domain rule rather than
 * a voter decision (see that service's docblock).
 *
 * No `ROLE_SUPER_ADMIN` clause on either shape: no admin route is designed
 * for coach invitation (specs/security-voter-designer-design.md, test
 * matrix row for this voter).
 *
 * Bare role check on the `CoachMembership`-subject shape (resend) too: by
 * the time a `CoachMembership` id loads at all, RLS/the Doctrine filter have
 * already confirmed it belongs to the acting trainer's own tenant — a
 * cross-tenant id is structurally invisible (404) before this voter runs.
 *
 * @see specs/security-voter-designer-design.md "Identity module" voter table
 * @see specs/requirements-analyst-epic-01-user-management-spec.md BR-01-15, AC-01-39, AC-01-42
 */
/**
 * @extends Voter<string, CoachMembership|null>
 */
final class CoachMembershipVoter extends Voter
{
    public const COACH_INVITE = 'COACH_INVITE';

    protected function supports(string $attribute, mixed $subject): bool
    {
        return self::COACH_INVITE === $attribute && (null === $subject || $subject instanceof CoachMembership);
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        $actor = $token->getUser();

        return $actor instanceof Account && AccountRole::Trainer === $actor->getRole();
    }
}
