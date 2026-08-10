<?php

declare(strict_types=1);

namespace App\Identity\Voter;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\AccountStatus;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * `Account` is global, so every attribute here decides for itself whether
 * `ROLE_SUPER_ADMIN` gets in — there is no role hierarchy to inherit one
 * (AccountRole's own docblock).
 *
 * @see specs/security-voter-designer-design.md "Identity module" voter table
 * @see specs/requirements-analyst-epic-01-user-management-spec.md BR-01-13, AC-01-48..59, AC-01-71/72
 */
/**
 * @extends Voter<string, Account|null>
 */
final class AccountVoter extends Voter
{
    public const ACCOUNT_EDIT = 'ACCOUNT_EDIT';
    public const ACCOUNT_VIEW = 'ACCOUNT_VIEW';
    public const ACCOUNT_CREATE_TRAINER = 'ACCOUNT_CREATE_TRAINER';
    public const ACCOUNT_DEACTIVATE = 'ACCOUNT_DEACTIVATE';
    public const ACCOUNT_REACTIVATE = 'ACCOUNT_REACTIVATE';
    public const ACCOUNT_DELETE_GDPR = 'ACCOUNT_DELETE_GDPR';
    public const EMAIL_VERIFY_RESEND = 'EMAIL_VERIFY_RESEND';

    /**
     * @var list<string>
     */
    private const SUBJECTLESS = [self::ACCOUNT_CREATE_TRAINER];

    /**
     * @var list<string>
     */
    private const WITH_SUBJECT = [
        self::ACCOUNT_EDIT,
        self::ACCOUNT_VIEW,
        self::ACCOUNT_DEACTIVATE,
        self::ACCOUNT_REACTIVATE,
        self::ACCOUNT_DELETE_GDPR,
        self::EMAIL_VERIFY_RESEND,
    ];

    protected function supports(string $attribute, mixed $subject): bool
    {
        if (\in_array($attribute, self::SUBJECTLESS, true)) {
            return true;
        }

        return \in_array($attribute, self::WITH_SUBJECT, true) && $subject instanceof Account;
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        $actor = $token->getUser();

        if (!$actor instanceof Account) {
            return false;
        }

        $isSuperAdmin = AccountRole::SuperAdmin === $actor->getRole();

        return match ($attribute) {
            // BR-01-13: no self-service branch exists for any of these three.
            self::ACCOUNT_CREATE_TRAINER, self::ACCOUNT_DEACTIVATE, self::ACCOUNT_DELETE_GDPR => $isSuperAdmin,
            // AC-01-58: a Deleted account can never be reactivated —
            // anonymisation is permanent, unlike deactivation.
            self::ACCOUNT_REACTIVATE => $isSuperAdmin
                && $subject instanceof Account
                && AccountStatus::Deleted !== $subject->getStatus(),
            // AC-01-71: Super Admin can edit/view any account; otherwise
            // self-service only.
            self::ACCOUNT_EDIT, self::ACCOUNT_VIEW => $isSuperAdmin || $subject === $actor,
            // "new — self only": no Super Admin branch is named for this one.
            self::EMAIL_VERIFY_RESEND => $subject === $actor,
            default => false,
        };
    }
}
