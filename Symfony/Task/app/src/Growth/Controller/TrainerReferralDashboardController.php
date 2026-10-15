<?php

declare(strict_types=1);

namespace App\Growth\Controller;

use App\Growth\Service\ReferralDashboardService;
use App\Platform\Entity\FeatureToggle;
use App\Platform\Entity\Trainer;
use App\Platform\Service\FeatureGate;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-06.04: the trainer referral dashboard — "Marketing" -> "Referrals"
 * ("Get the Assist").
 *
 * **Feature-gated** (`specs/api-designer-spec.md:655`: "requires the
 * Marketing feature toggle — `FeatureGate`"). Epic-06's own coder session
 * left this route deliberately ungated — "`feature_toggle` and
 * `FeatureGate` do not exist yet... building Epic-07's infrastructure here
 * would be inventing scope" (Epic-06 completion commit) — this is that
 * infrastructure's own arrival closing the gap it named. A controller-level
 * guard, not a voter clause: no `ReferralVoter` exists in the 23-voter
 * inventory (`specs/security-voter-designer-design.md` "Voter inventory")
 * and this route's own row there carries no `Voter → Subject` at all
 * ("`—`") — inventing a 24th voter class for one subjectless read screen
 * would contradict that settled, closed inventory. `FeatureGate` itself
 * stays a pure evaluator either way (its own docblock).
 *
 * @see specs/requirements-analyst-epic-06-marketing-growth-spec.md AC-06-13..16
 * @see specs/requirements-analyst-epic-07-super-admin-spec.md BR-07-1
 */
#[IsGranted('ROLE_TRAINER')]
final class TrainerReferralDashboardController extends AbstractController
{
    public function __construct(
        private readonly ReferralDashboardService $dashboard,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
        private readonly FeatureGate $featureGate,
    ) {
    }

    #[Route('/trainer/marketing/referrals', name: 'growth_trainer_referrals_dashboard', methods: ['GET'])]
    public function index(): Response
    {
        $trainer = $this->currentTrainer();

        if (!$this->featureGate->isEnabled($trainer, FeatureToggle::FEATURE_MARKETING)) {
            throw $this->createAccessDeniedException('Marketing tools are disabled for this trainer.');
        }

        $now = new \DateTimeImmutable();
        $monthStart = $now->modify('first day of this month')->setTime(0, 0);
        $monthEnd = $monthStart->modify('+1 month');

        return $this->render('growth/trainer_referrals_dashboard.html.twig', [
            'metrics' => $this->dashboard->overviewMetrics($trainer, $monthStart, $monthEnd),
            'topReferrers' => $this->dashboard->topReferrers($trainer),
            'activityLog' => $this->dashboard->activityLog($trainer),
        ]);
    }

    private function currentTrainer(): Trainer
    {
        /** @var Trainer $trainer */
        $trainer = $this->entityManager->getReference(Trainer::class, $this->tenantContext->requireTrainerId());

        return $trainer;
    }
}
