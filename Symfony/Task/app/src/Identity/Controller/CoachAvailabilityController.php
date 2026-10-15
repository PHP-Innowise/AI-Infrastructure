<?php

declare(strict_types=1);

namespace App\Identity\Controller;

use App\Identity\Entity\Account;
use App\Identity\Entity\CoachMembership;
use App\Identity\Form\AvailabilityGridType;
use App\Identity\Form\CoachAvailabilityType;
use App\Identity\Repository\CoachMembershipRepository;
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
 * AC-01-46: "My Times" — a recurring weekly schedule, multiple slots per day.
 */
#[IsGranted('ROLE_COACH')]
final class CoachAvailabilityController extends AbstractController
{
    public function __construct(
        private readonly AvailabilityService $availabilityService,
        private readonly CoachMembershipRepository $coachMemberships,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    #[Route('/coach/availability', name: 'identity_coach_availability_edit', methods: ['GET', 'POST'])]
    public function __invoke(Request $request): Response
    {
        $this->denyAccessUnlessGranted(AvailabilityVoter::AVAILABILITY_EDIT);

        /** @var Account $account */
        $account = $this->getUser();
        $membership = $this->coachMemberships->findOneForAccountInActiveTenant($account)
            ?? throw $this->createNotFoundException('No coach membership for this account in the active tenant.');

        $form = $this->createForm(CoachAvailabilityType::class, $this->initialData($membership));
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array<string, array{isAvailable: bool, startTime: ?\DateTimeImmutable, endTime: ?\DateTimeImmutable}> $data */
            $data = $form->getData();
            $slots = $this->toSlots($data);

            /** @var Trainer $trainer */
            $trainer = $this->entityManager->getReference(Trainer::class, $this->tenantContext->requireTrainerId());
            $this->availabilityService->setCoachAvailability($trainer, $membership, $slots);

            $this->addFlash('success', 'My Times saved.');

            return $this->redirectToRoute('identity_coach_availability_edit');
        }

        return $this->render('identity/coach_availability_edit.html.twig', [
            'form' => $form,
            'dayLabels' => AvailabilityGridType::DAY_LABELS,
            'slotsPerDay' => CoachAvailabilityType::SLOTS_PER_DAY,
        ]);
    }

    /**
     * @return array<string, array{isAvailable: bool, startTime: ?\DateTimeImmutable, endTime: ?\DateTimeImmutable}>
     */
    private function initialData(CoachMembership $membership): array
    {
        $data = [];
        $slotIndexByDay = [];

        foreach ($this->availabilityService->getCoachAvailability($membership) as $window) {
            $day = $window->getDayOfWeek();
            $slotIndex = $slotIndexByDay[$day] ??= 0;

            if ($slotIndex >= CoachAvailabilityType::SLOTS_PER_DAY) {
                continue;
            }

            $data[sprintf('day%d_slot%d', $day, $slotIndex)] = [
                'isAvailable' => $window->isAvailable(),
                'startTime' => $window->getStartTime(),
                'endTime' => $window->getEndTime(),
            ];
            $slotIndexByDay[$day] = $slotIndex + 1;
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
            for ($slot = 0; $slot < CoachAvailabilityType::SLOTS_PER_DAY; ++$slot) {
                $row = $data[sprintf('day%d_slot%d', $day, $slot)] ?? null;

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
        }

        return $slots;
    }
}
