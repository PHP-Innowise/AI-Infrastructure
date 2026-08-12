<?php

declare(strict_types=1);

namespace App\Scheduling\Voter;

use App\Identity\Entity\Account;
use App\Scheduling\Entity\CoachAssignment;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * BR-02-13..15, AC-02-35/36.
 *
 * Deliberately does NOT consult CoachVisibilityService: the assigned coach
 * confirming/declining their own assignment is a direct ownership check
 * against `CoachAssignment.coachMembership` — stronger and simpler than
 * reachability, since the assignment names the coach directly
 * (specs/security-voter-designer-design.md "Reach: CoachVisibilityService,
 * consumed, never restated").
 *
 * @see specs/security-voter-designer-design.md "Scheduling module"
 */
/**
 * @extends Voter<string, CoachAssignment>
 */
final class CoachAssignmentVoter extends Voter
{
    public const COACH_ASSIGNMENT_CONFIRM = 'COACH_ASSIGNMENT_CONFIRM';
    public const COACH_ASSIGNMENT_DECLINE = 'COACH_ASSIGNMENT_DECLINE';

    protected function supports(string $attribute, mixed $subject): bool
    {
        return \in_array($attribute, [self::COACH_ASSIGNMENT_CONFIRM, self::COACH_ASSIGNMENT_DECLINE], true)
            && $subject instanceof CoachAssignment;
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        $actor = $token->getUser();

        if (!$actor instanceof Account) {
            return false;
        }

        if ($subject->getCoachMembership()->getAccount() !== $actor) {
            return false;
        }

        return $subject->isPending();
    }
}
