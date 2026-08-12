<?php

declare(strict_types=1);

namespace App\Identity\Controller;

use App\Identity\Entity\Account;
use App\Identity\Entity\ShareLink;
use App\Identity\Form\UniqueShareLinkType;
use App\Identity\Repository\ShareLinkOpenRepository;
use App\Identity\Repository\ShareLinkRepository;
use App\Identity\Service\IdentityMailer;
use App\Identity\Service\ShareLinkService;
use App\Identity\Voter\CoachMembershipVoter;
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
 * AC-01-73: the trainer's static player link and their outstanding coach
 * invites, with open/join counts (BR-01-27). AC-03-59/61..63 (Epic-03): the
 * SAME static link viewed with its Epic-03 CRM framing, plus (optional MVP)
 * unique per-player links and their own tracking report.
 */
#[IsGranted('ROLE_TRAINER')]
final class TrainerShareLinkController extends AbstractController
{
    public function __construct(
        private readonly ShareLinkRepository $shareLinks,
        private readonly ShareLinkOpenRepository $shareLinkOpens,
        private readonly TenantContext $tenantContext,
        private readonly ShareLinkService $shareLinkService,
        private readonly IdentityMailer $mailer,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    #[Route('/trainer/sharelinks', name: 'identity_trainer_sharelinks_index', methods: ['GET'])]
    public function index(): Response
    {
        $trainerId = $this->tenantContext->requireTrainerId();
        $staticLink = $this->shareLinks->findStaticPlayerLink($trainerId);
        $uniqueLinks = $this->shareLinks->findUniquePlayerInvitesForTrainer($trainerId);

        return $this->render('identity/trainer_sharelinks_index.html.twig', [
            'staticLink' => $staticLink,
            'staticLinkOpens' => null !== $staticLink ? $this->shareLinkOpens->countFor((int) $staticLink->getId()) : 0,
            'coachInvites' => $this->shareLinks->findCoachInvitesForTrainer($trainerId),
            // AC-03-61/63: each unique link's own open count, for the
            // tracking report (date, link type, opens, joins).
            'uniqueLinks' => array_map(
                fn (ShareLink $link): array => [
                    'link' => $link,
                    'opens' => $this->shareLinkOpens->countFor((int) $link->getId()),
                ],
                $uniqueLinks,
            ),
            'uniqueLinkForm' => $this->createForm(UniqueShareLinkType::class),
            'now' => new \DateTimeImmutable(),
        ]);
    }

    /**
     * AC-03-61 (optional MVP): a unique, one-time link for one named
     * player/parent.
     */
    #[Route('/trainer/sharelinks/unique', name: 'identity_trainer_sharelinks_unique_create', methods: ['POST'])]
    public function createUnique(Request $request): Response
    {
        $this->denyAccessUnlessGranted(ShareLinkVoter::SHARELINK_CREATE);

        $form = $this->createForm(UniqueShareLinkType::class);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{recipientName: ?string, email: ?string} $data */
            $data = $form->getData();
            /** @var Account $actor */
            $actor = $this->getUser();
            $trainerId = $this->tenantContext->requireTrainerId();
            /** @var Trainer $trainer */
            $trainer = $this->entityManager->getReference(Trainer::class, $trainerId);

            $link = $this->shareLinkService->issueUniquePlayerInvite($trainer, $actor, '' === ($data['email'] ?? '') ? null : $data['email']);

            $this->addFlash('success', sprintf('Unique invite link generated: /join/%s', $link->getCode()));
        }

        return $this->redirectToRoute('identity_trainer_sharelinks_index');
    }

    /**
     * AC-01-42: reissue a fresh 7-day link and resend the email. Keyed on
     * the ShareLink itself — see TrainerCoachController's own docblock for
     * why a CoachMembership cannot back this action for an invite nobody has
     * accepted yet.
     */
    #[Route('/trainer/sharelinks/{shareLink<\d+>}/resend', name: 'identity_trainer_sharelink_resend', methods: ['POST'])]
    public function resend(Request $request, ShareLink $shareLink): Response
    {
        $this->denyAccessUnlessGranted(CoachMembershipVoter::COACH_INVITE, null);

        if (!$this->isCsrfTokenValid('resend-sharelink'.$shareLink->getId(), $request->request->getString('_token'))) {
            throw $this->createAccessDeniedException('Invalid CSRF token.');
        }

        if (!$shareLink->isCoachLink()) {
            throw $this->createNotFoundException();
        }

        $this->shareLinkService->resend($shareLink);
        $this->entityManager->flush();
        $this->mailer->sendCoachInvite((string) $shareLink->getTargetEmail(), $shareLink->getTrainer(), $shareLink->getCode());

        $this->addFlash('success', 'Invitation resent.');

        return $this->redirectToRoute('identity_trainer_sharelinks_index');
    }
}
