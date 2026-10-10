<?php

declare(strict_types=1);

namespace App\Identity\Voter;

use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Repository\ParentChildLinkRepository;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * AC-01-16..24, AC-01-27/28. Denies a child's own login on **every**
 * attribute here (AC-01-30: "cannot... change trainer associations") — the
 * distinguishing test is not which `PlayerProfile` is the beneficiary (a
 * parent and their child can target the same profile) but which `Account`
 * `getUser()` returns, exactly as specs/security-voter-designer-design.md
 * "Distinguishing a child's own request from a parent acting for the child"
 * describes.
 *
 * @see specs/security-voter-designer-design.md "Identity module" voter table
 * @see specs/requirements-analyst-epic-01-user-management-spec.md AC-01-16..24, AC-01-27/28, AC-01-30
 */
/**
 * @extends Voter<string, PlayerProfile|null>
 */
final class ChildProfileVoter extends Voter
{
    public const CHILD_PROFILE_CREATE = 'CHILD_PROFILE_CREATE';
    public const CHILD_PROFILE_EDIT = 'CHILD_PROFILE_EDIT';
    public const CHILD_TRAINER_ADD = 'CHILD_TRAINER_ADD';
    public const CHILD_TRAINER_REMOVE = 'CHILD_TRAINER_REMOVE';
    public const CHILD_TOKEN_APPROVAL_EDIT = 'CHILD_TOKEN_APPROVAL_EDIT';
    public const CHILD_PROFILE_VIEW = 'CHILD_PROFILE_VIEW';

    /**
     * @var list<string>
     */
    private const WITH_SUBJECT = [
        self::CHILD_PROFILE_EDIT,
        self::CHILD_TRAINER_ADD,
        self::CHILD_TRAINER_REMOVE,
        self::CHILD_TOKEN_APPROVAL_EDIT,
        self::CHILD_PROFILE_VIEW,
    ];

    public function __construct(
        private readonly ParentChildLinkRepository $parentChildLinks,
    ) {
    }

    protected function supports(string $attribute, mixed $subject): bool
    {
        if (self::CHILD_PROFILE_CREATE === $attribute) {
            return true;
        }

        return \in_array($attribute, self::WITH_SUBJECT, true) && $subject instanceof PlayerProfile;
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        $actor = $token->getUser();

        if (!$actor instanceof Account) {
            return false;
        }

        // AC-01-30: a child's own login is denied outright, on every
        // attribute this voter governs, with no exceptions.
        if ($this->isChildLogin($actor)) {
            return false;
        }

        if (self::CHILD_PROFILE_CREATE === $attribute) {
            return true;
        }

        \assert($subject instanceof PlayerProfile);

        $link = $this->parentChildLinks->findByChildPlayer($subject);

        return null !== $link && $link->getParentAccount() === $actor;
    }

    private function isChildLogin(Account $actor): bool
    {
        return null !== $this->parentChildLinks->findByChildAccount($actor);
    }
}
