<?php

declare(strict_types=1);

namespace App\Growth\Controller;

use App\Growth\Repository\ReferralRepository;
use App\Growth\Service\ReferralLinkService;
use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Service\PlayerContextResolver;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Routing\Generator\UrlGeneratorInterface;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-06.01: "Get the Assist" — the player's own automatic referral link,
 * viewed from their portal.
 *
 * @see specs/requirements-analyst-epic-06-marketing-growth-spec.md AC-06-1..3
 * @see specs/api-designer-spec.md "Growth module" (`growth_portal_referrals`)
 */
#[IsGranted('ROLE_PLAYER')]
final class PortalReferralController extends AbstractController
{
    public function __construct(
        private readonly ReferralLinkService $referralLinkService,
        private readonly ReferralRepository $referrals,
        private readonly PlayerContextResolver $playerContext,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    /**
     * AC-06-1/2: no "generate" action — the link is provisioned on this
     * very read if it does not already exist. AC-06-3: scoped to whichever
     * trainer is the current portal context; a player training with
     * several trainers sees a different link (and different stats) after
     * switching context, exactly like every other portal screen.
     */
    #[Route('/portal/referrals', name: 'growth_portal_referrals', methods: ['GET'])]
    public function index(Request $request): Response
    {
        $trainer = $this->currentTrainer();
        $player = $this->currentPlayer($request);

        $link = $this->referralLinkService->linkFor($trainer, $player);
        $stats = $this->referrals->statsForReferrer($trainer, $player);

        return $this->render('growth/portal_referrals.html.twig', [
            'trainer' => $trainer,
            'player' => $player,
            'referralUrl' => $this->generateUrl('growth_public_referral_join', [
                'trainerSlug' => $trainer->getSlug(),
                'playerId' => $player->getId(),
            ], UrlGeneratorInterface::ABSOLUTE_URL),
            'totalReferrals' => $stats['total'],
            'totalConverted' => $stats['converted'],
        ]);
    }

    private function currentPlayer(Request $request): PlayerProfile
    {
        return $this->playerContext->resolve($request, $this->actor());
    }

    private function currentTrainer(): Trainer
    {
        /** @var Trainer $trainer */
        $trainer = $this->entityManager->getReference(Trainer::class, $this->tenantContext->requireTrainerId());

        return $trainer;
    }

    private function actor(): Account
    {
        /** @var Account $account */
        $account = $this->getUser();

        return $account;
    }
}
