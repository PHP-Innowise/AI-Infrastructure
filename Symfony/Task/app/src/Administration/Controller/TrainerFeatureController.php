<?php

declare(strict_types=1);

namespace App\Administration\Controller;

use App\Identity\Entity\Account;
use App\Platform\Entity\FeatureToggle;
use App\Platform\Entity\Trainer;
use App\Platform\Repository\FeatureToggleRepository;
use App\Platform\Service\AuditLogger;
use App\Platform\Voter\PlatformConfigurationVoter;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-07.05: Super Admin configures per-trainer feature toggles —
 * AC-07-18..21, BR-07-1..3.
 *
 * One feature toggled per request, not a combined 3-checkbox save: AC-07-20's
 * own example wording ("Disable LPPP Content for [Trainer]? Existing
 * content will be hidden from players.") names a SINGLE trainer and a
 * SINGLE feature per confirmation, matching this codebase's own established
 * confirm-then-commit shape for consequential changes
 * (`administration_user_deactivate`/`_delete`, event cancellation) rather
 * than a JS `confirm()` this suite's `KernelBrowser`-based functional tests
 * could not exercise for real.
 *
 * `FeatureToggle` is global — no `AdministrativeScope` needed
 * (`PlatformConfigurationVoter`'s own docblock).
 *
 * @see specs/api-designer-spec.md "Administration module" — administration_trainer_features_edit
 * @see specs/requirements-analyst-epic-07-super-admin-spec.md AC-07-18..21, BR-07-1..3
 */
#[IsGranted('ROLE_SUPER_ADMIN')]
final class TrainerFeatureController extends AbstractController
{
    /**
     * BR-07-1: the specific, per-feature consequence named in the
     * confirmation — one sentence per feature, matching the epic's own
     * wording as closely as the other two (unstated) features allow.
     */
    private const DISABLE_WARNINGS = [
        FeatureToggle::FEATURE_LPPP => 'Existing content will be hidden from players.',
        FeatureToggle::FEATURE_MARKETING => 'Trainers will lose access to coupons and the referral dashboard.',
        FeatureToggle::FEATURE_CAMPS => 'The trainer will lose the ability to create camp events.',
    ];

    public function __construct(
        private readonly FeatureToggleRepository $featureToggles,
        private readonly AuditLogger $auditLogger,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    /**
     * AC-07-18: view the toggle list. With `?feature=`, shows the
     * confirmation step for that one feature instead (still a GET — no
     * state changes until the POST below).
     */
    #[Route('/super-admin/trainers/{trainer}/features', name: 'administration_trainer_features_edit', methods: ['GET', 'POST'])]
    public function edit(Request $request, Trainer $trainer): Response
    {
        $this->denyAccessUnlessGranted(PlatformConfigurationVoter::PLATFORM_CONFIG_EDIT, $this->representativeToggle($trainer));

        // BR-07-2: every trainer has all three rows from creation onward;
        // this call is the defensive second line for a trainer that
        // predates that guarantee and was somehow missed by the Epic-07
        // migration's own backfill.
        $this->featureToggles->seedDefaultsForTrainer($trainer, $this->actor());
        $this->entityManager->flush();

        if ($request->isMethod('POST')) {
            return $this->handleToggle($request, $trainer);
        }

        $requestedFeature = $request->query->get('feature');
        $pendingFeature = \is_string($requestedFeature) && \in_array($requestedFeature, FeatureToggle::ALL_FEATURES, true)
            ? $requestedFeature
            : null;

        return $this->render('administration/trainer_features_edit.html.twig', [
            'trainer' => $trainer,
            'toggles' => $this->featureToggles->findAllForTrainer($trainer),
            'pendingFeature' => $pendingFeature,
            'pendingToggle' => null !== $pendingFeature ? $this->featureToggles->findOneByTrainerAndFeature($trainer, $pendingFeature) : null,
            'warnings' => self::DISABLE_WARNINGS,
        ]);
    }

    /**
     * AC-07-20: saving applies the change immediately. AC-07-21: every
     * change is logged — who, which feature, for which trainer, when.
     */
    private function handleToggle(Request $request, Trainer $trainer): Response
    {
        if (!$this->isCsrfTokenValid('trainer-feature-toggle'.$trainer->getId(), $request->request->getString('_token'))) {
            throw $this->createAccessDeniedException('Invalid CSRF token.');
        }

        $feature = $request->request->getString('feature');

        if (!\in_array($feature, FeatureToggle::ALL_FEATURES, true)) {
            throw $this->createNotFoundException('Unknown feature.');
        }

        $toggle = $this->featureToggles->findOneByTrainerAndFeature($trainer, $feature)
            ?? throw $this->createNotFoundException('Feature toggle not found.');

        $wasEnabled = $toggle->isEnabled();
        $actor = $this->actor();

        // BR-07-3: hides/restores, never deletes — enable()/disable() only
        // ever flip this one flag; nothing under LPPP/Marketing/Camps is
        // touched here.
        $wasEnabled ? $toggle->disable($actor) : $toggle->enable($actor);
        $this->entityManager->flush();

        $this->auditLogger->record($actor, 'feature_toggled', 'FeatureToggle', $toggle->getId(), $trainer, [
            'feature' => $feature,
            'oldEnabled' => $wasEnabled,
            'newEnabled' => $toggle->isEnabled(),
        ]);
        $this->entityManager->flush();

        $this->addFlash('success', sprintf(
            '%s %s for %s.',
            $toggle->label(),
            $toggle->isEnabled() ? 'enabled' : 'disabled',
            $trainer->getBusinessName(),
        ));

        return $this->redirectToRoute('administration_trainer_features_edit', ['trainer' => $trainer->getId()]);
    }

    /**
     * A subject for the voter check — any one of the trainer's three rows
     * is representative (PlatformConfigurationVoter's own logic never
     * inspects the subject beyond its type; see that class's docblock).
     * Falls back to `null` before the seed above has ever run for this
     * trainer (a brand-new database, or a `Trainer` built directly in a
     * test) — the voter supports a `null` subject too.
     */
    private function representativeToggle(Trainer $trainer): ?FeatureToggle
    {
        return $this->featureToggles->findOneByTrainerAndFeature($trainer, FeatureToggle::FEATURE_LPPP);
    }

    private function actor(): Account
    {
        /** @var Account $account */
        $account = $this->getUser();

        return $account;
    }
}
