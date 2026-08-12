<?php

declare(strict_types=1);

namespace App\Platform\Voter;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use Symfony\Component\Security\Core\Authentication\Token\SwitchUserToken;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * `IMPERSONATION_START`'s `Account` subject is the TARGET, never the actor.
 * It checks that the actor holds Super Admin, and adds the rule a role check
 * cannot express: BR-01-21/AC-01-37, a Super Admin may not target another
 * Super Admin.
 *
 * **This voter is the whole rule, on every path.** `security.yaml` names
 * `IMPERSONATION_START` as the `switch_user` attribute, so Symfony's own
 * firewall listener asks this same question with the same target before it
 * swaps any token — not only `administration_impersonation_start` does. That
 * matters because the query parameter reaches every URL in the application,
 * and for a while it was the way around this class.
 *
 * `IMPERSONATION_END` has no subject: it is reachable from any prefix, and
 * grants whenever the current token is a `SwitchUserToken` — the identical
 * structural check Symfony's own `IS_IMPERSONATOR` attribute
 * (`AuthenticatedVoter`) uses. There is no "previous admin" role to check;
 * impersonation is detected by token class, not by a granted role.
 *
 * @see specs/security-voter-designer-design.md "Platform module" voter table
 * @see specs/requirements-analyst-epic-01-user-management-spec.md BR-01-21/22, AC-01-33..38
 */
/**
 * @extends Voter<string, Account|null>
 */
final class ImpersonationVoter extends Voter
{
    public const IMPERSONATION_START = 'IMPERSONATION_START';
    public const IMPERSONATION_END = 'IMPERSONATION_END';

    protected function supports(string $attribute, mixed $subject): bool
    {
        return match ($attribute) {
            self::IMPERSONATION_START => $subject instanceof Account,
            self::IMPERSONATION_END => true,
            default => false,
        };
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        $actor = $token->getUser();

        if (!$actor instanceof Account) {
            return false;
        }

        if (self::IMPERSONATION_END === $attribute) {
            // Symfony's own IS_IMPERSONATOR attribute (AuthenticatedVoter)
            // uses the identical check — there is no "previous admin" role,
            // impersonation is detected structurally, by token class.
            return $token instanceof SwitchUserToken;
        }

        \assert($subject instanceof Account);

        if (AccountRole::SuperAdmin !== $actor->getRole()) {
            return false;
        }

        // Compared by identifier, not by object identity: the firewall's own
        // switch_user path loads the target through the user provider, which
        // need not hand back the very instance sitting in the actor's token,
        // so `$subject === $actor` would quietly stop recognising an admin
        // targeting themselves.
        if ($subject->getId() === $actor->getId()) {
            return false;
        }

        // BR-01-21/AC-01-37: the one capability this voter exists to deny.
        return AccountRole::SuperAdmin !== $subject->getRole();
    }
}
