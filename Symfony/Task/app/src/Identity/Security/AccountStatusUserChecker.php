<?php

declare(strict_types=1);

namespace App\Identity\Security;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountStatus;
use Symfony\Component\Security\Core\Exception\CustomUserMessageAccountStatusException;
use Symfony\Component\Security\Core\User\UserCheckerInterface;
use Symfony\Component\Security\Core\User\UserInterface;

/**
 * AC-01-52, edge case table ("A deactivated user attempts to log in"):
 * "Login blocked with 'Account deactivated. Contact support.'"
 *
 * Symfony's `form_login` authenticator has no built-in concept of an
 * account's own enabled/disabled state — `UserInterface` alone carries none,
 * and the old `AdvancedUserInterface` this used to live on was removed. A
 * `UserCheckerInterface` is the documented replacement: the firewall calls
 * it during authentication, before this class existed nothing here checked
 * `Account::isActive()` at all, so a deactivated (or GDPR-deleted) account
 * could still sign in with its original password.
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md "Edge cases", AC-01-52
 */
final class AccountStatusUserChecker implements UserCheckerInterface
{
    public function checkPreAuth(UserInterface $user): void
    {
        if (!$user instanceof Account) {
            return;
        }

        if (AccountStatus::Deleted === $user->getStatus()) {
            // BR-01-24: anonymized in place, permanently. The generic
            // message is intentional here — a deleted account should read no
            // differently than any other login failure, since the email on
            // file no longer belongs to anyone.
            throw new CustomUserMessageAccountStatusException('Invalid credentials.');
        }

        if (!$user->isActive()) {
            throw new CustomUserMessageAccountStatusException('Account deactivated. Contact support.');
        }
    }

    public function checkPostAuth(UserInterface $user): void
    {
        // Nothing beyond the pre-auth check — status cannot change between
        // the two calls within one authentication attempt.
    }
}
