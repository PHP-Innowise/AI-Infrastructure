<?php

declare(strict_types=1);

namespace App\Forms\Voter;

use App\Forms\Entity\Form;
use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Platform\Entity\FeatureToggle;
use App\Platform\Entity\Trainer;
use App\Platform\Service\FeatureGate;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * Trainer-console authorization for camp/evaluation forms — BR-08-19/20:
 * "Trainers can only view and edit their own forms" and "Coaches cannot
 * create or manage forms; this is a trainer-only feature" (both implicit
 * denials here, not special-cased — every attribute already requires
 * `AccountRole::Trainer`). BR-08-21's Super Admin access is "via
 * impersonation mode" (the epic's own words) — no `ROLE_SUPER_ADMIN` clause
 * anywhere, matching `PlaylistVoter`'s identical precedent: impersonation
 * makes `getUser()` return the target trainer, so ordinary ownership checks
 * already cover it.
 *
 * **Structurally same-tenant already** for every attribute with a `Form`
 * subject (RLS + the Doctrine filter make a foreign-tenant `Form` invisible
 * before this voter runs, matching `CouponVoter`'s own precedent) — the
 * `getTrainer()->getId() === current tenant` check is defense in depth, not
 * the primary boundary.
 *
 * **Feature gate**: `FORM_CREATE` for a camp, and `FORM_TOGGLE` (camps
 * only, BR-08-5), AND `FeatureGate::isEnabled($trainer, 'camps')` —
 * Epic-07's own toggle warning text says plainly "The trainer will lose the
 * ability to create camp events" (`TrainerFeatureController::DISABLE_WARNINGS`),
 * scoped to CREATION and the public submission surface
 * (`FormSubmissionVoter`), not to managing forms/registrations already
 * collected — disabling never deletes data (BR-07-3) and a trainer should
 * never lose the ability to see/export/manage what already happened.
 * Evaluations are never gated: the toggle is named "Camps," BR-08-5 makes
 * evaluations "always on," and Epic-08 itself never states the toggle
 * covers them — recorded as an interpretation of a gap both the Epic-07 and
 * Epic-08 specs flag as unresolved, in the coder's final report.
 *
 * @see specs/api-designer-spec.md "Forms module"
 * @see specs/requirements-analyst-epic-08-forms-registration-spec.md BR-08-19..21
 */
/**
 * @extends Voter<string, Form|string|null>
 */
final class FormVoter extends Voter
{
    public const FORM_CREATE = 'FORM_CREATE';
    public const FORM_EDIT = 'FORM_EDIT';
    public const FORM_TOGGLE = 'FORM_TOGGLE';
    public const FORM_DELETE = 'FORM_DELETE';
    public const FORM_VIEW_SUBMISSIONS = 'FORM_VIEW_SUBMISSIONS';
    public const FORM_EXPORT = 'FORM_EXPORT';
    public const FORM_MARK_ATTENDANCE = 'FORM_MARK_ATTENDANCE';
    public const FORM_SEND_BULK_EMAIL = 'FORM_SEND_BULK_EMAIL';

    /**
     * @var list<string>
     */
    private const OWN_TENANT_FORM_ATTRIBUTES = [
        self::FORM_EDIT,
        self::FORM_TOGGLE,
        self::FORM_DELETE,
        self::FORM_VIEW_SUBMISSIONS,
        self::FORM_EXPORT,
        self::FORM_MARK_ATTENDANCE,
        self::FORM_SEND_BULK_EMAIL,
    ];

    public function __construct(
        private readonly TenantContext $tenantContext,
        private readonly FeatureGate $featureGate,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    protected function supports(string $attribute, mixed $subject): bool
    {
        if (self::FORM_CREATE === $attribute) {
            return null === $subject || \in_array($subject, Form::TYPES, true);
        }

        return \in_array($attribute, self::OWN_TENANT_FORM_ATTRIBUTES, true) && $subject instanceof Form;
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        $actor = $token->getUser();

        if (!$actor instanceof Account || AccountRole::Trainer !== $actor->getRole()) {
            return false;
        }

        if (self::FORM_CREATE === $attribute) {
            if (Form::TYPE_CAMP === $subject) {
                return $this->campsEnabledForCurrentTenant();
            }

            return true;
        }

        \assert($subject instanceof Form);

        if (!$this->isOwnTenant($subject)) {
            return false;
        }

        // BR-08-5: a no-op/hidden control for an evaluation, not merely
        // gated by the feature toggle — see Form::enable()/disable()'s own
        // refusal.
        if (self::FORM_TOGGLE === $attribute) {
            return $subject->isCamp() && $this->campsEnabledForCurrentTenant();
        }

        return true;
    }

    private function isOwnTenant(Form $form): bool
    {
        return $form->getTrainer()->getId() === $this->tenantContext->getTrainerIdOrNull();
    }

    /**
     * No active tenant resolved has nothing to gate against, so this stays
     * neutral (`true`) and leaves the decision to the ordinary role/
     * ownership checks — matching `PlaylistVoter::lpppEnabledForCurrentTenant()`'s
     * own precedent exactly, including the lazy reference (not a query).
     */
    private function campsEnabledForCurrentTenant(): bool
    {
        $tenantId = $this->tenantContext->getTrainerIdOrNull();

        if (null === $tenantId) {
            return true;
        }

        /** @var Trainer $trainer */
        $trainer = $this->entityManager->getReference(Trainer::class, $tenantId);

        return $this->featureGate->isEnabled($trainer, FeatureToggle::FEATURE_CAMPS);
    }
}
