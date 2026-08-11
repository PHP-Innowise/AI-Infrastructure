<?php

declare(strict_types=1);

namespace App\Platform\Voter;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Platform\Entity\PlatformConfiguration;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * BR-06-4, AC-06-29..31: Super Admin edits platform-wide configuration.
 *
 * `PlatformConfiguration` is global (`architect-architecture.md:357`) — no
 * tenant to leak, no `AdministrativeScope` needed. Placed in `Platform`
 * because `Platform` already owns `PlatformConfiguration` outright
 * (`specs/security-voter-designer-design.md` line 103's own reasoning);
 * `specs/api-designer-spec.md` names the voter and attribute but never a
 * module.
 *
 * `specs/security-voter-designer-design.md` also has this voter cover a
 * `FeatureToggle` subject for Epic-07's feature-toggle screen — that entity
 * does not exist in this codebase (Epic-07/Administration's feature-toggle
 * infrastructure has not been built), so this voter supports
 * `PlatformConfiguration` only. Widening `supports()` to a second subject
 * class is straightforward once `FeatureToggle` exists; adding it
 * speculatively now would be exactly the kind of invented requirement the
 * task's hard rules forbid.
 */
/**
 * @extends Voter<string, PlatformConfiguration|null>
 */
final class PlatformConfigurationVoter extends Voter
{
    public const PLATFORM_CONFIG_EDIT = 'PLATFORM_CONFIG_EDIT';

    protected function supports(string $attribute, mixed $subject): bool
    {
        return self::PLATFORM_CONFIG_EDIT === $attribute
            && (null === $subject || $subject instanceof PlatformConfiguration);
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        $actor = $token->getUser();

        if (!$actor instanceof Account) {
            return false;
        }

        // ROLE_SUPER_ADMIN-only, no self-service branch — the route itself
        // is already `ROLE_SUPER_ADMIN`-gated; this voter restates the same
        // rule at the object level rather than trusting the coarse gate
        // alone (specs/security-voter-designer-design.md "Why the coarse
        // gate is never the real check").
        return AccountRole::SuperAdmin === $actor->getRole();
    }
}
