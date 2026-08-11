<?php

declare(strict_types=1);

namespace App\Administration\Controller;

use App\Billing\Repository\TrainerBillingSettingsRepository;
use App\Billing\Service\StripeGateway;
use App\Identity\Entity\Account;
use App\Identity\Exception\DuplicateEmailException;
use App\Identity\Form\CreateTrainerType;
use App\Identity\Service\TrainerProvisioningService;
use App\Identity\Voter\AccountVoter;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\AdministrativeScope;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\Form\FormError;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-01.01/US-07.04: Super Admin creates a trainer account. BR-01-13: no
 * self-registration exists for this role anywhere else in the codebase.
 * AC-07-16/17: the creation form additionally captures a subscription tier
 * and saving provisions the platform subscription (Billing) alongside the
 * account (Identity) — `specs/api-designer-spec.md:686`, "creates the
 * Account+Trainer (Identity) and the PlatformSubscription+Stripe
 * subscription (Billing)."
 *
 * The two calls stay separate rather than folding into
 * `TrainerProvisioningService` itself: `Identity`'s own module-map row may
 * call only `Platform` (`architect-architecture.md` "Module map") — it
 * cannot reach into `Billing` — so `Administration`, which "may call...
 * every module's services," is the only layer allowed to sequence both.
 */
#[IsGranted('ROLE_SUPER_ADMIN')]
final class TrainerCreationController extends AbstractController
{
    public function __construct(
        private readonly TrainerProvisioningService $trainerProvisioningService,
        private readonly TrainerBillingSettingsRepository $billingSettings,
        private readonly StripeGateway $stripeGateway,
        private readonly AdministrativeScope $administrativeScope,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    #[Route('/super-admin/trainers/new', name: 'administration_trainer_create', methods: ['GET', 'POST'])]
    public function __invoke(Request $request): Response
    {
        $this->denyAccessUnlessGranted(AccountVoter::ACCOUNT_CREATE_TRAINER);

        $form = $this->createForm(CreateTrainerType::class);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{businessName: string, trainerFirstName: string, trainerLastName: string, email: string, phone: ?string, subscriptionTier: int} $data */
            $data = $form->getData();

            $actor = $this->actor();

            try {
                $trainer = $this->trainerProvisioningService->createTrainer(
                    $actor,
                    $data['businessName'],
                    $data['trainerFirstName'],
                    $data['trainerLastName'],
                    $data['email'],
                    $data['phone'],
                );
            } catch (DuplicateEmailException $e) {
                // AC-01-8: a clear, specific error.
                $form->get('email')->addError(new FormError($e->getMessage()));

                return $this->render('administration/trainer_create.html.twig', ['form' => $form]);
            }

            $this->provisionPlatformSubscription($trainer, $data['subscriptionTier'], $actor);

            // AC-01-6: the new trainer appears in the Users list with status
            // Active — true by construction, Account defaults to Active.
            $this->addFlash('success', sprintf('%s has been created.', $trainer->getBusinessName()));

            return $this->redirectToRoute('administration_trainer_show', ['trainer' => $trainer->getId()]);
        }

        return $this->render('administration/trainer_create.html.twig', ['form' => $form]);
    }

    /**
     * AC-07-16: the chosen tier becomes the trainer's starting monthly
     * subscription price (`TrainerBillingSettings.monthlySubscriptionPriceMinorUnits`,
     * trainer-scoped — needs a genuine tenant context first, the same
     * `AdministrativeScope` mechanism `TrainerFeeController` already uses
     * for the identical write), then `StripeGateway::provisionPlatformSubscription()`
     * — its own docblock names this exact call site as "the prepared
     * extension point" — creates the Stripe subscription and moves
     * `PlatformSubscription` to Active.
     */
    private function provisionPlatformSubscription(Trainer $trainer, int $subscriptionTierMinorUnits, Account $actor): void
    {
        $this->administrativeScope->openFor($trainer, $actor);

        try {
            $settings = $this->billingSettings->getOrCreateForTrainer($trainer);
            $settings->updatePricing($subscriptionTierMinorUnits, $settings->getPlatformFeeBasisPoints());
            $this->entityManager->flush();

            $this->stripeGateway->provisionPlatformSubscription($trainer);
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
