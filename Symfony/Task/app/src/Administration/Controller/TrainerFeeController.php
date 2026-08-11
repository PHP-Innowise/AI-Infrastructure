<?php

declare(strict_types=1);

namespace App\Administration\Controller;

use App\Administration\Form\TrainerFeeType;
use App\Billing\Repository\TrainerBillingSettingsRepository;
use App\Billing\Voter\TrainerSettingsVoter;
use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use App\Platform\Service\AuditLogger;
use App\Platform\Tenancy\AdministrativeScope;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-05.10, AC-05-27/28: Super Admin sets a custom subscription price
 * and/or application fee rate for one trainer.
 *
 * Routed under `Administration`, not `Billing` — architect-architecture.md's
 * own cross-cutting services table states `AdministrativeScope`'s callers
 * as "Administration (Event Master, fee edits)" explicitly, and
 * specs/api-designer-spec.md's "Module boundary calls" repeats it.
 *
 * @see specs/api-designer-spec.md "Module boundary calls", "Administration module"
 */
#[IsGranted('ROLE_SUPER_ADMIN')]
final class TrainerFeeController extends AbstractController
{
    public function __construct(
        private readonly TrainerBillingSettingsRepository $billingSettings,
        private readonly AdministrativeScope $administrativeScope,
        private readonly AuditLogger $auditLogger,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    #[Route('/super-admin/trainers/{trainer}/fees', name: 'administration_trainer_fee_edit', methods: ['GET', 'POST'])]
    public function edit(Request $request, Trainer $trainer): Response
    {
        // AdministrativeScope opens BEFORE the voter check, not after: the
        // voter's own TRAINER_FEE_EDIT branch calls
        // `AdministrativeScope::isOpenFor($trainer)`, which can only ever
        // answer true once a scope actually IS open — matching
        // EventMasterController::withScope()'s own precedent exactly.
        $this->administrativeScope->openFor($trainer, $this->actor());

        try {
            $this->denyAccessUnlessGranted(TrainerSettingsVoter::TRAINER_FEE_EDIT, $trainer);

            $settings = $this->billingSettings->getOrCreateForTrainer($trainer);

            $form = $this->createForm(TrainerFeeType::class, [
                'monthlySubscriptionPriceMinorUnits' => $settings->getMonthlySubscriptionPriceMinorUnits(),
                'platformFeeBasisPoints' => $settings->getPlatformFeeBasisPoints(),
            ]);
            $form->handleRequest($request);

            if ($form->isSubmitted() && $form->isValid()) {
                /** @var array{monthlySubscriptionPriceMinorUnits: int, platformFeeBasisPoints: int, reason: ?string} $data */
                $data = $form->getData();

                $oldSubscriptionPrice = $settings->getMonthlySubscriptionPriceMinorUnits();
                $oldFeeRate = $settings->getPlatformFeeBasisPoints();

                // AC-05-27: "the new subscription rate applies from the
                // trainer's next billing cycle, and new transactions use
                // the new fee rate immediately" — both are stored now;
                // `FeeCalculator`/`StripeGateway::applyCurrentFee()` always
                // read the CURRENT row at charge time (never a snapshot
                // from the past), which is exactly what makes the fee half
                // "immediate" — the subscription price's own "next cycle"
                // half is Stripe Billing's own proration behavior on the
                // next invoice, not something this platform re-implements.
                $settings->updatePricing($data['monthlySubscriptionPriceMinorUnits'], $data['platformFeeBasisPoints']);
                $this->entityManager->flush();

                // AC-05-28: "who changed it, when, the old rate, the new
                // rate, and an optional reason."
                $this->auditLogger->record($this->actor(), 'trainer.fee_edit', 'TrainerBillingSettings', null, $trainer, [
                    'oldMonthlySubscriptionPriceMinorUnits' => $oldSubscriptionPrice,
                    'newMonthlySubscriptionPriceMinorUnits' => $data['monthlySubscriptionPriceMinorUnits'],
                    'oldPlatformFeeBasisPoints' => $oldFeeRate,
                    'newPlatformFeeBasisPoints' => $data['platformFeeBasisPoints'],
                    'reason' => $data['reason'],
                ]);
                $this->entityManager->flush();

                $this->addFlash('success', 'Pricing updated.');

                return $this->redirectToRoute('administration_trainer_fee_edit', ['trainer' => $trainer->getId()]);
            }

            return $this->render('administration/trainer_fee_edit.html.twig', ['form' => $form, 'trainer' => $trainer]);
        } finally {
            $this->administrativeScope->close();
        }
    }

    private function actor(): Account
    {
        /** @var Account $account */
        $account = $this->getUser();

        return $account;
    }
}
