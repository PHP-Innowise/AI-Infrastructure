<?php

declare(strict_types=1);

namespace App\Crm\Controller;

use App\Crm\Service\QuickViewDashboardService;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-03.08: the trainer's Quick View dashboard.
 *
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md AC-03-34..42, BR-03-16..19
 */
#[IsGranted('ROLE_TRAINER')]
final class TrainerQuickViewController extends AbstractController
{
    public function __construct(
        private readonly QuickViewDashboardService $dashboard,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    /**
     * AC-03-41: calculated from the database on every request — no caching,
     * no stored metrics entity.
     */
    #[Route('/trainer/dashboard', name: 'crm_trainer_quick_view', methods: ['GET'])]
    public function __invoke(): Response
    {
        $trainerId = $this->tenantContext->requireTrainerId();
        /** @var Trainer $trainer */
        $trainer = $this->entityManager->getReference(Trainer::class, $trainerId);

        return $this->render('crm/trainer_quick_view.html.twig', [
            'metrics' => $this->dashboard->build($trainer, new \DateTimeImmutable()),
        ]);
    }
}
