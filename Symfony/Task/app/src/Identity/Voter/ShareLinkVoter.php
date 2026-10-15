<?php

declare(strict_types=1);

namespace App\Identity\Voter;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\ShareLink;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * `SHARELINK_RESOLVE` runs only once `TenantResolver` source 5 has already
 * set a genuine tenant from the code — it never decides tenancy itself, only
 * whether the now-visible `ShareLink` row is usable. "Nothing to check about
 * the viewer": this attribute is reachable anonymously, so it never inspects
 * the token beyond confirming one exists (PUBLIC_ACCESS routes still carry an
 * anonymous token).
 *
 * `SHARELINK_CREATE` is the trainer's (AC-01-73) or coach's (AC-03-52, out of
 * Epic-01's own scope but the same voter/attribute per the settled design)
 * act of generating a new link — a class-level capability check, since the
 * link is always created inside the actor's own already-active tenant.
 *
 * @see specs/security-voter-designer-design.md "Identity module" voter table
 * @see specs/requirements-analyst-epic-01-user-management-spec.md AC-01-9..14, AC-01-31, AC-01-39..42, AC-01-73
 */
/**
 * @extends Voter<string, ShareLink|null>
 */
final class ShareLinkVoter extends Voter
{
    public const SHARELINK_RESOLVE = 'SHARELINK_RESOLVE';
    public const SHARELINK_CREATE = 'SHARELINK_CREATE';

    protected function supports(string $attribute, mixed $subject): bool
    {
        return match ($attribute) {
            self::SHARELINK_RESOLVE => $subject instanceof ShareLink,
            self::SHARELINK_CREATE => true,
            default => false,
        };
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        if (self::SHARELINK_RESOLVE === $attribute) {
            \assert($subject instanceof ShareLink);

            return $subject->isUsable(new \DateTimeImmutable());
        }

        $actor = $token->getUser();

        return $actor instanceof Account
            && \in_array($actor->getRole(), [AccountRole::Trainer, AccountRole::Coach], true);
    }
}
