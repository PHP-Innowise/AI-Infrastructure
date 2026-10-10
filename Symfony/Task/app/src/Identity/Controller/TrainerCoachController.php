<?php

declare(strict_types=1);

namespace App\Identity\Controller;

use App\Identity\Entity\Account;
use App\Identity\Form\InviteCoachType;
use App\Identity\Repository\CoachMembershipRepository;
use App\Identity\Service\IdentityMailer;
use App\Identity\Service\ShareLinkService;
use App\Identity\Voter\CoachMembershipVoter;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-01.08, trainer side: inviting coaches and tracking invitations.
 *
 * Note: AC-01-42's "resend" is keyed on the ShareLink itself
 * (TrainerShareLinkController::resendCoachInvite()), not on a CoachMembership
 * — `coach_membership.account_id` is NOT NULL per the settled schema, so no
 * CoachMembership row can exist for an invite nobody has accepted yet. The
 * status a trainer sees for an unaccepted invite (Pending/Accepted/Expired)
 * is read from the ShareLink's own state, on the ShareLinks page.
 */
#[IsGranted('ROLE_TRAINER')]
final class TrainerCoachController extends AbstractController
{
    public function __construct(
        private readonly CoachMembershipRepository $coachMemberships,
        private readonly ShareLinkService $shareLinkService,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
        private readonly IdentityMailer $mailer,
    ) {
    }

    #[Route('/trainer/coaches', name: 'identity_trainer_coaches_index', methods: ['GET'])]
    public function index(): Response
    {
        return $this->render('identity/trainer_coaches_index.html.twig', [
            'coaches' => $this->coachMemberships->findAllForActiveTenant(),
            'now' => new \DateTimeImmutable(),
        ]);
    }

    /**
     * AC-01-39: name/message optional, only email required. A unique,
     * one-time-use, 7-day-expiry ShareLink is generated and emailed.
     */
    #[Route('/trainer/coaches/invite', name: 'identity_trainer_coach_invite', methods: ['GET', 'POST'])]
    public function invite(Request $request): Response
    {
        $this->denyAccessUnlessGranted(CoachMembershipVoter::COACH_INVITE);

        $form = $this->createForm(InviteCoachType::class);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{email: string} $data */
            $data = $form->getData();

            /** @var Account $account */
            $account = $this->getUser();
            $trainerId = $this->tenantContext->requireTrainerId();
            /** @var Trainer $trainerEntity */
            $trainerEntity = $this->entityManager->getReference(Trainer::class, $trainerId);

            $link = $this->shareLinkService->issueCoachInvite($trainerEntity, $account, $data['email']);
            $this->mailer->sendCoachInvite($data['email'], $trainerEntity, $link->getCode());

            $this->addFlash('success', 'Invitation sent.');

            return $this->redirectToRoute('identity_trainer_coaches_index');
        }

        return $this->render('identity/trainer_coach_invite.html.twig', ['form' => $form]);
    }
}
