<?php

declare(strict_types=1);

namespace App\Identity\Controller;

use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Form\AvailabilityGridType;
use App\Identity\Repository\PlayerProfileRepository;
use App\Identity\Service\AvailabilityService;
use App\Identity\Voter\AvailabilityVoter;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * AC-01-43/44: "Best Times" for the current player context — self, or the
 * child selected via `identity_portal_context_child_switch`.
 */
#[IsGranted('ROLE_PLAYER')]
final class PortalAvailabilityController extends AbstractController
{
    public function __construct(
        private readonly AvailabilityService $availabilityService,
        private readonly PlayerProfileRepository $playerProfiles,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    #[Route('/portal/availability', name: 'identity_portal_availability_edit', methods: ['GET', 'POST'])]
    public function __invoke(Request $request): Response
    {
        $this->denyAccessUnlessGranted(AvailabilityVoter::AVAILABILITY_EDIT);

        $player = $this->currentPlayerContext($request);
        $form = $this->createForm(AvailabilityGridType::class, $this->initialData($player));
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array<string, array{isAvailable: bool, startTime: ?\DateTimeImmutable, endTime: ?\DateTimeImmutable}> $data */
            $data = $form->getData();
            $slots = $this->toSlots($data);

            /** @var Trainer $trainer */
            $trainer = $this->entityManager->getReference(Trainer::class, $this->tenantContext->requireTrainerId());
            $this->availabilityService->setPlayerAvailability($trainer, $player, $slots);

            $this->addFlash('success', 'Availability saved.');

            return $this->redirectToRoute('identity_portal_availability_edit');
        }

        return $this->render('identity/availability_edit.html.twig', [
            'form' => $form,
            'player' => $player,
            'dayLabels' => AvailabilityGridType::DAY_LABELS,
        ]);
    }

    private function currentPlayerContext(Request $request): PlayerProfile
    {
        /** @var Account $account */
        $account = $this->getUser();
        $contextId = $request->getSession()->get('current_player_context_id');

        if (\is_int($contextId)) {
            $player = $this->playerProfiles->find($contextId);

            if (null !== $player) {
                return $player;
            }
        }

        return $this->playerProfiles->findOneForSelfAccount($account) ?? throw $this->createNotFoundException('No player profile for this account.');
    }

    /**
     * @return array<string, array{isAvailable: bool, startTime: ?\DateTimeImmutable, endTime: ?\DateTimeImmutable}>
     */
    private function initialData(PlayerProfile $player): array
    {
        $data = [];

        foreach ($this->availabilityService->getPlayerAvailability($player) as $window) {
            $data['day'.$window->getDayOfWeek()] = [
                'isAvailable' => $window->isAvailable(),
                'startTime' => $window->getStartTime(),
                'endTime' => $window->getEndTime(),
            ];
        }

        return $data;
    }

    /**
     * @param array<string, array{isAvailable: bool, startTime: ?\DateTimeImmutable, endTime: ?\DateTimeImmutable}> $data
     *
     * @return list<array{dayOfWeek: int, startTime: \DateTimeImmutable, endTime: \DateTimeImmutable, isAvailable: bool}>
     */
    private function toSlots(array $data): array
    {
        $slots = [];

        foreach (AvailabilityGridType::DAY_LABELS as $day => $label) {
            $row = $data['day'.$day] ?? null;

            if (null === $row || !$row['isAvailable'] || null === $row['startTime'] || null === $row['endTime']) {
                continue;
            }

            $slots[] = [
                'dayOfWeek' => $day,
                'startTime' => $row['startTime'],
                'endTime' => $row['endTime'],
                'isAvailable' => true,
            ];
        }

        return $slots;
    }
}
