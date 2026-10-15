<?php

declare(strict_types=1);

namespace App\Platform\Voter;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * AC-07-29..34, BR-07-4..6: Super Admin views the platform-wide audit log.
 *
 * `—` subject (`specs/security-voter-designer-design.md` "Platform module"
 * voter table) — `AuditLogEntry` is explicitly global and "spans tenants by
 * design" (architect-architecture.md:355); this is deliberate platform-wide
 * accountability, not a per-entry ownership decision, so there is nothing
 * to load and check per row. Bare `ROLE_SUPER_ADMIN`, matching
 * `ImpersonationVoter`/`PlatformConfigurationVoter`'s own "protect global
 * subjects only" reasoning in this same module.
 *
 * @see specs/security-voter-designer-design.md "Platform module", Voter test matrix
 */
/**
 * @extends Voter<string, null>
 */
final class AuditVoter extends Voter
{
    public const AUDIT_LOG_VIEW = 'AUDIT_LOG_VIEW';

    protected function supports(string $attribute, mixed $subject): bool
    {
        return self::AUDIT_LOG_VIEW === $attribute && null === $subject;
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        $actor = $token->getUser();

        return $actor instanceof Account && AccountRole::SuperAdmin === $actor->getRole();
    }
}
