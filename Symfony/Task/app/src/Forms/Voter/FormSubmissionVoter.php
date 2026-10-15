<?php

declare(strict_types=1);

namespace App\Forms\Voter;

use App\Forms\Entity\Form;
use App\Forms\Entity\FormSubmission;
use App\Forms\Repository\FormSubmissionRepository;
use App\Platform\Entity\FeatureToggle;
use App\Platform\Service\FeatureGate;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * Public, unauthenticated authorization for the code-resolved Forms routes.
 * "Resolution is not authorization" (council-sharelink-tenant-resolution.md)
 * is the whole point of this class: `TenantResolver` establishing a tenant
 * from the form's code only makes the trainer's tables reachable, never
 * grants anything by itself — this voter is what actually decides.
 *
 * `FORM_SUBMISSION_CREATE` "evaluates form state only (exists, active,
 * under capacity), never actor identity, since there is no actor"
 * (api-designer-spec.md's own words) — deliberately redundant with
 * `PublicFormController`'s own explicit "Camp Full"/"Registration Closed"
 * checks (which render a specific page instead of a generic deny), matching
 * this codebase's established "voter as backstop" pattern
 * (`ContentItemRepository::findAccessible()`'s own docblock: "defense in
 * depth, mirroring why Layer 2 is kept alongside Layer 5").
 *
 * @see specs/council-sharelink-tenant-resolution.md "Resolution is not authorization"
 * @see specs/api-designer-spec.md "Forms module" — public route table
 */
/**
 * @extends Voter<string, Form|FormSubmission>
 */
final class FormSubmissionVoter extends Voter
{
    public const FORM_SUBMISSION_CREATE = 'FORM_SUBMISSION_CREATE';
    public const FORM_SUBMISSION_CONVERT = 'FORM_SUBMISSION_CONVERT';

    public function __construct(
        private readonly FormSubmissionRepository $submissions,
        private readonly FeatureGate $featureGate,
    ) {
    }

    protected function supports(string $attribute, mixed $subject): bool
    {
        return match ($attribute) {
            self::FORM_SUBMISSION_CREATE => $subject instanceof Form,
            self::FORM_SUBMISSION_CONVERT => $subject instanceof FormSubmission,
            default => false,
        };
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        if (self::FORM_SUBMISSION_CREATE === $attribute) {
            \assert($subject instanceof Form);

            return $this->canSubmit($subject);
        }

        \assert($subject instanceof FormSubmission);

        // AC-08-26: "not already converted" — the caller (PublicFormController)
        // already resolves the submission from the code-matched form before
        // this runs, so there is nothing more to check here.
        return !$subject->isConverted();
    }

    /**
     * BR-08-3/5/7, AC-08-15: active (or an evaluation, always on) and under
     * capacity. Camps only additionally require the platform's own Camps
     * feature toggle (`FormVoter`'s own docblock explains the scoping).
     */
    private function canSubmit(Form $form): bool
    {
        if (!$form->isOpenForRegistration()) {
            return false;
        }

        if ($form->isCamp() && !$this->featureGate->isEnabled($form->getTrainer(), FeatureToggle::FEATURE_CAMPS)) {
            return false;
        }

        return !$form->isFull($this->submissions->countConfirmedForForm($form));
    }
}
