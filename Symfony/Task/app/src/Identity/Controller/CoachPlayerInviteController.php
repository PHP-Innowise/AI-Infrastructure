<?php

declare(strict_types=1);

namespace App\Identity\Controller;

use App\Identity\Entity\Account;
use App\Identity\Form\InvitePlayerType;
use App\Identity\Repository\ShareLinkRepository;
use App\Identity\Service\ShareLinkService;
use App\Identity\Voter\ShareLinkVoter;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-03.11: a coach invites a player directly, via their own single-use
 * ShareLink — distinct from `TrainerCoachController` (a trainer inviting a
 * coach) and from the trainer's own static mass-invite link
 * (`TrainerShareLinkController`).
 *
 * @see specs/api-designer-spec.md "Identity module" — `identity_coach_player_invite`
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md AC-03-50..53
 */
#[IsGranted('ROLE_COACH')]
final class CoachPlayerInviteController extends AbstractController
{
    public function __construct(
        private readonly ShareLinkRepository $shareLinks,
        private readonly ShareLinkService $shareLinkService,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    /**
     * AC-03-52: "the coach can view their sent invitations and status."
     */
    #[Route('/coach/players/invite', name: 'identity_coach_player_invite_index', methods: ['GET'])]
    public function index(): Response
    {
        /** @var Account $account */
        $account = $this->getUser();

        return $this->render('identity/coach_player_invite_index.html.twig', [
            'form' => $this->createForm(InvitePlayerType::class),
            'invites' => $this->shareLinks->findCoachPlayerInvitesCreatedBy($account),
            'now' => new \DateTimeImmutable(),
        ]);
    }

    /**
     * AC-03-50: generates a unique `/invite/{code}` ShareLink the coach can
     * copy or send by email.
     */
    #[Route('/coach/players/invite', name: 'identity_coach_player_invite', methods: ['POST'])]
    public function invite(Request $request): Response
    {
        $this->denyAccessUnlessGranted(ShareLinkVoter::SHARELINK_CREATE);

        /** @var Account $account */
        $account = $this->getUser();
        $form = $this->createForm(InvitePlayerType::class);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{email: ?string} $data */
            $data = $form->getData();
            $trainerId = $this->tenantContext->requireTrainerId();
            /** @var Trainer $trainer */
            $trainer = $this->entityManager->getReference(Trainer::class, $trainerId);

            $link = $this->shareLinkService->issueCoachPlayerInvite($trainer, $account, '' === ($data['email'] ?? '') ? null : $data['email']);

            $this->addFlash('success', sprintf('Invite link generated: /invite/%s', $link->getCode()));

            return $this->redirectToRoute('identity_coach_player_invite_index');
        }

        return $this->render('identity/coach_player_invite_index.html.twig', [
            'form' => $form,
            'invites' => $this->shareLinks->findCoachPlayerInvitesCreatedBy($account),
            'now' => new \DateTimeImmutable(),
        ]);
    }
}
