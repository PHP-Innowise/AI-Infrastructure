<?php

declare(strict_types=1);

namespace App\Administration\Controller;

use App\Billing\Repository\PlatformSubscriptionRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Repository\TrainerRepository;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * "In Scope (MVP)" § "Trainer Management" (no dedicated user story) —
 * AC-07-38: view every trainer, view/edit trainer details, view each
 * trainer's subscription status. AC-07-39 (deactivate/reactivate) reuses
 * `administration_user_deactivate`/`_reactivate` outright — a Trainer's
 * owner Account is a plain Account, no separate route
 * (`specs/api-designer-spec.md:687`, "also the trainer-deactivation
 * action... no separate route").
 *
 * `Trainer` and `PlatformSubscription` are both global
 * (architect-architecture.md "Entity population — Global") — reachable
 * directly through their own repositories, no `CrossTenantReadService`
 * crossing read needed, unlike Event Master or the Users tool's
 * trainer-scoped-adjacent reads.
 *
 * @see specs/api-designer-spec.md "Administration module" — administration_trainers_index, administration_trainer_show
 * @see specs/requirements-analyst-epic-07-super-admin-spec.md AC-07-38, AC-07-39
 */
#[IsGranted('ROLE_SUPER_ADMIN')]
final class TrainersController extends AbstractController
{
    public function __construct(
        private readonly TrainerRepository $trainers,
        private readonly PlatformSubscriptionRepository $platformSubscriptions,
    ) {
    }

    #[Route('/super-admin/trainers', name: 'administration_trainers_index', methods: ['GET'])]
    public function index(Request $request): Response
    {
        $q = $request->query->get('q');
        $trainers = $this->trainers->search(\is_string($q) && '' !== $q ? $q : null);

        $subscriptionStatuses = [];
        foreach ($trainers as $trainer) {
            $subscriptionStatuses[(int) $trainer->getId()] = $this->platformSubscriptions->findForTrainer($trainer)?->getStatus() ?? 'pending';
        }

        return $this->render('administration/trainers_index.html.twig', [
            'trainers' => $trainers,
            'subscriptionStatuses' => $subscriptionStatuses,
        ]);
    }

    /**
     * AC-07-38: "view each trainer's subscription status via Stripe" — the
     * STATUS column is read directly off the local, global
     * `platform_subscription` row (database-designer-schema.md
     * "`platform_subscription`": "BR-07-7's... dashboard count reads this
     * column directly, no crossing read needed" — the same reasoning
     * applies to displaying it here), never a live Stripe figure; only
     * money figures are forbidden from being duplicated locally
     * (AC-07-7), and a status string is not one. The page separately links
     * to `administration_trainer_stripe_dashboard_link` for the actual
     * Stripe-side detail.
     */
    #[Route('/super-admin/trainers/{trainer<\d+>}', name: 'administration_trainer_show', methods: ['GET'])]
    public function show(Trainer $trainer): Response
    {
        return $this->render('administration/trainer_show.html.twig', [
            'trainer' => $trainer,
            'subscription' => $this->platformSubscriptions->findForTrainer($trainer),
        ]);
    }
}
