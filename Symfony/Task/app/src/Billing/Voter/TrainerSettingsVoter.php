<?php

declare(strict_types=1);

namespace App\Billing\Voter;

use App\Billing\Entity\TrainerBillingSettings;
use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\AdministrativeScope;
use App\Platform\Tenancy\TenantContext;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * AC-05-1..3/25..28: a trainer's own Stripe Connect/pricing settings, and
 * Super Admin's per-trainer fee override.
 *
 * `TRAINER_SETTINGS_VIEW`/`TRAINER_SETTINGS_EDIT` take a
 * `TrainerBillingSettings` subject (the trainer's own screen);
 * `TRAINER_FEE_EDIT` takes a bare `Trainer` (Super Admin's screen, per
 * specs/api-designer-spec.md's route table: "`TrainerSettingsVoter::TRAINER_FEE_EDIT
 * → Trainer` (+ opens `AdministrativeScope`)") — two subject shapes for one
 * voter class because both govern the same underlying billing-settings
 * concept, matching `ChildApprovalVoter`'s own "two unrelated attributes
 * sharing one class" precedent.
 *
 * @see specs/api-designer-spec.md "Billing module", "Administration module"
 */
/**
 * @extends Voter<string, TrainerBillingSettings|Trainer>
 */
final class TrainerSettingsVoter extends Voter
{
    public const TRAINER_SETTINGS_VIEW = 'TRAINER_SETTINGS_VIEW';
    public const TRAINER_SETTINGS_EDIT = 'TRAINER_SETTINGS_EDIT';
    public const TRAINER_FEE_EDIT = 'TRAINER_FEE_EDIT';

    public function __construct(
        private readonly TenantContext $tenantContext,
        private readonly AdministrativeScope $administrativeScope,
    ) {
    }

    protected function supports(string $attribute, mixed $subject): bool
    {
        return match ($attribute) {
            self::TRAINER_SETTINGS_VIEW, self::TRAINER_SETTINGS_EDIT => $subject instanceof TrainerBillingSettings,
            self::TRAINER_FEE_EDIT => $subject instanceof Trainer,
            default => false,
        };
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        $actor = $token->getUser();

        if (!$actor instanceof Account) {
            return false;
        }

        if (self::TRAINER_FEE_EDIT === $attribute) {
            \assert($subject instanceof Trainer);

            // AC-05-27/28: Super Admin only, via an audited administrative
            // tenant scope — matching EventVoter's own
            // Super-Admin-via-AdministrativeScope precedent.
            return AccountRole::SuperAdmin === $actor->getRole() && $this->administrativeScope->isOpenFor($subject);
        }

        \assert($subject instanceof TrainerBillingSettings);

        // AC-05-1..3: own-tenant trainer only — structurally same-tenant
        // already (RLS + the Doctrine filter make a foreign-tenant row
        // invisible before this voter runs), matching EventVoter's own
        // "trainer role -> true" shortcut for own-tenant attributes.
        return AccountRole::Trainer === $actor->getRole()
            && $subject->getTrainer()->getId() === $this->tenantContext->getTrainerIdOrNull();
    }
}
