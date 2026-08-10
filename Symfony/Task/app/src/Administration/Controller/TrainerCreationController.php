<?php

declare(strict_types=1);

namespace App\Administration\Controller;

use App\Identity\Entity\Account;
use App\Identity\Exception\DuplicateEmailException;
use App\Identity\Form\CreateTrainerType;
use App\Identity\Service\TrainerProvisioningService;
use App\Identity\Voter\AccountVoter;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\Form\FormError;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-01.01: Super Admin creates a trainer account. BR-01-13: no
 * self-registration exists for this role anywhere else in the codebase.
 */
#[IsGranted('ROLE_SUPER_ADMIN')]
final class TrainerCreationController extends AbstractController
{
    public function __construct(
        private readonly TrainerProvisioningService $trainerProvisioningService,
    ) {
    }

    #[Route('/super-admin/trainers/new', name: 'administration_trainer_create', methods: ['GET', 'POST'])]
    public function __invoke(Request $request): Response
    {
        $this->denyAccessUnlessGranted(AccountVoter::ACCOUNT_CREATE_TRAINER);

        $form = $this->createForm(CreateTrainerType::class);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{businessName: string, trainerFirstName: string, trainerLastName: string, email: string, phone: ?string} $data */
            $data = $form->getData();

            /** @var Account $actor */
            $actor = $this->getUser();

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

            // AC-01-6: the new trainer appears in the Users list with status
            // Active — true by construction, Account defaults to Active.
            $this->addFlash('success', sprintf('%s has been created.', $trainer->getBusinessName()));

            return $this->redirectToRoute('administration_user_show', ['account' => $trainer->getOwnerAccount()->getId()]);
        }

        return $this->render('administration/trainer_create.html.twig', ['form' => $form]);
    }
}
