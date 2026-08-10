<?php

declare(strict_types=1);

namespace App\Crm\Voter;

use App\Crm\Entity\Label;
use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * BR-03-3/4/5, AC-03-11..13.
 *
 * Trainer-only, with **no** Super Admin clause at all — the one deliberate,
 * sourced prohibition in the Crm voter inventory, not merely an unstated
 * mechanism: AC-03-58 states Super Admin "cannot edit trainer-specific
 * labels, respecting each trainer's own customization." Every other
 * Crm-module Super-Admin gap in this design is "capability granted,
 * mechanism unstated"; this one is "capability explicitly denied" — kept
 * distinct rather than conflated with the others.
 *
 * @see specs/security-voter-designer-design.md "Crm module"
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md AC-03-58
 */
/**
 * @extends Voter<string, Label|null>
 */
final class LabelVoter extends Voter
{
    public const LABEL_MANAGE = 'LABEL_MANAGE';

    protected function supports(string $attribute, mixed $subject): bool
    {
        return self::LABEL_MANAGE === $attribute && (null === $subject || $subject instanceof Label);
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        $actor = $token->getUser();

        // Structurally same-tenant already when $subject is a Label: RLS +
        // the Doctrine filter make a foreign-tenant Label invisible before
        // this voter ever runs. No Super Admin branch, by design — see this
        // class's own docblock.
        return $actor instanceof Account && AccountRole::Trainer === $actor->getRole();
    }
}
