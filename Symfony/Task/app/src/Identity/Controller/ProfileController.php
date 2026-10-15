<?php

declare(strict_types=1);

namespace App\Identity\Controller;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Identity\Form\EditProfileType;
use App\Identity\Service\AccountProfileService;
use App\Identity\Voter\AccountVoter;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\File\UploadedFile;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-01.11: "Profile"/"Account Settings", any role.
 */
final class ProfileController extends AbstractController
{
    public function __construct(
        private readonly AccountProfileService $accountProfileService,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    #[Route('/account/profile', name: 'identity_account_profile_edit', methods: ['GET', 'POST'])]
    #[IsGranted('IS_AUTHENTICATED_FULLY')]
    public function __invoke(Request $request): Response
    {
        /** @var Account $account */
        $account = $this->getUser();
        $this->denyAccessUnlessGranted(AccountVoter::ACCOUNT_EDIT, $account);

        $profile = $account->getProfile();
        $form = $this->createForm(EditProfileType::class, null === $profile ? [] : [
            'firstName' => $profile->getFirstName(),
            'lastName' => $profile->getLastName(),
            'phone' => $profile->getPhone(),
            'schoolOrOrganization' => $profile->getSchoolOrOrganization(),
        ], ['role' => $account->getRole()]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array<string, mixed> $data */
            $data = $form->getData();

            /** @var UploadedFile|null $photo */
            $photo = $form->get('photo')->getData();

            $trainerContext = null;

            if (AccountRole::Trainer === $account->getRole()) {
                $trainerId = $this->tenantContext->requireTrainerId();
                /** @var Trainer $trainerContext */
                $trainerContext = $this->entityManager->getReference(Trainer::class, $trainerId);
            }

            $this->accountProfileService->updateProfile(
                $account,
                (string) $data['firstName'],
                (string) $data['lastName'],
                $data['phone'] ?? null,
                $data['schoolOrOrganization'] ?? null,
                $photo,
                [
                    'schoolOrTeam' => $data['schoolOrTeam'] ?? null,
                    'jerseyNumber' => $data['jerseyNumber'] ?? null,
                ],
                [
                    'bio' => $data['bio'] ?? null,
                    'credentials' => $data['credentials'] ?? null,
                    'certifications' => $data['certifications'] ?? null,
                    'isPublicProfile' => $data['isPublicProfile'] ?? false,
                ],
                [
                    'organizationAddress' => $data['organizationAddress'] ?? null,
                    'organizationWebsite' => $data['organizationWebsite'] ?? null,
                    'organizationDescription' => $data['organizationDescription'] ?? null,
                ],
                $trainerContext,
            );

            // AC-01-49: confirmation message.
            $this->addFlash('success', 'Profile saved.');

            return $this->redirectToRoute('identity_account_profile_edit');
        }

        return $this->render('identity/profile_edit.html.twig', [
            'form' => $form,
            'account' => $account,
        ]);
    }
}
