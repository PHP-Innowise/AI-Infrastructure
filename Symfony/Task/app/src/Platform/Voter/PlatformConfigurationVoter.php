<?php

declare(strict_types=1);

namespace App\Platform\Voter;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Platform\Entity\FeatureToggle;
use App\Platform\Entity\PlatformConfiguration;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * BR-06-4, AC-06-29..31: Super Admin edits platform-wide configuration.
 * BR-07-1..3, AC-07-18..21: Super Admin edits a trainer's feature toggles.
 *
 * `PlatformConfiguration` is global (`architect-architecture.md:357`) — no
 * tenant to leak, no `AdministrativeScope` needed. Placed in `Platform`
 * because `Platform` already owns `PlatformConfiguration` outright
 * (`specs/security-voter-designer-design.md` line 103's own reasoning);
 * `specs/api-designer-spec.md` names the voter and attribute but never a
 * module.
 *
 * `FeatureToggle` is also global (see its own docblock for the module
 * placement this required), so the same reasoning applies unchanged:
 * `voteOnAttribute()` needs no subject-specific branch for it, only the
 * bare `ROLE_SUPER_ADMIN` check every attribute here already performs.
 */
/**
 * @extends Voter<string, PlatformConfiguration|FeatureToggle|null>
 */
final class PlatformConfigurationVoter extends Voter
{
    public const PLATFORM_CONFIG_EDIT = 'PLATFORM_CONFIG_EDIT';

    protected function supports(string $attribute, mixed $subject): bool
    {
        return self::PLATFORM_CONFIG_EDIT === $attribute
            && (null === $subject || $subject instanceof PlatformConfiguration || $subject instanceof FeatureToggle);
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
