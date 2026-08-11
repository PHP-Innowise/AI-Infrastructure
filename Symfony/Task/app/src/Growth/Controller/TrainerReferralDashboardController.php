<?php

declare(strict_types=1);

namespace App\Growth\Controller;

use App\Growth\Service\ReferralDashboardService;
use App\Platform\Entity\Trainer;
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
 * @see specs/requirements-analyst-epic-06-marketing-growth-spec.md AC-06-13..16
 */
#[IsGranted('ROLE_TRAINER')]
final class TrainerReferralDashboardController extends AbstractController
{
    public function __construct(
        private readonly ReferralDashboardService $dashboard,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    #[Route('/trainer/marketing/referrals', name: 'growth_trainer_referrals_dashboard', methods: ['GET'])]
    public function index(): Response
    {
        $trainer = $this->currentTrainer();
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
