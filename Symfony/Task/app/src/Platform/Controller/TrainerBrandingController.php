<?php

declare(strict_types=1);

namespace App\Platform\Controller;

use App\Platform\Entity\Trainer;
use App\Platform\Form\PortalBrandingType;
use App\Platform\Service\TrainerBrandingService;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\File\UploadedFile;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-01.14: logo + primary colour.
 */
#[IsGranted('ROLE_TRAINER')]
final class TrainerBrandingController extends AbstractController
{
    public function __construct(
        private readonly TrainerBrandingService $brandingService,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
        private readonly string $uploadsDirectory,
        private readonly string $uploadsPublicPath,
    ) {
    }

    #[Route('/trainer/branding', name: 'identity_trainer_branding_edit', methods: ['GET', 'POST'])]
    public function __invoke(Request $request): Response
    {
        /** @var Trainer $trainer */
        $trainer = $this->entityManager->getReference(Trainer::class, $this->tenantContext->requireTrainerId());
        $settings = $this->brandingService->getOrCreate($trainer);

        $form = $this->createForm(PortalBrandingType::class, ['primaryColorHex' => $settings->getPrimaryColorHex()]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var UploadedFile|null $logo */
            $logo = $form->get('logo')->getData();

            if (null !== $logo) {
                $filename = bin2hex(random_bytes(16)).'.'.($logo->guessExtension() ?? 'bin');
                $logo->move($this->uploadsDirectory.'/branding', $filename);
                $this->brandingService->updateLogo($trainer, $this->uploadsPublicPath.'/branding/'.$filename);
            }

            /** @var array{primaryColorHex: ?string} $data */
            $data = $form->getData();
            $this->brandingService->updatePrimaryColor($trainer, '' === ($data['primaryColorHex'] ?? '') ? null : $data['primaryColorHex']);

            // AC-01-62: applied immediately — nothing is cached.
            $this->addFlash('success', 'Branding saved.');

            return $this->redirectToRoute('identity_trainer_branding_edit');
        }

        return $this->render('platform/trainer_branding_edit.html.twig', [
            'form' => $form,
            'settings' => $settings,
        ]);
    }
}
